use crate::base::error::{Error, Result};
use alloc::{string::String, vec::Vec};

pub const MAX_PATH: usize = 4096;
pub const MAX_NAME: usize = 255;

/// A single path component: no slashes, not empty, not "." or "..".
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(Error::InvalidArgument);
    }
    if name.len() > MAX_NAME {
        return Err(Error::NameTooLong);
    }
    Ok(())
}

/// Turns `path` (absolute, or relative to `cwd`) into its canonical absolute form:
/// no ".", no "..", no repeated or trailing slashes.
pub fn resolve_against(cwd: &str, path: &str) -> Result<String> {
    if path.is_empty() {
        return Err(Error::NotFound);
    }
    if path.len() > MAX_PATH {
        return Err(Error::NameTooLong);
    }
    let mut parts: Vec<&str> = Vec::new();
    let base = if path.starts_with('/') { "" } else { cwd };
    for comp in base.split('/').chain(path.split('/')) {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => {
                if c.len() > MAX_NAME {
                    return Err(Error::NameTooLong);
                }
                parts.push(c);
            }
        }
    }
    let mut out = String::new();
    if parts.is_empty() {
        out.push('/');
    }
    for part in parts {
        out.push('/');
        out.push_str(part);
    }
    Ok(out)
}

/// "/etc/zpm/x" -> ("/etc/zpm", "x"). A top-level name has parent "/".
pub fn split_parent(path: &str) -> Result<(&str, &str)> {
    if path == "/" {
        return Err(Error::InvalidArgument);
    }
    let idx = path.rfind('/').ok_or(Error::InvalidArgument)?;
    let parent = if idx == 0 { "/" } else { &path[..idx] };
    Ok((parent, &path[idx + 1..]))
}
