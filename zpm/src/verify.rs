use std::fs;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::config::Layout;
use crate::database::InstalledPackage;
use crate::error::{Result, ZpmError};

#[cfg(test)]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|e| ZpmError::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| ZpmError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn check_checksum(what: &str, expected: &str, actual: &str) -> Result<()> {
    if expected.eq_ignore_ascii_case(actual) {
        Ok(())
    } else {
        Err(ZpmError::ChecksumMismatch {
            what: what.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        })
    }
}

#[derive(Debug, Default)]
pub struct VerifyReport {
    pub checked: usize,
    pub missing: Vec<String>,
    pub modified: Vec<String>,
}

impl VerifyReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.modified.is_empty()
    }
}

/// Re-hashes every file the package owns and compares it with the record
/// made at install time.
pub fn verify_installed(layout: &Layout, pkg: &InstalledPackage) -> Result<VerifyReport> {
    let root = layout.system().join(&pkg.target);
    let mut report = VerifyReport::default();
    for file in &pkg.files {
        let path = root.join(&file.path);
        report.checked += 1;
        if fs::symlink_metadata(&path).is_err() {
            report.missing.push(file.path.clone());
        } else if !sha256_file(&path)?.eq_ignore_ascii_case(&file.sha256) {
            report.modified.push(file.path.clone());
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::testutil::record;
    use crate::package::FileRecord;

    #[test]
    fn checksum_comparison() {
        let sum = sha256_hex(b"abc");
        assert!(check_checksum("x", &sum, &sum.to_uppercase()).is_ok());
        assert!(matches!(check_checksum("x", &sum, &sha256_hex(b"abd")), Err(ZpmError::ChecksumMismatch { .. })));
    }

    #[test]
    fn detects_missing_and_modified_files() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        let mut pkg = record("org.a.app", "1.0.0", &[]);
        let root = layout.system().join(&pkg.target);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a"), b"one").unwrap();
        fs::write(root.join("b"), b"two").unwrap();
        pkg.files = vec![
            FileRecord { path: "a".into(), sha256: sha256_hex(b"one") },
            FileRecord { path: "b".into(), sha256: sha256_hex(b"two") },
            FileRecord { path: "c".into(), sha256: sha256_hex(b"three") },
        ];
        assert!(verify_installed(&layout, &pkg).unwrap().missing == ["c"]);

        fs::write(root.join("b"), b"tampered").unwrap();
        let report = verify_installed(&layout, &pkg).unwrap();
        assert_eq!(report.modified, ["b"]);
        assert!(!report.is_clean());
    }
}