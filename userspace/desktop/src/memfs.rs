//! A small in-memory filesystem used by the desktop terminal until the
//! kernel VFS is reachable through syscalls.
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use zodiac_abi::{DirEntry, Fs, FsError};

struct Node {
    path: String,
    is_dir: bool,
    author: String,
    created: u64,
    modified: u64,
}

pub struct MemFs {
    nodes: Vec<Node>,
    /// Updated by the terminal before each command.
    pub now: u64,
}

fn parent_of(p: &str) -> &str {
    match p.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &p[..i],
    }
}

fn name_of(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

impl MemFs {
    pub fn new() -> Self {
        let mut fs = MemFs { nodes: Vec::new(), now: 0 };
        for d in ["/bin", "/boot", "/dev", "/etc", "/home", "/root", "/tmp", "/usr", "/var"] {
            fs.nodes.push(Node {
                path: d.to_string(),
                is_dir: true,
                author: "root".to_string(),
                created: 0,
                modified: 0,
            });
        }
        fs.nodes.push(Node {
            path: "/root/README.md".to_string(),
            is_dir: false,
            author: "root".to_string(),
            created: 0,
            modified: 0,
        });
        fs
    }

    fn exists(&self, path: &str) -> bool {
        path == "/" || self.nodes.iter().any(|n| n.path == path)
    }

    fn create(&mut self, path: &str, author: &str, is_dir: bool) -> Result<(), FsError> {
        if path == "/" || !path.starts_with('/') {
            return Err(FsError::InvalidPath);
        }
        let parent = parent_of(path);
        if !self.exists(parent) {
            return Err(FsError::NotFound);
        }
        if !self.is_dir(parent) {
            return Err(FsError::NotADirectory);
        }
        if self.exists(path) {
            return Err(FsError::Exists);
        }
        self.nodes.push(Node {
            path: path.to_string(),
            is_dir,
            author: author.to_string(),
            created: self.now,
            modified: self.now,
        });
        Ok(())
    }
}

impl Fs for MemFs {
    fn now(&self) -> u64 {
        self.now
    }

    fn is_dir(&self, path: &str) -> bool {
        path == "/" || self.nodes.iter().any(|n| n.path == path && n.is_dir)
    }

    fn new_file(&mut self, path: &str, author: &str) -> Result<(), FsError> {
        self.create(path, author, false)
    }

    fn new_dir(&mut self, path: &str, author: &str) -> Result<(), FsError> {
        self.create(path, author, true)
    }

    fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        if !self.exists(path) {
            return Err(FsError::NotFound);
        }
        if !self.is_dir(path) {
            return Err(FsError::NotADirectory);
        }
        Ok(self
            .nodes
            .iter()
            .filter(|n| parent_of(&n.path) == path)
            .map(|n| DirEntry {
                name: name_of(&n.path).to_string(),
                is_dir: n.is_dir,
                author: n.author.clone(),
                created: n.created,
                modified: n.modified,
            })
            .collect())
    }
}