use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, ZpmError};

pub const MAX_ID_LEN: usize = 64;

/// Package ids are lowercase ASCII: `org.zodiacos.sysinfo`.
pub fn validate_id(id: &str) -> Result<()> {
    let bad = || ZpmError::InvalidId(id.to_string());
    if id.len() < 3 || id.len() > MAX_ID_LEN {
        return Err(bad());
    }
    if !id.as_bytes()[0].is_ascii_alphanumeric() || id.ends_with('.') || id.contains("..") {
        return Err(bad());
    }
    let ok = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'_');
    if !id.bytes().all(ok) {
        return Err(bad());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Application,
    Driver,
    Plugin,
}

impl Category {
    /// Top-level directory under `<root>/system/` that holds this category.
    pub fn dir(self) -> &'static str {
        match self {
            Category::Application => "applications",
            Category::Driver => "drivers",
            Category::Plugin => "plugins",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Application => "application",
            Category::Driver => "driver",
            Category::Plugin => "plugin",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `id` or `id@version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSpec {
    pub id: String,
    pub version: Option<Version>,
}

impl PackageSpec {
    pub fn parse(s: &str) -> Result<Self> {
        let (id, version) = match s.split_once('@') {
            Some((id, v)) => {
                let v = Version::parse(v).map_err(|e| ZpmError::InvalidVersion(format!("'{v}': {e}")))?;
                (id, Some(v))
            }
            None => (s, None),
        };
        validate_id(id)?;
        Ok(Self { id: id.to_string(), version })
    }
}

/// A relative, forward-slash path with only normal components. No `..`, no
/// absolute paths, no backslashes. Returns the normalized path.
pub fn safe_relative(p: &str) -> Option<PathBuf> {
    if p.is_empty() || p.contains('\\') || p.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(p).components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn posix(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Forward-slash path relative to the package's install target.
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtractLimits {
    pub max_files: usize,
    pub max_unpacked_bytes: u64,
}

/// Extracts a `.zpkg` (gzip-compressed tar) into `dest`, which must be a fresh
/// staging directory. Only regular files and directories are accepted: links,
/// devices and anything that could escape `dest` are rejected, and nothing is
/// written through an existing path (`create_new`).
pub fn extract_archive(archive: &Path, dest: &Path, limits: &ExtractLimits) -> Result<Vec<FileRecord>> {
    let file = fs::File::open(archive).map_err(|e| ZpmError::io(archive, e))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    fs::create_dir_all(dest).map_err(|e| ZpmError::io(dest, e))?;

    let unreadable = |e: std::io::Error| ZpmError::UnsafeArchive(format!("unreadable archive: {e}"));
    let unsafe_path = |s: &str| ZpmError::UnsafeArchive(format!("path '{s}' is absolute, contains '..' or is malformed"));

    let mut seen = BTreeSet::new();
    let mut records = Vec::new();
    let mut total: u64 = 0;

    for entry in tar.entries().map_err(unreadable)? {
        let mut entry = entry.map_err(unreadable)?;
        let kind = entry.header().entry_type();
        let raw = entry.path().map_err(unreadable)?.into_owned();
        let raw_str = raw
            .to_str()
            .ok_or_else(|| ZpmError::UnsafeArchive(format!("non-UTF-8 path {raw:?}")))?
            .to_string();

        if kind.is_dir() {
            if matches!(raw_str.trim_end_matches('/'), "" | ".") {
                continue;
            }
            let rel = safe_relative(&raw_str).ok_or_else(|| unsafe_path(&raw_str))?;
            let dir = dest.join(rel);
            fs::create_dir_all(&dir).map_err(|e| ZpmError::io(&dir, e))?;
            continue;
        }
        if !kind.is_file() {
            return Err(ZpmError::UnsafeArchive(format!(
                "entry '{raw_str}' is a link or special file; ZPM rejects those"
            )));
        }

        let rel = safe_relative(&raw_str).ok_or_else(|| unsafe_path(&raw_str))?;
        let key = posix(&rel);
        if records.len() >= limits.max_files {
            return Err(ZpmError::UnsafeArchive(format!("more than {} files", limits.max_files)));
        }
        if !seen.insert(key.clone()) {
            return Err(ZpmError::UnsafeArchive(format!("duplicate entry '{key}'")));
        }

        let out_path = dest.join(&rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| ZpmError::io(parent, e))?;
        }
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out_path)
            .map_err(|e| ZpmError::io(&out_path, e))?;

        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf).map_err(unreadable)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > limits.max_unpacked_bytes {
                return Err(ZpmError::UnsafeArchive(format!(
                    "unpacked size exceeds the limit of {} bytes",
                    limits.max_unpacked_bytes
                )));
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| ZpmError::io(&out_path, e))?;
        }
        records.push(FileRecord { path: key, sha256: hex::encode(hasher.finalize()) });
    }

    records.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(records)
}

#[cfg(test)]
pub(crate) mod testutil {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    /// Builds a .zpkg in memory. `raw_name` bypasses the tar crate's own path
    /// checks so tests can craft hostile archives.
    pub fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            let bytes = name.as_bytes();
            h.as_gnu_mut().unwrap().name[..bytes.len()].copy_from_slice(bytes);
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            builder.append(&h, *data).unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    pub fn symlink_archive(name: &str, target: &str) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_path(name).unwrap();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_link_name(target).unwrap();
        h.set_size(0);
        h.set_cksum();
        builder.append(&h, std::io::empty()).unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    const LIMITS: ExtractLimits = ExtractLimits { max_files: 100, max_unpacked_bytes: 1024 * 1024 };

    fn extract(bytes: &[u8], limits: &ExtractLimits) -> (tempfile::TempDir, Result<Vec<FileRecord>>) {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zpkg");
        fs::write(&archive, bytes).unwrap();
        let dest = dir.path().join("out");
        let res = extract_archive(&archive, &dest, limits);
        (dir, res)
    }

    #[test]
    fn ids() {
        assert!(validate_id("org.zodiacos.sysinfo").is_ok());
        for bad in ["ab", "Org.X", "-x.y", "a..b", "a/b", "a b", "x.", ""] {
            assert!(validate_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn spec_parsing() {
        let s = PackageSpec::parse("org.x.y@1.2.3").unwrap();
        assert_eq!(s.version, Some(Version::new(1, 2, 3)));
        assert!(PackageSpec::parse("org.x.y@nope").is_err());
        assert_eq!(PackageSpec::parse("org.x.y").unwrap().version, None);
    }

    #[test]
    fn extracts_and_hashes() {
        let bytes = archive(&[("bin/tool", b"hello"), ("share/readme.txt", b"hi")]);
        let (dir, res) = extract(&bytes, &LIMITS);
        let files = res.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "bin/tool");
        assert_eq!(files[0].sha256, hex::encode(Sha256::digest(b"hello")));
        assert_eq!(fs::read(dir.path().join("out/bin/tool")).unwrap(), b"hello");
    }

    #[test]
    fn rejects_path_traversal_and_absolute() {
        for name in ["../evil", "a/../../evil", "/etc/passwd"] {
            let (dir, res) = extract(&archive(&[(name, b"x")]), &LIMITS);
            assert!(matches!(res, Err(ZpmError::UnsafeArchive(_))), "{name}");
            assert!(!dir.path().join("evil").exists());
        }
    }

    #[test]
    fn rejects_symlinks() {
        let (_d, res) = extract(&symlink_archive("link", "/etc/passwd"), &LIMITS);
        assert!(matches!(res, Err(ZpmError::UnsafeArchive(_))));
    }

    #[test]
    fn rejects_duplicates_and_limits() {
        let (_d, res) = extract(&archive(&[("a", b"1"), ("a", b"2")]), &LIMITS);
        assert!(matches!(res, Err(ZpmError::UnsafeArchive(_))));

        let small = ExtractLimits { max_files: 100, max_unpacked_bytes: 4 };
        let (_d, res) = extract(&archive(&[("a", b"12345")]), &small);
        assert!(matches!(res, Err(ZpmError::UnsafeArchive(_))));

        let few = ExtractLimits { max_files: 1, max_unpacked_bytes: 100 };
        let (_d, res) = extract(&archive(&[("a", b"1"), ("b", b"2")]), &few);
        assert!(matches!(res, Err(ZpmError::UnsafeArchive(_))));
    }

    #[test]
    fn garbage_is_rejected() {
        let (_d, res) = extract(b"not a gzip stream", &LIMITS);
        assert!(matches!(res, Err(ZpmError::UnsafeArchive(_))));
    }
}