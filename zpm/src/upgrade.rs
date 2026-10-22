use semver::{Version, VersionReq};

use crate::config::{Context, UpgradePolicy};
use crate::database::Database;
use crate::dependency::{self, Environment, Plan, Request};
use crate::error::{Result, ZpmError};
use crate::install;
use crate::repository::RemoteCatalog;

pub struct Upgrade {
    pub id: String,
    pub from: Version,
    pub to: Version,
    pub plan: Plan,
}

#[derive(Default)]
pub struct UpgradeSet {
    pub upgrades: Vec<Upgrade>,
    /// Packages that have a newer version but cannot move to it, with the reason.
    pub held_back: Vec<(String, String)>,
}

fn policy_requirement(policy: UpgradePolicy, installed: &Version) -> Result<VersionReq> {
    let text = match policy {
        UpgradePolicy::Any => format!(">{installed}"),
        UpgradePolicy::SameMajor => format!(">{installed}, <{}.0.0", installed.major + 1),
        UpgradePolicy::PatchOnly => format!(">{installed}, <{}.{}.0", installed.major, installed.minor + 1),
    };
    VersionReq::parse(&text).map_err(|e| ZpmError::InvalidVersion(e.to_string()))
}

/// Works out which installed packages can be upgraded under the configured
/// policy. Nothing is modified.
pub fn plan(ctx: &Context) -> Result<UpgradeSet> {
    let db = Database::load(&ctx.layout.db_file())?;
    let catalog = RemoteCatalog::new(&ctx.repos, &ctx.trust);
    let env = Environment { os_version: &ctx.config.os_version, architecture: &ctx.config.architecture };
    let mut set = UpgradeSet::default();

    for pkg in db.packages.values() {
        let req = policy_requirement(ctx.config.upgrade_policy, &pkg.version)?;
        let versions = match dependency::Catalog::versions(&catalog, &pkg.id) {
            Ok(v) => v,
            Err(ZpmError::NotFound(_)) => {
                set.held_back.push((pkg.id.clone(), "no longer available in any repository".into()));
                continue;
            }
            Err(e) => return Err(e),
        };
        if !versions.iter().any(|v| req.matches(v)) {
            continue; // up to date under the current policy
        }

        let request = Request { id: pkg.id.clone(), req, upgrade: true };
        let plan = match dependency::resolve(&request, &env, &db, &catalog) {
            Ok(p) => p,
            Err(e @ ZpmError::Resolution(_)) | Err(e @ ZpmError::Incompatible(_)) => {
                set.held_back.push((pkg.id.clone(), e.to_string()));
                continue;
            }
            Err(e) => return Err(e),
        };
        let Some(target) = plan.items.iter().find(|i| i.manifest.id() == pkg.id) else { continue };
        let to = target.manifest.version().clone();

        // Installed packages that depend on this one must still be satisfied.
        let broken: Vec<String> = db
            .dependents(&pkg.id)
            .into_iter()
            .filter(|(_, req)| VersionReq::parse(req).map_or(false, |r| !r.matches(&to)))
            .map(|(p, req)| format!("{} requires {req}", p.id))
            .collect();
        if !broken.is_empty() {
            set.held_back.push((pkg.id.clone(), format!("{to} would break: {}", broken.join(", "))));
            continue;
        }
        set.upgrades.push(Upgrade { id: pkg.id.clone(), from: pkg.version.clone(), to, plan });
    }
    Ok(set)
}

pub fn execute(ctx: &Context, set: &UpgradeSet) -> Result<()> {
    for upgrade in &set.upgrades {
        install::execute(ctx, &upgrade.plan)?;
    }
    Ok(())
}