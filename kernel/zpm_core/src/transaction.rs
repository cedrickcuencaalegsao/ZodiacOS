//! Crash-safe file transactions.
//!
//! A transaction moves staged files into place and removes old ones. Every
//! file that is replaced or removed is first moved into a backup directory,
//! and the plan is written to a journal before anything changes. That gives
//! three outcomes:
//!
//!  * `commit` + `finish`: backups are discarded.
//!  * any failure: `rollback` puts every file back and removes what was added.
//!  * a crash: `recover` runs on the next start. If the installed-package
//!    database already reflects the transaction's result it is finished,
//!    otherwise it is rolled back.

use std::fs;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::config::Layout;
use crate::database::{write_atomic, Database};
use crate::error::{Result, ZpmError};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Op {
    Place { staged: PathBuf, dest: PathBuf, backup: PathBuf },
    Remove { dest: PathBuf, backup: PathBuf },
}

/// What the database must say for the transaction to count as complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Expectation {
    id: String,
    /// `None` means the package must be absent.
    version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    ops: Vec<Op>,
    created_dirs: Vec<PathBuf>,
    expect: Vec<Expectation>,
    staging: PathBuf,
    backups: PathBuf,
    prune_stop: PathBuf,
}

pub struct Transaction {
    journal: Journal,
    journal_path: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Recovery {
    RolledBack,
    RolledForward,
}

fn tx_err(path: &Path, e: std::io::Error) -> ZpmError {
    match ZpmError::io(path, e) {
        ZpmError::Io(msg) => ZpmError::Transaction(msg),
        other => other,
    }
}

impl Transaction {
    pub fn begin(layout: &Layout) -> Result<Self> {
        let journal_path = layout.journal_file();
        if journal_path.exists() {
            return Err(ZpmError::Transaction("a previous transaction is still pending".into()));
        }
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let staging = layout.staging_dir().join(&unique);
        let backups = layout.backups_dir().join(&unique);
        for d in [&staging, &backups] {
            fs::create_dir_all(d).map_err(|e| ZpmError::io(d, e))?;
        }
        Ok(Self {
            journal: Journal {
                ops: Vec::new(),
                created_dirs: Vec::new(),
                expect: Vec::new(),
                staging,
                backups,
                prune_stop: layout.system(),
            },
            journal_path,
        })
    }

    /// A fresh directory inside this transaction's staging area.
    pub fn staging_dir(&self, name: &str) -> Result<PathBuf> {
        let dir = self.journal.staging.join(name);
        fs::create_dir_all(&dir).map_err(|e| ZpmError::io(&dir, e))?;
        Ok(dir)
    }

    fn next_backup(&self) -> PathBuf {
        self.journal.backups.join(self.journal.ops.len().to_string())
    }

    /// Queues moving `staged` to `dest`. An existing `dest` is backed up.
    pub fn place(&mut self, staged: PathBuf, dest: PathBuf) {
        let mut missing = Vec::new();
        let mut dir = dest.parent();
        while let Some(d) = dir {
            if d.exists() {
                break;
            }
            missing.push(d.to_path_buf());
            dir = d.parent();
        }
        for d in missing.into_iter().rev() {
            if !self.journal.created_dirs.contains(&d) {
                self.journal.created_dirs.push(d);
            }
        }
        let backup = self.next_backup();
        self.journal.ops.push(Op::Place { staged, dest, backup });
    }

    /// Queues removing `dest` (kept in the backup area until `finish`).
    pub fn remove(&mut self, dest: PathBuf) {
        let backup = self.next_backup();
        self.journal.ops.push(Op::Remove { dest, backup });
    }

    /// Declares the database state that means "this transaction completed".
    pub fn expect(&mut self, id: &str, version: Option<&Version>) {
        self.journal.expect.push(Expectation { id: id.to_string(), version: version.map(|v| v.to_string()) });
    }

    /// Writes the journal, then applies every operation in order.
    pub fn commit(&mut self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.journal).map_err(|e| ZpmError::Transaction(e.to_string()))?;
        write_atomic(&self.journal_path, &bytes)?;
        for d in &self.journal.created_dirs {
            fs::create_dir_all(d).map_err(|e| tx_err(d, e))?;
        }
        for op in &self.journal.ops {
            apply(op)?;
        }
        Ok(())
    }

    /// Undoes everything, best effort but idempotent. Safe to call at any point.
    pub fn rollback(&mut self) -> Result<()> {
        let mut first_err = None;
        for op in self.journal.ops.iter().rev() {
            if let Err(e) = undo(op) {
                first_err.get_or_insert(e);
            }
        }
        if first_err.is_some() {
            // Keep backups and journal so `recover` can try again.
            return Err(first_err.unwrap());
        }
        for d in self.journal.created_dirs.iter().rev() {
            let _ = fs::remove_dir(d);
        }
        self.cleanup();
        Ok(())
    }

    /// Call after the database has been updated: drops backups and the journal.
    pub fn finish(self) -> Result<()> {
        for op in &self.journal.ops {
            if let Op::Remove { dest, .. } = op {
                prune_empty(dest.parent(), &self.journal.prune_stop);
            }
        }
        self.cleanup();
        if self.journal_path.exists() {
            return Err(ZpmError::Transaction("could not remove the transaction journal".into()));
        }
        Ok(())
    }

    fn cleanup(&self) {
        let _ = fs::remove_dir_all(&self.journal.staging);
        let _ = fs::remove_dir_all(&self.journal.backups);
        let _ = fs::remove_file(&self.journal_path);
    }
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

fn remove_any(p: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(p),
        Ok(_) => fs::remove_file(p),
        Err(_) => Ok(()),
    }
}

fn apply(op: &Op) -> Result<()> {
    match op {
        Op::Place { staged, dest, backup } => {
            if exists(dest) {
                fs::rename(dest, backup).map_err(|e| tx_err(dest, e))?;
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| tx_err(parent, e))?;
            }
            fs::rename(staged, dest).map_err(|e| tx_err(dest, e))
        }
        Op::Remove { dest, backup } => {
            if exists(dest) {
                fs::rename(dest, backup).map_err(|e| tx_err(dest, e))?;
            }
            Ok(())
        }
    }
}

fn undo(op: &Op) -> Result<()> {
    match op {
        Op::Place { staged, dest, backup } => {
            if exists(backup) {
                remove_any(dest).map_err(|e| tx_err(dest, e))?;
                fs::rename(backup, dest).map_err(|e| tx_err(dest, e))?;
            } else if !exists(staged) && exists(dest) {
                // The staged file was moved into place and nothing was replaced.
                remove_any(dest).map_err(|e| tx_err(dest, e))?;
            }
            Ok(())
        }
        Op::Remove { dest, backup } => {
            if exists(backup) && !exists(dest) {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|e| tx_err(parent, e))?;
                }
                fs::rename(backup, dest).map_err(|e| tx_err(dest, e))?;
            }
            Ok(())
        }
    }
}

fn prune_empty(mut dir: Option<&Path>, stop: &Path) {
    while let Some(d) = dir {
        // Never remove `system/` itself or its top-level category directories.
        if d == stop || d.parent() == Some(stop) {
            break;
        }
        if fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// Finishes or undoes a transaction a previous run left behind.
pub fn recover(layout: &Layout) -> Result<Option<Recovery>> {
    let path = layout.journal_file();
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ZpmError::io(&path, e)),
    };
    let journal: Journal = serde_json::from_slice(&bytes)
        .map_err(|e| ZpmError::Transaction(format!("transaction journal is unreadable: {e}")))?;
    let db = Database::load(&layout.db_file())?;
    let complete = journal.expect.iter().all(|x| {
        db.get(&x.id).map(|p| p.version.to_string()) == x.version
    });
    let mut tx = Transaction { journal, journal_path: path };
    if complete {
        tx.finish()?;
        Ok(Some(Recovery::RolledForward))
    } else {
        tx.rollback()?;
        Ok(Some(Recovery::RolledBack))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::testutil::record;

    fn setup() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        layout.ensure().unwrap();
        (dir, layout)
    }

    fn stage(tx: &Transaction, name: &str, file: &str, body: &str) -> PathBuf {
        let dir = tx.staging_dir(name).unwrap();
        let path = dir.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn commit_places_files_and_finish_cleans_up() {
        let (_d, layout) = setup();
        let dest = layout.system().join("applications/org.a.app/bin/run");
        let mut tx = Transaction::begin(&layout).unwrap();
        let staged = stage(&tx, "a", "bin/run", "v1");
        tx.place(staged, dest.clone());
        tx.commit().unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "v1");
        assert!(layout.journal_file().exists());
        tx.finish().unwrap();
        assert!(!layout.journal_file().exists());
        assert_eq!(fs::read_dir(layout.staging_dir()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(layout.backups_dir()).unwrap().count(), 0);
    }

    #[test]
    fn rollback_restores_replaced_files_and_removes_new_ones() {
        let (_d, layout) = setup();
        let base = layout.system().join("applications/org.a.app");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("old"), "old").unwrap();
        fs::write(base.join("gone"), "keep me").unwrap();

        let mut tx = Transaction::begin(&layout).unwrap();
        let staged_old = stage(&tx, "a", "old", "new");
        let staged_new = stage(&tx, "b", "sub/new", "added");
        tx.place(staged_old, base.join("old"));
        tx.place(staged_new, base.join("sub/new"));
        tx.remove(base.join("gone"));
        tx.commit().unwrap();
        assert_eq!(fs::read_to_string(base.join("old")).unwrap(), "new");
        assert!(!base.join("gone").exists());

        tx.rollback().unwrap();
        assert_eq!(fs::read_to_string(base.join("old")).unwrap(), "old");
        assert_eq!(fs::read_to_string(base.join("gone")).unwrap(), "keep me");
        assert!(!base.join("sub").exists());
        assert!(!layout.journal_file().exists());
    }

    #[test]
    fn failed_commit_can_be_rolled_back() {
        let (_d, layout) = setup();
        let base = layout.system().join("applications/org.a.app");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("one"), "original").unwrap();

        let mut tx = Transaction::begin(&layout).unwrap();
        let good = stage(&tx, "a", "one", "changed");
        tx.place(good, base.join("one"));
        // This staged file does not exist, so applying the second op fails.
        tx.place(layout.staging_dir().join("missing"), base.join("two"));
        assert!(tx.commit().is_err());
        tx.rollback().unwrap();
        assert_eq!(fs::read_to_string(base.join("one")).unwrap(), "original");
        assert!(!base.join("two").exists());
    }

    #[test]
    fn recover_rolls_back_when_database_does_not_match() {
        let (_d, layout) = setup();
        let dest = layout.system().join("applications/org.a.app/bin/run");
        let mut tx = Transaction::begin(&layout).unwrap();
        let staged = stage(&tx, "a", "bin/run", "v1");
        tx.place(staged, dest.clone());
        tx.expect("org.a.app", Some(&Version::new(1, 0, 0)));
        tx.commit().unwrap();
        drop(tx); // simulated crash: no database update, no finish

        assert_eq!(recover(&layout).unwrap(), Some(Recovery::RolledBack));
        assert!(!dest.exists());
        assert!(!layout.journal_file().exists());
        assert_eq!(recover(&layout).unwrap(), None);
    }

    #[test]
    fn recover_rolls_forward_when_database_matches() {
        let (_d, layout) = setup();
        let dest = layout.system().join("applications/org.a.app/bin/run");
        let mut tx = Transaction::begin(&layout).unwrap();
        let staged = stage(&tx, "a", "bin/run", "v1");
        tx.place(staged, dest.clone());
        tx.expect("org.a.app", Some(&Version::new(1, 0, 0)));
        tx.commit().unwrap();
        drop(tx); // crash after the database was saved

        let mut db = Database::default();
        db.packages.insert("org.a.app".into(), record("org.a.app", "1.0.0", &[]));
        db.save(&layout.db_file()).unwrap();

        assert_eq!(recover(&layout).unwrap(), Some(Recovery::RolledForward));
        assert_eq!(fs::read_to_string(&dest).unwrap(), "v1");
        assert!(!layout.journal_file().exists());
    }

    #[test]
    fn finish_prunes_empty_directories_after_removal() {
        let (_d, layout) = setup();
        let base = layout.system().join("applications/org.a.app");
        fs::create_dir_all(base.join("bin")).unwrap();
        fs::write(base.join("bin/run"), "x").unwrap();
        let mut tx = Transaction::begin(&layout).unwrap();
        tx.remove(base.join("bin/run"));
        tx.commit().unwrap();
        tx.finish().unwrap();
        assert!(!base.exists());
        assert!(layout.system().join("applications").exists());
    }
}