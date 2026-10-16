use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::Context;
use crate::error::{Result, ZpmError};
use crate::manifest::Manifest;
use crate::repository::Repository;
use crate::verify;

/// Downloads the archive for `manifest` into the cache and returns its path.
/// The cache is content-addressed by the manifest's SHA-256, so a cached file
/// is only reused after it hashes correctly again.
pub fn fetch_archive(ctx: &Context, repo: &Repository, manifest: &Manifest) -> Result<PathBuf> {
    let dir = ctx.layout.archives_dir();
    fs::create_dir_all(&dir).map_err(|e| ZpmError::io(&dir, e))?;
    let expected = &manifest.artifacts.checksum;
    let final_path = dir.join(format!("{expected}.zpkg"));

    if final_path.exists() {
        if verify::sha256_file(&final_path)?.eq_ignore_ascii_case(expected) {
            return Ok(final_path);
        }
        let _ = fs::remove_file(&final_path);
    }

    let part = dir.join(format!("{expected}.part"));
    let label = format!("{} {}", manifest.id(), manifest.version());
    let result = repo.download(manifest.id(), manifest.version()).and_then(|(reader, len)| {
        copy_verified(reader, len, ctx.config.max_download_bytes, expected, &part, &label, true)
    });
    match result {
        Ok(()) => {
            fs::rename(&part, &final_path).map_err(|e| ZpmError::io(&final_path, e))?;
            Ok(final_path)
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Streams `reader` into `part`, enforcing the size limit while reading and
/// checking length and SHA-256 at the end. Never holds the archive in memory.
pub fn copy_verified(
    mut reader: impl Read,
    declared_len: Option<u64>,
    max_bytes: u64,
    expected_sha256: &str,
    part: &Path,
    label: &str,
    show_progress: bool,
) -> Result<()> {
    if declared_len.map_or(false, |l| l > max_bytes) {
        return Err(ZpmError::TooLarge(max_bytes));
    }
    let mut out = fs::File::create(part).map_err(|e| ZpmError::io(part, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    let mut next_report = 10;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|_| ZpmError::Interrupted { expected: declared_len.unwrap_or(0), received: total })?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max_bytes {
            return Err(ZpmError::TooLarge(max_bytes));
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| ZpmError::io(part, e))?;
        if let (true, Some(len)) = (show_progress, declared_len) {
            let pct = total * 100 / len.max(1);
            if pct >= next_report {
                eprint!("\r  downloading {label}: {pct}%");
                next_report = (pct / 10 + 1) * 10;
            }
        }
    }
    if show_progress && declared_len.is_some() {
        eprintln!();
    }
    if let Some(len) = declared_len {
        if total != len {
            return Err(ZpmError::Interrupted { expected: len, received: total });
        }
    }
    out.sync_all().map_err(|e| ZpmError::io(part, e))?;
    verify::check_checksum(label, expected_sha256, &hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct Flaky {
        data: Vec<u8>,
        pos: usize,
        fail_at: usize,
    }

    impl Read for Flaky {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.fail_at {
                return Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
            }
            let n = buf.len().min(self.fail_at - self.pos).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    fn run(data: &[u8], len: Option<u64>, max: u64, sum: &str) -> (tempfile::TempDir, Result<()>) {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        let res = copy_verified(data, len, max, sum, &part, "test", false);
        (dir, res)
    }

    #[test]
    fn successful_download() {
        let data = vec![7u8; 200_000];
        let sum = verify::sha256_hex(&data);
        let (dir, res) = run(&data, Some(data.len() as u64), 1 << 20, &sum);
        res.unwrap();
        assert_eq!(fs::read(dir.path().join("a.part")).unwrap(), data);
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let (_d, res) = run(b"corrupted", Some(9), 1 << 20, &verify::sha256_hex(b"original"));
        assert!(matches!(res, Err(ZpmError::ChecksumMismatch { .. })));
    }

    #[test]
    fn oversized_download_is_rejected() {
        let data = vec![0u8; 5000];
        let sum = verify::sha256_hex(&data);
        let (_d, res) = run(&data, Some(5000), 1000, &sum);
        assert!(matches!(res, Err(ZpmError::TooLarge(1000))));
        // Also when the server does not declare a length.
        let (_d, res) = run(&data, None, 1000, &sum);
        assert!(matches!(res, Err(ZpmError::TooLarge(1000))));
    }

    #[test]
    fn interrupted_download_is_detected() {
        let data = vec![1u8; 10_000];
        let sum = verify::sha256_hex(&data);
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        let flaky = Flaky { data: data.clone(), pos: 0, fail_at: 4000 };
        let res = copy_verified(flaky, Some(10_000), 1 << 20, &sum, &part, "test", false);
        assert!(matches!(res, Err(ZpmError::Interrupted { received: 4000, .. })));

        // A stream that ends early without an error is caught by the length check.
        let (_d, res) = run(&data[..6000], Some(10_000), 1 << 20, &sum);
        assert!(matches!(res, Err(ZpmError::Interrupted { expected: 10_000, received: 6000 })));
    }
}