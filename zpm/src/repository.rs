use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::config::{Config, RepoConfig};
use crate::dependency::Catalog;
use crate::error::{Result, ZpmError};
use crate::manifest::Manifest;
use crate::package::validate_id;
use crate::signature::TrustStore;

const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
pub const API_PREFIX: &str = "/api/v1";

// ---- wire types: the contract shared with the package store ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryInfo {
    pub name: String,
    pub api_version: u32,
    pub package_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub latest_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub items: Vec<PackageSummary>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionsResponse {
    pub versions: Vec<String>,
}

/// What the store returns for one version. The signature covers the exact
/// bytes of `manifest_toml`, and the manifest pins the archive checksum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRecord {
    pub manifest_toml: String,
    pub signature: String,
    pub key_id: String,
}

// ---- one repository ----

pub struct Repository {
    pub name: String,
    base: String,
    agent: ureq::Agent,
}

fn is_loopback(authority: &str) -> bool {
    ["localhost", "127.0.0.1", "[::1]"].iter().any(|h| {
        authority == *h || authority.strip_prefix(h).map_or(false, |rest| rest.starts_with(':'))
    })
}

/// Only https is accepted, except for loopback hosts used in development.
pub fn check_url(url: &str) -> Result<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let (scheme, rest) = trimmed
        .split_once("://")
        .ok_or_else(|| ZpmError::Config(format!("repository url '{url}' needs a scheme")))?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(ZpmError::Config(format!("repository url '{url}' is malformed (credentials are not allowed)")));
    }
    match scheme {
        "https" => Ok(trimmed.to_string()),
        "http" if is_loopback(authority) => Ok(trimmed.to_string()),
        _ => Err(ZpmError::InsecureRepository(url.to_string())),
    }
}

impl Repository {
    pub fn new(cfg: &RepoConfig) -> Result<Self> {
        let base = check_url(&cfg.url)?;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .redirects(0)
            .user_agent(concat!("zpm/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(Self { name: cfg.name.clone(), base, agent })
    }

    /// `Ok(None)` means HTTP 404.
    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<ureq::Response>> {
        let mut req = self.agent.get(&format!("{}{}{}", self.base, API_PREFIX, path));
        for (k, v) in query {
            req = req.query(k, v);
        }
        match req.call() {
            Ok(resp) if resp.status() == 200 => Ok(Some(resp)),
            Ok(resp) => Err(ZpmError::Repository(format!("'{}' answered with HTTP {}", self.name, resp.status()))),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(ureq::Error::Status(code, _)) => {
                Err(ZpmError::Repository(format!("'{}' answered with HTTP {code}", self.name)))
            }
            Err(ureq::Error::Transport(t)) => Err(ZpmError::Network(format!("'{}': {t}", self.name))),
        }
    }

    fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<T>> {
        match self.get(path, query)? {
            None => Ok(None),
            Some(resp) => {
                let reader = resp.into_reader().take(MAX_JSON_BYTES);
                serde_json::from_reader(reader)
                    .map(Some)
                    .map_err(|e| ZpmError::Repository(format!("'{}' sent an invalid response: {e}", self.name)))
            }
        }
    }

    pub fn metadata(&self) -> Result<RepositoryInfo> {
        self.get_json("/repository", &[])?
            .ok_or_else(|| ZpmError::Repository(format!("'{}' has no /repository endpoint", self.name)))
    }

    pub fn search(
        &self,
        query: &str,
        category: Option<&str>,
        arch: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<SearchResponse> {
        let page = page.to_string();
        let per_page = per_page.to_string();
        let mut q: Vec<(&str, &str)> = vec![("q", query), ("page", &page), ("per_page", &per_page)];
        if let Some(c) = category {
            q.push(("category", c));
        }
        if let Some(a) = arch {
            q.push(("architecture", a));
        }
        self.get_json("/packages", &q)?
            .ok_or_else(|| ZpmError::Repository(format!("'{}' has no /packages endpoint", self.name)))
    }

    /// Sorted ascending; `None` if the repository does not have the package.
    pub fn versions(&self, id: &str) -> Result<Option<Vec<Version>>> {
        validate_id(id)?;
        let Some(resp) = self.get_json::<VersionsResponse>(&format!("/packages/{id}/versions"), &[])? else {
            return Ok(None);
        };
        let mut out = Vec::new();
        for v in resp.versions {
            let parsed = Version::parse(&v)
                .map_err(|e| ZpmError::Repository(format!("'{}' lists invalid version '{v}' for {id}: {e}", self.name)))?;
            out.push(parsed);
        }
        out.sort();
        out.dedup();
        Ok(Some(out))
    }

    pub fn version_record(&self, id: &str, version: &Version) -> Result<VersionRecord> {
        validate_id(id)?;
        self.get_json(&format!("/packages/{id}/versions/{version}"), &[])?
            .ok_or_else(|| ZpmError::NotFound(format!("{id}@{version}")))
    }

    /// Opens the archive stream. The URL is built here from validated parts and
    /// is never taken from server-supplied data.
    pub fn download(&self, id: &str, version: &Version) -> Result<(Box<dyn Read + Send + Sync>, Option<u64>)> {
        validate_id(id)?;
        let resp = self
            .get(&format!("/packages/{id}/versions/{version}/download"), &[])?
            .ok_or_else(|| ZpmError::NotFound(format!("{id}@{version}")))?;
        let len = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok());
        Ok((resp.into_reader(), len))
    }
}

// ---- all configured repositories ----

pub struct Repositories {
    repos: Vec<Repository>,
}

impl Repositories {
    pub fn new(config: &Config) -> Result<Self> {
        let repos = config.repositories.iter().map(Repository::new).collect::<Result<Vec<_>>>()?;
        Ok(Self { repos })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Repository> {
        self.repos.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    pub fn get(&self, index: usize) -> &Repository {
        &self.repos[index]
    }

    pub fn by_name(&self, name: &str) -> Result<&Repository> {
        self.repos
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| ZpmError::Config(format!("repository '{name}' is not configured")))
    }

    /// Finds which repository serves `id`. If more than one does, ZPM refuses
    /// to pick (dependency-confusion protection). An unreachable repository is
    /// an error rather than being skipped, because the answer could change.
    pub fn find(&self, id: &str) -> Result<(usize, Vec<Version>)> {
        if self.repos.is_empty() {
            return Err(ZpmError::Config("no repositories configured (see `zpm repo add`)".into()));
        }
        let mut hits = Vec::new();
        for (i, repo) in self.repos.iter().enumerate() {
            if let Some(v) = repo.versions(id)? {
                if !v.is_empty() {
                    hits.push((i, v));
                }
            }
        }
        match hits.len() {
            0 => Err(ZpmError::NotFound(id.to_string())),
            1 => Ok(hits.remove(0)),
            _ => Err(ZpmError::Ambiguous {
                id: id.to_string(),
                repos: hits.iter().map(|(i, _)| self.repos[*i].name.clone()).collect::<Vec<_>>().join(", "),
            }),
        }
    }
}

// ---- verified catalog over the network ----

/// Fetches manifests, checks their signature against the trust store and
/// confirms they say what was asked for. Results are cached per run.
pub struct RemoteCatalog<'a> {
    repos: &'a Repositories,
    trust: &'a TrustStore,
    lookups: RefCell<BTreeMap<String, (usize, Vec<Version>)>>,
    manifests: RefCell<BTreeMap<(String, Version), Manifest>>,
}

impl<'a> RemoteCatalog<'a> {
    pub fn new(repos: &'a Repositories, trust: &'a TrustStore) -> Self {
        Self { repos, trust, lookups: RefCell::new(BTreeMap::new()), manifests: RefCell::new(BTreeMap::new()) }
    }

    fn lookup(&self, id: &str) -> Result<(usize, Vec<Version>)> {
        if let Some(hit) = self.lookups.borrow().get(id) {
            return Ok(hit.clone());
        }
        let found = self.repos.find(id)?;
        self.lookups.borrow_mut().insert(id.to_string(), found.clone());
        Ok(found)
    }
}

impl Catalog for RemoteCatalog<'_> {
    fn versions(&self, id: &str) -> Result<Vec<Version>> {
        Ok(self.lookup(id)?.1)
    }

    fn manifest(&self, id: &str, version: &Version) -> Result<Manifest> {
        let key = (id.to_string(), version.clone());
        if let Some(m) = self.manifests.borrow().get(&key) {
            return Ok(m.clone());
        }
        let (index, _) = self.lookup(id)?;
        let repo = self.repos.get(index);
        let record = repo.version_record(id, version)?;
        self.trust.verify(&record.key_id, record.manifest_toml.as_bytes(), &record.signature)?;
        let manifest = Manifest::parse(&record.manifest_toml)?;
        if manifest.id() != id || manifest.version() != version {
            return Err(ZpmError::Manifest(format!(
                "'{}' served {} {} when {id} {version} was requested",
                repo.name,
                manifest.id(),
                manifest.version()
            )));
        }
        self.manifests.borrow_mut().insert(key, manifest.clone());
        Ok(manifest)
    }

    fn repository_of(&self, id: &str) -> Result<String> {
        Ok(self.repos.get(self.lookup(id)?.0).name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy() {
        assert!(check_url("https://packages.zodiacos.org/").is_ok());
        assert!(check_url("http://127.0.0.1:8080").is_ok());
        assert!(check_url("http://localhost").is_ok());
        assert!(matches!(check_url("http://packages.example.org"), Err(ZpmError::InsecureRepository(_))));
        assert!(matches!(check_url("http://localhost.evil.org"), Err(ZpmError::InsecureRepository(_))));
        assert!(check_url("https://user:pw@host").is_err());
        assert!(check_url("ftp://host").is_err());
        assert!(check_url("no-scheme").is_err());
    }
}