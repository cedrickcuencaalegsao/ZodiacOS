use std::collections::BTreeMap;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::error::{Result, ZpmError};
use crate::package::{safe_relative, validate_id, Category};

pub const SCHEMA_VERSION: u32 = 1;
pub const SUPPORTED_ARCHITECTURES: &[&str] = &["x86_64", "aarch64"];
/// Package API versions this client understands.
pub const SUPPORTED_API_VERSIONS: &[&str] = &["1"];
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub package: PackageSection,
    pub compatibility: Compatibility,
    #[serde(default)]
    pub dependencies: Dependencies,
    pub artifacts: Artifacts,
    pub installation: Installation,
    /// Required for `category = "driver"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<DriverMeta>,
    /// Required for `category = "plugin"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSection {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub description: String,
    pub category: Category,
    pub architecture: Vec<String>,
    pub license: String,
    pub publisher: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub minimum_os_version: Version,
    /// A version, or "*" for no upper bound.
    pub maximum_os_version: String,
    pub api_version: String,
}

/// Maps package id to a semver requirement such as ">=1.2.0, <2.0.0".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {
    #[serde(default)]
    pub required: BTreeMap<String, String>,
    /// Recorded but never installed automatically.
    #[serde(default)]
    pub optional: BTreeMap<String, String>,
    /// Package ids that must not be installed alongside this one.
    #[serde(default)]
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifacts {
    /// File name of the archive on the server; never a path.
    pub archive: String,
    pub checksum_algorithm: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    /// Relative to `<root>/system/`, e.g. `applications/example-app`.
    pub target: String,
    /// Relative to the target.
    pub entrypoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriverMeta {
    /// Hardware ids this driver handles, e.g. `pci:8086:100e`.
    pub hardware: Vec<String>,
    /// Kernel driver interface the package was built against.
    pub kernel_interface: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMeta {
    /// Host application id to version requirement.
    pub hosts: BTreeMap<String, String>,
    pub interface: String,
}

fn bad(msg: impl Into<String>) -> ZpmError {
    ZpmError::Manifest(msg.into())
}

fn check_text(field: &str, value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max {
        return Err(bad(format!("'{field}' must be 1-{max} characters")));
    }
    Ok(())
}

fn check_requirements(section: &str, own_id: &str, map: &BTreeMap<String, String>) -> Result<()> {
    for (id, req) in map {
        validate_id(id).map_err(|_| bad(format!("{section}: invalid package id '{id}'")))?;
        if id == own_id {
            return Err(bad(format!("{section}: package cannot reference itself")));
        }
        VersionReq::parse(req).map_err(|e| bad(format!("{section}: requirement '{req}' for '{id}': {e}")))?;
    }
    Ok(())
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_MANIFEST_BYTES {
            return Err(bad("manifest is too large"));
        }
        let manifest: Manifest = toml::from_str(text).map_err(|e| bad(e.to_string().trim().to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn id(&self) -> &str {
        &self.package.id
    }

    pub fn version(&self) -> &Version {
        &self.package.version
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(bad(format!(
                "unsupported schema_version {} (this client understands {})",
                self.schema_version, SCHEMA_VERSION
            )));
        }
        let p = &self.package;
        validate_id(&p.id)?;
        check_text("package.name", &p.name, 128)?;
        check_text("package.description", &p.description, 2048)?;
        check_text("package.license", &p.license, 64)?;
        check_text("package.publisher", &p.publisher, 128)?;

        if p.architecture.is_empty() {
            return Err(bad("package.architecture must list at least one architecture"));
        }
        for (i, a) in p.architecture.iter().enumerate() {
            if !SUPPORTED_ARCHITECTURES.contains(&a.as_str()) {
                return Err(bad(format!("unsupported architecture '{a}'")));
            }
            if p.architecture[..i].contains(a) {
                return Err(bad(format!("duplicate architecture '{a}'")));
            }
        }

        let c = &self.compatibility;
        if c.maximum_os_version != "*" {
            let max = Version::parse(&c.maximum_os_version)
                .map_err(|e| bad(format!("compatibility.maximum_os_version: {e}")))?;
            if max < c.minimum_os_version {
                return Err(bad("compatibility.maximum_os_version is lower than minimum_os_version"));
            }
        }
        if c.api_version.is_empty() || !c.api_version.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad("compatibility.api_version must be a number such as \"1\""));
        }

        check_requirements("dependencies.required", &p.id, &self.dependencies.required)?;
        check_requirements("dependencies.optional", &p.id, &self.dependencies.optional)?;
        for id in &self.dependencies.conflicts {
            validate_id(id).map_err(|_| bad(format!("dependencies.conflicts: invalid package id '{id}'")))?;
            if *id == p.id {
                return Err(bad("dependencies.conflicts: package cannot conflict with itself"));
            }
        }
        if let Some(id) = self.dependencies.required.keys().find(|k| self.dependencies.conflicts.contains(k)) {
            return Err(bad(format!("'{id}' is both required and conflicting")));
        }

        let a = &self.artifacts;
        if a.checksum_algorithm != "sha256" {
            return Err(bad("artifacts.checksum_algorithm must be \"sha256\""));
        }
        if a.checksum.len() != 64 || !a.checksum.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(bad("artifacts.checksum must be 64 lowercase hex characters"));
        }
        if a.archive.is_empty()
            || a.archive.len() > 128
            || a.archive.starts_with('.')
            || !a.archive.ends_with(".zpkg")
            || !a.archive.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
        {
            return Err(bad("artifacts.archive must be a plain file name ending in .zpkg"));
        }

        let target = safe_relative(&self.installation.target)
            .ok_or_else(|| bad("installation.target must be a relative path without '..'"))?;
        let parts: Vec<_> = target.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if parts.len() < 2 || parts[0] != p.category.dir() {
            return Err(bad(format!(
                "installation.target must be inside '{}/' for category '{}'",
                p.category.dir(),
                p.category
            )));
        }
        let name_ok = |s: &String| s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if !parts.iter().all(name_ok) {
            return Err(bad("installation.target may only contain letters, digits, '.', '-' and '_'"));
        }
        if safe_relative(&self.installation.entrypoint).is_none() {
            return Err(bad("installation.entrypoint must be a relative path without '..'"));
        }

        match (p.category, &self.driver, &self.plugin) {
            (Category::Driver, Some(d), None) => {
                if d.hardware.is_empty() {
                    return Err(bad("driver.hardware must list at least one hardware id"));
                }
                check_text("driver.kernel_interface", &d.kernel_interface, 64)?;
            }
            (Category::Plugin, None, Some(pl)) => {
                if pl.hosts.is_empty() {
                    return Err(bad("plugin.hosts must name at least one host application"));
                }
                check_requirements("plugin.hosts", &p.id, &pl.hosts)?;
                check_text("plugin.interface", &pl.interface, 64)?;
            }
            (Category::Application, None, None) => {}
            (Category::Driver, _, _) => return Err(bad("drivers need a [driver] section and no [plugin] section")),
            (Category::Plugin, _, _) => return Err(bad("plugins need a [plugin] section and no [driver] section")),
            (Category::Application, _, _) => return Err(bad("applications cannot have [driver] or [plugin] sections")),
        }
        Ok(())
    }

    /// Required dependencies as parsed requirements, in id order.
    pub fn required(&self) -> Result<Vec<(String, VersionReq)>> {
        self.dependencies
            .required
            .iter()
            .map(|(id, r)| {
                VersionReq::parse(r)
                    .map(|req| (id.clone(), req))
                    .map_err(|e| bad(format!("requirement '{r}' for '{id}': {e}")))
            })
            .collect()
    }

    pub fn check_compatibility(&self, os: &Version, arch: &str) -> Result<()> {
        let id = format!("{} {}", self.package.id, self.package.version);
        if !self.package.architecture.iter().any(|a| a == arch) {
            return Err(ZpmError::Incompatible(format!(
                "{id} supports {} but this system is {arch}",
                self.package.architecture.join(", ")
            )));
        }
        let c = &self.compatibility;
        if *os < c.minimum_os_version {
            return Err(ZpmError::Incompatible(format!(
                "{id} needs ZodiacOS >= {} (this is {os})",
                c.minimum_os_version
            )));
        }
        if c.maximum_os_version != "*" {
            if let Ok(max) = Version::parse(&c.maximum_os_version) {
                if *os > max {
                    return Err(ZpmError::Incompatible(format!("{id} supports ZodiacOS up to {max} (this is {os})")));
                }
            }
        }
        if !SUPPORTED_API_VERSIONS.contains(&c.api_version.as_str()) {
            return Err(ZpmError::Incompatible(format!("{id} needs package API {}", c.api_version)));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    pub const ZERO_SUM: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    pub fn text(id: &str, version: &str, deps: &[(&str, &str)]) -> String {
        let deps: String = deps.iter().map(|(i, r)| format!("\"{i}\" = \"{r}\"\n")).collect();
        format!(
            r#"schema_version = 1

[package]
id = "{id}"
name = "Test {id}"
version = "{version}"
description = "A test package."
category = "application"
architecture = ["x86_64"]
license = "MIT"
publisher = "Test"

[compatibility]
minimum_os_version = "0.1.0"
maximum_os_version = "*"
api_version = "1"

[dependencies.required]
{deps}
[artifacts]
archive = "{id}-{version}.zpkg"
checksum_algorithm = "sha256"
checksum = "{ZERO_SUM}"

[installation]
target = "applications/{id}"
entrypoint = "bin/run"
"#
        )
    }

    pub fn manifest(id: &str, version: &str, deps: &[(&str, &str)]) -> Manifest {
        Manifest::parse(&text(id, version, deps)).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn replace(from: &str, to: &str) -> String {
        text("org.test.app", "1.0.0", &[]).replace(from, to)
    }

    #[test]
    fn parses_valid_manifest() {
        let m = Manifest::parse(&text("org.test.app", "1.2.3", &[("org.test.lib", ">=1.0.0")])).unwrap();
        assert_eq!(m.id(), "org.test.app");
        assert_eq!(m.version(), &Version::new(1, 2, 3));
        assert_eq!(m.required().unwrap().len(), 1);
    }

    #[test]
    fn rejects_missing_fields() {
        let broken = text("org.test.app", "1.0.0", &[]).replace("license = \"MIT\"\n", "");
        assert!(matches!(Manifest::parse(&broken), Err(ZpmError::Manifest(_))));
        assert!(matches!(Manifest::parse("schema_version = 1"), Err(ZpmError::Manifest(_))));
    }

    #[test]
    fn rejects_unknown_fields() {
        let extra = replace("license = \"MIT\"", "license = \"MIT\"\nsurprise = true");
        assert!(Manifest::parse(&extra).is_err());
    }

    #[test]
    fn rejects_invalid_id_and_version() {
        assert!(Manifest::parse(&replace("id = \"org.test.app\"", "id = \"Bad ID\"")).is_err());
        assert!(Manifest::parse(&replace("version = \"1.0.0\"", "version = \"one\"")).is_err());
    }

    #[test]
    fn rejects_unsupported_architecture_and_schema() {
        assert!(Manifest::parse(&replace("[\"x86_64\"]", "[\"sparc\"]")).is_err());
        assert!(Manifest::parse(&replace("schema_version = 1", "schema_version = 2")).is_err());
    }

    #[test]
    fn rejects_malformed_dependencies() {
        let bad_req = text("org.test.app", "1.0.0", &[("org.test.lib", "not a req")]);
        assert!(Manifest::parse(&bad_req).is_err());
        let bad_id = text("org.test.app", "1.0.0", &[("BAD", "*")]);
        assert!(Manifest::parse(&bad_id).is_err());
        let selfdep = text("org.test.app", "1.0.0", &[("org.test.app", "*")]);
        assert!(Manifest::parse(&selfdep).is_err());
    }

    #[test]
    fn rejects_unsafe_paths_and_artifacts() {
        assert!(Manifest::parse(&replace("applications/org.test.app", "applications/../../etc")).is_err());
        assert!(Manifest::parse(&replace("applications/org.test.app", "/etc/passwd")).is_err());
        assert!(Manifest::parse(&replace("applications/org.test.app", "drivers/org.test.app")).is_err());
        assert!(Manifest::parse(&replace("entrypoint = \"bin/run\"", "entrypoint = \"../run\"")).is_err());
        assert!(Manifest::parse(&replace("org.test.app-1.0.0.zpkg", "../x.zpkg")).is_err());
        assert!(Manifest::parse(&replace(ZERO_SUM, "abc")).is_err());
        assert!(Manifest::parse(&replace("\"sha256\"", "\"md5\"")).is_err());
    }

    #[test]
    fn category_sections_are_enforced() {
        let driver = replace("category = \"application\"", "category = \"driver\"")
            .replace("applications/org.test.app", "drivers/org.test.app");
        assert!(Manifest::parse(&driver).is_err());
        let ok = format!("{driver}\n[driver]\nhardware = [\"pci:8086:100e\"]\nkernel_interface = \"1\"\n");
        assert!(Manifest::parse(&ok).is_ok());
    }

    #[test]
    fn compatibility_checks() {
        let m = manifest("org.test.app", "1.0.0", &[]);
        assert!(m.check_compatibility(&Version::new(0, 1, 0), "x86_64").is_ok());
        assert!(matches!(m.check_compatibility(&Version::new(0, 0, 9), "x86_64"), Err(ZpmError::Incompatible(_))));
        assert!(matches!(m.check_compatibility(&Version::new(0, 1, 0), "aarch64"), Err(ZpmError::Incompatible(_))));

        let capped = Manifest::parse(&replace("maximum_os_version = \"*\"", "maximum_os_version = \"0.5.0\"")).unwrap();
        assert!(capped.check_compatibility(&Version::new(0, 6, 0), "x86_64").is_err());
    }
}