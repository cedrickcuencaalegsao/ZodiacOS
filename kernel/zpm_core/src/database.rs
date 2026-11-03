use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::config::Layout;
use crate::error::{Result, ZpmError};
use crate::package::{Category, FileRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    /// The user asked for it.
    Manual,
    /// Pulled in to satisfy a dependency.
    Dependency,
}

/// What ZPM knows about an installed package. `files` is the ownership
/// record: removal and upgrades only ever touch paths listed here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub category: Category,
    /// Relative to `<root>/system/`.
    pub target: String,
    pub entrypoint: String,
    pub repository: String,
    pub reason: Reason,
    pub installed_at: u64,
    #[serde(default)]
    pub requires: BTreeMap<String, String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    pub files: Vec<FileRecord>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Database {
    #[serde(default)]
    pub packages: BTreeMap<String, InstalledPackage>,
}

impl Database {
    /// A missing file is an empty database; a corrupt one is an error, never
    /// silently reset.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| ZpmError::Database(format!("{} is corrupt: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ZpmError::io(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| ZpmError::Database(e.to_string()))?;
        write_atomic(path, &bytes)
    }

    pub fn get(&self, id: &str) -> Option<&InstalledPackage> {
        self.packages.get(id)
    }

    /// Installed packages that require `id`, with the requirement they declared.
    pub fn dependents(&self, id: &str) -> Vec<(&InstalledPackage, &String)> {
        self.packages
            .values()
            .filter_map(|p| p.requires.get(id).map(|req| (p, req)))
            .collect()
    }
}

/// Writes to a temporary file next to `path`, syncs it, then renames it over
/// the target, so readers see either the old or the new content.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let write = || -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        ZpmError::io(path, e)
    })
}

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Appends one line to the audit log. Failures are reported but never abort
/// the operation that was already carried out.
pub fn audit(layout: &Layout, action: &str, detail: &str) {
    let line = format!("{} {} {}\n", now_unix(), action, detail);
    let result = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(layout.audit_file())
        .and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = result {
        eprintln!("warning: could not write audit log: {e}");
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    pub fn record(id: &str, version: &str, requires: &[(&str, &str)]) -> InstalledPackage {
        InstalledPackage {
            id: id.into(),
            name: id.into(),
            version: Version::parse(version).unwrap(),
            category: Category::Application,
            target: format!("applications/{id}"),
            entrypoint: "bin/run".into(),
            repository: "test".into(),
            reason: Reason::Manual,
            installed_at: 0,
            requires: requires.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            conflicts: vec![],
            files: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn roundtrip_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        assert!(Database::load(&path).unwrap().packages.is_empty());

        let mut db = Database::default();
        db.packages.insert("org.a.a".into(), record("org.a.a", "1.0.0", &[]));
        db.save(&path).unwrap();
        let back = Database::load(&path).unwrap();
        assert_eq!(back.get("org.a.a").unwrap().version, Version::new(1, 0, 0));
        assert!(!dir.path().join("installed.tmp").exists());
    }

    #[test]
    fn corrupt_database_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(matches!(Database::load(&path), Err(ZpmError::Database(_))));
    }

    #[test]
    fn finds_dependents() {
        let mut db = Database::default();
        db.packages.insert("org.a.lib".into(), record("org.a.lib", "1.0.0", &[]));
        db.packages.insert("org.a.app".into(), record("org.a.app", "1.0.0", &[("org.a.lib", ">=1")]));
        assert_eq!(db.dependents("org.a.lib").len(), 1);
        assert!(db.dependents("org.a.app").is_empty());
    }
}