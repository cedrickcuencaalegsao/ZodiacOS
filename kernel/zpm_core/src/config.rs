use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::{Result, ZpmError};
use crate::repository::Repositories;
use crate::signature::TrustStore;
use crate::transaction;

const MAX_CONFIG_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpgradePolicy {
    /// Take the newest compatible version.
    #[default]
    Any,
    /// Stay within the installed major version.
    SameMajor,
    /// Only patch releases of the installed major.minor.
    PatchOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Installation root. ZPM never writes outside of it.
    pub root: PathBuf,
    /// The ZodiacOS version packages are checked against.
    #[serde(default = "default_os_version")]
    pub os_version: Version,
    /// The architecture of the target system (not necessarily the host).
    #[serde(default = "default_architecture")]
    pub architecture: String,
    #[serde(default = "default_max_download")]
    pub max_download_bytes: u64,
    #[serde(default = "default_max_unpacked")]
    pub max_unpacked_bytes: u64,
    #[serde(default = "default_max_files")]
    pub max_files_per_package: usize,
    #[serde(default)]
    pub upgrade_policy: UpgradePolicy,
    #[serde(default)]
    pub repositories: Vec<RepoConfig>,
    /// Key id to hex-encoded ed25519 public key.
    #[serde(default)]
    pub trusted_keys: BTreeMap<String, String>,
    #[serde(skip)]
    pub path: PathBuf,
}

fn default_os_version() -> Version {
    Version::new(0, 1, 0)
}
fn default_architecture() -> String {
    "x86_64".to_string()
}
fn default_max_download() -> u64 {
    256 * 1024 * 1024
}
fn default_max_unpacked() -> u64 {
    1024 * 1024 * 1024
}
fn default_max_files() -> usize {
    50_000
}

impl Config {
    /// Looks for the config in this order: `--config`, `$ZPM_CONFIG`, `./zpm.toml`.
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = match explicit {
            Some(p) => p.to_path_buf(),
            None => match std::env::var_os("ZPM_CONFIG") {
                Some(p) => PathBuf::from(p),
                None => PathBuf::from("zpm.toml"),
            },
        };
        let meta = fs::metadata(&path)
            .map_err(|e| ZpmError::Config(format!("cannot read {}: {e} (pass --config or set ZPM_CONFIG)", path.display())))?;
        if meta.len() > MAX_CONFIG_BYTES {
            return Err(ZpmError::Config("config file is too large".into()));
        }
        let text = fs::read_to_string(&path).map_err(|e| ZpmError::io(&path, e))?;
        let mut config: Config =
            toml::from_str(&text).map_err(|e| ZpmError::Config(format!("{}: {}", path.display(), e.to_string().trim())))?;
        config.path = path;
        config.validate()?;
        Ok(config)
    }

    pub fn override_root(&mut self, root: Option<PathBuf>) -> Result<()> {
        if let Some(root) = root {
            self.root = root;
        }
        self.root = absolute(&self.root)?;
        self.validate()
    }

    pub fn validate(&self) -> Result<()> {
        if self.root.as_os_str().is_empty() {
            return Err(ZpmError::Config("'root' must be set".into()));
        }
        if absolute(&self.root)?.parent().is_none() {
            return Err(ZpmError::Config("'root' must not be the filesystem root".into()));
        }
        let mut names = Vec::new();
        for r in &self.repositories {
            let ok = !r.name.is_empty()
                && r.name.len() <= 64
                && r.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'));
            if !ok {
                return Err(ZpmError::Config(format!(
                    "repository name '{}' must be lowercase letters, digits, '-' or '_'",
                    r.name
                )));
            }
            if names.contains(&r.name) {
                return Err(ZpmError::Config(format!("duplicate repository '{}'", r.name)));
            }
            names.push(r.name.clone());
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| ZpmError::Config(e.to_string()))?;
        crate::database::write_atomic(&self.path, text.as_bytes())
    }

    pub fn layout(&self) -> Layout {
        Layout::new(&self.root)
    }
}

fn absolute(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir().map_err(|e| ZpmError::io(p, e))?.join(p))
    }
}

/// Everything ZPM touches lives under `<root>/system/`.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    pub fn system(&self) -> PathBuf {
        self.root.join("system")
    }
    pub fn cache(&self) -> PathBuf {
        self.system().join("cache")
    }
    pub fn archives_dir(&self) -> PathBuf {
        self.cache().join("archives")
    }
    pub fn index_dir(&self) -> PathBuf {
        self.cache().join("index")
    }
    pub fn staging_dir(&self) -> PathBuf {
        self.cache().join("staging")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.cache().join("backups")
    }
    pub fn database_dir(&self) -> PathBuf {
        self.system().join("database")
    }
    pub fn db_file(&self) -> PathBuf {
        self.database_dir().join("installed.json")
    }
    pub fn journal_file(&self) -> PathBuf {
        self.database_dir().join("transaction.json")
    }
    pub fn audit_file(&self) -> PathBuf {
        self.database_dir().join("audit.log")
    }

    pub fn ensure(&self) -> Result<()> {
        let dirs = [
            self.system().join("applications"),
            self.system().join("drivers"),
            self.system().join("plugins"),
            self.system().join("libraries"),
            self.system().join("packages"),
            self.archives_dir(),
            self.index_dir(),
            self.database_dir(),
        ];
        for d in dirs {
            fs::create_dir_all(&d).map_err(|e| ZpmError::io(&d, e))?;
        }
        Ok(())
    }
}

/// Everything a command needs: configuration, repositories and trusted keys.
pub struct Context {
    pub config: Config,
    pub layout: Layout,
    pub repos: Repositories,
    pub trust: TrustStore,
}

impl Context {
    /// Opens the installation and finishes or undoes any transaction that a
    /// crash left behind before anything else touches the filesystem.
    pub fn open(config: Config) -> Result<Self> {
        let layout = config.layout();
        layout.ensure()?;
        match transaction::recover(&layout)? {
            Some(transaction::Recovery::RolledBack) => {
                eprintln!("note: an interrupted transaction was found and rolled back");
            }
            Some(transaction::Recovery::RolledForward) => {
                eprintln!("note: an interrupted transaction had completed and was cleaned up");
            }
            None => {}
        }
        let repos = Repositories::new(&config)?;
        let trust = TrustStore::from_hex_map(&config.trusted_keys)?;
        Ok(Self { config, layout, repos, trust })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zpm.toml");
        fs::write(
            &path,
            format!(
                "root = {:?}\n[[repositories]]\nname = \"official\"\nurl = \"http://127.0.0.1:8080\"\n",
                dir.path().join("root")
            ),
        )
        .unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        assert_eq!(cfg.architecture, "x86_64");
        assert_eq!(cfg.repositories.len(), 1);
        cfg.save().unwrap();
        assert!(Config::load(Some(&path)).is_ok());
    }

    #[test]
    fn rejects_bad_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zpm.toml");
        fs::write(&path, "root = \"/\"\n").unwrap();
        assert!(Config::load(Some(&path)).is_err());
        fs::write(&path, "root = \"/tmp/x\"\nbogus = 1\n").unwrap();
        assert!(Config::load(Some(&path)).is_err());
        fs::write(&path, "root = \"/tmp/x\"\n[[repositories]]\nname = \"Bad Name\"\nurl = \"https://x\"\n").unwrap();
        assert!(Config::load(Some(&path)).is_err());
    }
}