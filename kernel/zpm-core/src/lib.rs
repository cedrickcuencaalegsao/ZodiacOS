#![no_std]

// #[cfg(feature = "std")]
// extern crate std;

/// Maximum length of a package id.
pub const ID_MAX: usize = 32;

/// Sizes (in bytes) of the regions packages may use.
/// Field meaning is a guess from the call site: adjust names to taste.
#[derive(Clone, Copy, Debug)]
pub struct PackageSpace {
    pub system: u64,
    pub apps: u64,
    pub config: u64,
    pub data: u64,
}

impl PackageSpace {
    pub const fn new(system: u64, apps: u64, config: u64, data: u64) -> Self {
        Self {
            system,
            apps,
            config,
            data,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    App,
    Library,
    Service,
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Version {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

/// Fixed-size package id, so no heap is needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageId {
    buf: [u8; ID_MAX],
    len: u8,
}

impl PackageId {
    /// Returns `None` if the id is empty or longer than `ID_MAX`.
    pub fn new(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.is_empty() || b.len() > ID_MAX {
            return None;
        }
        let mut buf = [0u8; ID_MAX];
        buf[..b.len()].copy_from_slice(b);
        Some(Self {
            buf,
            len: b.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Package {
    pub id: PackageId,
    pub version: Version,
    pub kind: PackageKind,
}

impl Package {
    pub fn new(id: &str, version: Version, kind: PackageKind) -> Option<Self> {
        Some(Self {
            id: PackageId::new(id)?,
            version,
            kind,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    Full,
    AlreadyInstalled,
    NotFound,
}

/// In-memory table of installed packages, at most `N` of them.
pub struct Registry<const N: usize> {
    slots: [Option<Package>; N],
    space: PackageSpace,
}

impl<const N: usize> Registry<N> {
    pub const fn new(space: PackageSpace) -> Self {
        Self {
            slots: [None; N],
            space,
        }
    }

    pub fn space(&self) -> &PackageSpace {
        &self.space
    }

    pub fn iter(&self) -> impl Iterator<Item = &Package> {
        self.slots.iter().filter_map(|s| s.as_ref())
    }

    pub fn get(&self, id: &str) -> Option<&Package> {
        self.iter().find(|p| p.id.as_str() == id)
    }

    pub fn install(&mut self, pkg: Package) -> Result<(), RegistryError> {
        if self.get(pkg.id.as_str()).is_some() {
            return Err(RegistryError::AlreadyInstalled);
        }
        match self.slots.iter_mut().find(|s| s.is_none()) {
            Some(slot) => {
                *slot = Some(pkg);
                Ok(())
            }
            None => Err(RegistryError::Full),
        }
    }

    pub fn remove(&mut self, id: &str) -> Result<(), RegistryError> {
        for slot in self.slots.iter_mut() {
            if matches!(slot, Some(p) if p.id.as_str() == id) {
                *slot = None;
                return Ok(());
            }
        }
        Err(RegistryError::NotFound)
    }
}
