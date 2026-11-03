//! Deterministic dependency resolution.
//!
//! For each requested package the resolver picks the highest version that
//! satisfies every constraint seen so far and is compatible with this system,
//! then resolves that version's own dependencies depth-first. It does not
//! backtrack: if a later constraint contradicts an earlier choice it reports
//! the conflict instead of guessing, so the same inputs always give the same
//! plan or the same error.

use std::collections::BTreeMap;

use semver::{Version, VersionReq};

use crate::database::{Database, Reason};
use crate::error::{Result, ZpmError};
use crate::manifest::Manifest;

pub trait Catalog {
    /// All versions of `id` (any order). `NotFound` if it does not exist.
    fn versions(&self, id: &str) -> Result<Vec<Version>>;
    /// The verified manifest of one version.
    fn manifest(&self, id: &str, version: &Version) -> Result<Manifest>;
    /// Name of the repository that serves `id`.
    fn repository_of(&self, id: &str) -> Result<String>;
}

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub manifest: Manifest,
    pub reason: Reason,
    /// The installed version this item replaces, if any.
    pub replaces: Option<Version>,
    pub repository: String,
}

/// Packages to install, dependencies first.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub items: Vec<PlanItem>,
}

pub struct Request {
    pub id: String,
    pub req: VersionReq,
    /// Replace an installed version instead of refusing.
    pub upgrade: bool,
}

pub struct Environment<'a> {
    pub os_version: &'a Version,
    pub architecture: &'a str,
}

/// The newest version of `id` that matches `reqs`, is newer than `above`
/// (if given) and is compatible with this system.
pub fn pick_version(
    catalog: &dyn Catalog,
    env: &Environment,
    id: &str,
    reqs: &[&VersionReq],
    above: Option<&Version>,
) -> Result<Manifest> {
    let mut versions = catalog.versions(id)?;
    versions.sort();
    versions.reverse();
    let mut last_incompat = None;
    let mut any_match = false;
    for v in versions {
        if above.map_or(false, |a| v <= *a) {
            break;
        }
        if !reqs.iter().all(|r| r.matches(&v)) {
            continue;
        }
        any_match = true;
        let manifest = catalog.manifest(id, &v)?;
        match manifest.check_compatibility(env.os_version, env.architecture) {
            Ok(()) => return Ok(manifest),
            Err(e) => last_incompat = Some(e),
        }
    }
    let wanted = reqs.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" and ");
    match last_incompat {
        Some(e) if any_match => Err(ZpmError::Resolution(format!("no installable version of '{id}' ({wanted}): {e}"))),
        _ => Err(ZpmError::Resolution(format!("no version of '{id}' satisfies {wanted}"))),
    }
}

struct Resolver<'a> {
    env: &'a Environment<'a>,
    installed: &'a Database,
    catalog: &'a dyn Catalog,
    upgrade_root: Option<&'a str>,
    constraints: BTreeMap<String, Vec<(String, VersionReq)>>,
    chosen: BTreeMap<String, Manifest>,
    order: Vec<String>,
    stack: Vec<String>,
}

impl Resolver<'_> {
    fn reqs_for(&self, id: &str) -> Vec<&VersionReq> {
        self.constraints.get(id).map(|v| v.iter().map(|(_, r)| r).collect()).unwrap_or_default()
    }

    fn describe(&self, id: &str) -> String {
        self.constraints
            .get(id)
            .map(|v| v.iter().map(|(by, r)| format!("{by} wants {r}")).collect::<Vec<_>>().join("; "))
            .unwrap_or_default()
    }

    fn visit(&mut self, id: &str, req: VersionReq, requested_by: &str) -> Result<()> {
        if self.stack.iter().any(|s| s == id) {
            let mut cycle = self.stack.clone();
            cycle.push(id.to_string());
            return Err(ZpmError::Resolution(format!("dependency cycle: {}", cycle.join(" -> "))));
        }
        self.constraints.entry(id.to_string()).or_default().push((requested_by.to_string(), req));

        if let Some(chosen) = self.chosen.get(id) {
            if self.reqs_for(id).iter().all(|r| r.matches(chosen.version())) {
                return Ok(());
            }
            return Err(ZpmError::Resolution(format!(
                "version conflict on '{id}': {} was selected, but {}",
                chosen.version(),
                self.describe(id)
            )));
        }

        let is_upgrade_root = self.upgrade_root == Some(id);
        if !is_upgrade_root {
            if let Some(installed) = self.installed.get(id) {
                if self.reqs_for(id).iter().all(|r| r.matches(&installed.version)) {
                    return Ok(());
                }
                return Err(ZpmError::Resolution(format!(
                    "installed '{id}' {} does not satisfy: {}; upgrade it first",
                    installed.version,
                    self.describe(id)
                )));
            }
        }

        let above = if is_upgrade_root { self.installed.get(id).map(|i| i.version.clone()) } else { None };
        let reqs = self.reqs_for(id);
        let manifest = pick_version(self.catalog, self.env, id, &reqs, above.as_ref())
            .map_err(|e| match e {
                ZpmError::Resolution(m) if requested_by != "you" => {
                    ZpmError::Resolution(format!("{m} (needed by {requested_by})"))
                }
                other => other,
            })?;

        self.stack.push(id.to_string());
        for (dep, dep_req) in manifest.required()? {
            self.visit(&dep, dep_req, id)?;
        }
        self.stack.pop();

        self.chosen.insert(id.to_string(), manifest);
        self.order.push(id.to_string());
        Ok(())
    }
}

pub fn resolve(request: &Request, env: &Environment, installed: &Database, catalog: &dyn Catalog) -> Result<Plan> {
    if !request.upgrade {
        if installed.get(&request.id).is_some() {
            return Err(ZpmError::AlreadyInstalled(request.id.clone()));
        }
    }
    let mut resolver = Resolver {
        env,
        installed,
        catalog,
        upgrade_root: request.upgrade.then_some(request.id.as_str()),
        constraints: BTreeMap::new(),
        chosen: BTreeMap::new(),
        order: Vec::new(),
        stack: Vec::new(),
    };
    resolver.visit(&request.id, request.req.clone(), "you")?;

    // Declared conflicts, in both directions, against what will exist afterwards.
    for (id, manifest) in &resolver.chosen {
        for c in &manifest.dependencies.conflicts {
            if resolver.chosen.contains_key(c) || installed.get(c).is_some() {
                return Err(ZpmError::Resolution(format!("'{id}' conflicts with '{c}'")));
            }
        }
        for existing in installed.packages.values() {
            if existing.id != *id && existing.conflicts.iter().any(|c| c == id) {
                return Err(ZpmError::Resolution(format!("installed '{}' conflicts with '{id}'", existing.id)));
            }
        }
    }

    let mut items = Vec::new();
    for id in &resolver.order {
        let manifest = resolver.chosen[id].clone();
        items.push(PlanItem {
            reason: if *id == request.id { Reason::Manual } else { Reason::Dependency },
            replaces: installed.get(id).map(|i| i.version.clone()),
            repository: catalog.repository_of(id)?,
            manifest,
        });
    }
    Ok(Plan { items })
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use crate::manifest::testutil::manifest;

    #[derive(Default)]
    pub struct MemCatalog {
        pub manifests: BTreeMap<(String, Version), Manifest>,
    }

    impl MemCatalog {
        pub fn add(&mut self, id: &str, version: &str, deps: &[(&str, &str)]) -> &mut Self {
            let m = manifest(id, version, deps);
            self.manifests.insert((id.to_string(), m.version().clone()), m);
            self
        }
    }

    impl Catalog for MemCatalog {
        fn versions(&self, id: &str) -> Result<Vec<Version>> {
            let v: Vec<_> = self.manifests.keys().filter(|(i, _)| i == id).map(|(_, v)| v.clone()).collect();
            if v.is_empty() {
                Err(ZpmError::NotFound(id.to_string()))
            } else {
                Ok(v)
            }
        }
        fn manifest(&self, id: &str, version: &Version) -> Result<Manifest> {
            self.manifests.get(&(id.to_string(), version.clone())).cloned().ok_or_else(|| ZpmError::NotFound(id.to_string()))
        }
        fn repository_of(&self, _id: &str) -> Result<String> {
            Ok("test".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::MemCatalog;
    use super::*;
    use crate::database::testutil::record;

    fn env() -> (Version, &'static str) {
        (Version::new(0, 1, 0), "x86_64")
    }

    fn run(cat: &MemCatalog, id: &str, db: &Database, upgrade: bool) -> Result<Plan> {
        let (os, arch) = env();
        let env = Environment { os_version: &os, architecture: arch };
        resolve(&Request { id: id.into(), req: VersionReq::STAR, upgrade }, &env, db, cat)
    }

    fn ids(plan: &Plan) -> Vec<String> {
        plan.items.iter().map(|i| format!("{} {}", i.manifest.id(), i.manifest.version())).collect()
    }

    #[test]
    fn package_without_dependencies() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[]).add("org.t.app", "1.1.0", &[]);
        let plan = run(&cat, "org.t.app", &Database::default(), false).unwrap();
        assert_eq!(ids(&plan), ["org.t.app 1.1.0"]);
        assert_eq!(plan.items[0].reason, Reason::Manual);
    }

    #[test]
    fn multi_level_dependencies_install_dependencies_first() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[("org.t.mid", ">=1.0.0")])
            .add("org.t.mid", "1.0.0", &[("org.t.base", "^1")])
            .add("org.t.base", "1.2.0", &[])
            .add("org.t.base", "2.0.0", &[]);
        let plan = run(&cat, "org.t.app", &Database::default(), false).unwrap();
        assert_eq!(ids(&plan), ["org.t.base 1.2.0", "org.t.mid 1.0.0", "org.t.app 1.0.0"]);
        assert_eq!(plan.items[0].reason, Reason::Dependency);
    }

    #[test]
    fn installed_dependency_is_reused() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[("org.t.lib", ">=1")]).add("org.t.lib", "1.0.0", &[]);
        let mut db = Database::default();
        db.packages.insert("org.t.lib".into(), record("org.t.lib", "1.0.0", &[]));
        assert_eq!(ids(&run(&cat, "org.t.app", &db, false).unwrap()), ["org.t.app 1.0.0"]);
    }

    #[test]
    fn missing_dependency() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[("org.t.ghost", "*")]);
        assert!(matches!(run(&cat, "org.t.app", &Database::default(), false), Err(ZpmError::NotFound(_))));
    }

    #[test]
    fn version_conflict_is_reported() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[("org.t.a", "*"), ("org.t.b", "*")])
            .add("org.t.a", "1.0.0", &[("org.t.lib", ">=2")])
            .add("org.t.b", "1.0.0", &[("org.t.lib", "<2")])
            .add("org.t.lib", "1.0.0", &[])
            .add("org.t.lib", "2.0.0", &[]);
        let err = run(&cat, "org.t.app", &Database::default(), false).unwrap_err();
        assert!(matches!(err, ZpmError::Resolution(ref m) if m.contains("conflict")), "{err}");
    }

    #[test]
    fn dependency_cycle_is_detected() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.a", "1.0.0", &[("org.t.b", "*")]).add("org.t.b", "1.0.0", &[("org.t.a", "*")]);
        let err = run(&cat, "org.t.a", &Database::default(), false).unwrap_err();
        assert!(matches!(err, ZpmError::Resolution(ref m) if m.contains("cycle")), "{err}");
    }

    #[test]
    fn installed_version_that_does_not_satisfy_asks_for_upgrade() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[("org.t.lib", ">=2")]).add("org.t.lib", "2.0.0", &[]);
        let mut db = Database::default();
        db.packages.insert("org.t.lib".into(), record("org.t.lib", "1.0.0", &[]));
        assert!(matches!(run(&cat, "org.t.app", &db, false), Err(ZpmError::Resolution(_))));
    }

    #[test]
    fn duplicate_install_is_refused_and_upgrade_picks_newer() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[]).add("org.t.app", "1.5.0", &[]);
        let mut db = Database::default();
        db.packages.insert("org.t.app".into(), record("org.t.app", "1.0.0", &[]));
        assert!(matches!(run(&cat, "org.t.app", &db, false), Err(ZpmError::AlreadyInstalled(_))));

        let plan = run(&cat, "org.t.app", &db, true).unwrap();
        assert_eq!(ids(&plan), ["org.t.app 1.5.0"]);
        assert_eq!(plan.items[0].replaces, Some(Version::new(1, 0, 0)));

        db.packages.insert("org.t.app".into(), record("org.t.app", "1.5.0", &[]));
        assert!(matches!(run(&cat, "org.t.app", &db, true), Err(ZpmError::Resolution(_))));
    }

    #[test]
    fn incompatible_versions_are_skipped() {
        let mut cat = MemCatalog::default();
        cat.add("org.t.app", "1.0.0", &[]);
        let newer = crate::manifest::Manifest::parse(
            &crate::manifest::testutil::text("org.t.app", "2.0.0", &[]).replace("minimum_os_version = \"0.1.0\"", "minimum_os_version = \"9.0.0\""),
        )
        .unwrap();
        cat.manifests.insert(("org.t.app".into(), Version::new(2, 0, 0)), newer);
        let plan = run(&cat, "org.t.app", &Database::default(), false).unwrap();
        assert_eq!(ids(&plan), ["org.t.app 1.0.0"]);
    }

    #[test]
    fn declared_conflicts_block_installation() {
        let mut cat = MemCatalog::default();
        let text = crate::manifest::testutil::text("org.t.app", "1.0.0", &[]).replace("[dependencies.required]", "[dependencies]\nconflicts = [\"org.t.rival\"]\n[dependencies.required]");
        let m = crate::manifest::Manifest::parse(&text).unwrap();
        cat.manifests.insert(("org.t.app".into(), Version::new(1, 0, 0)), m);
        let mut db = Database::default();
        db.packages.insert("org.t.rival".into(), record("org.t.rival", "1.0.0", &[]));
        assert!(matches!(run(&cat, "org.t.app", &db, false), Err(ZpmError::Resolution(_))));
    }
}