use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use semver::VersionReq;

use crate::config::Context;
use crate::database::{audit, now_unix, Database, InstalledPackage, Reason};
use crate::dependency::{self, Environment, Plan, PlanItem, Request};
use crate::download;
use crate::error::{Result, ZpmError};
use crate::package::{extract_archive, posix, safe_relative, ExtractLimits, PackageSpec};
use crate::repository::RemoteCatalog;
use crate::transaction::Transaction;

/// Resolves what installing `spec` would change. Nothing is modified.
pub fn plan(ctx: &Context, spec: &PackageSpec) -> Result<Plan> {
    let db = Database::load(&ctx.layout.db_file())?;
    let catalog = RemoteCatalog::new(&ctx.repos, &ctx.trust);
    let req = match &spec.version {
        Some(v) => VersionReq::parse(&format!("={v}")).map_err(|e| ZpmError::InvalidVersion(e.to_string()))?,
        None => VersionReq::STAR,
    };
    let env = Environment { os_version: &ctx.config.os_version, architecture: &ctx.config.architecture };
    dependency::resolve(&Request { id: spec.id.clone(), req, upgrade: false }, &env, &db, &catalog)
}

/// Downloads, verifies, stages and commits every item of `plan` as one
/// transaction. Either all packages are installed and recorded, or none are.
pub fn execute(ctx: &Context, plan: &Plan) -> Result<()> {
    let db_path = ctx.layout.db_file();
    let mut db = Database::load(&db_path)?;

    let todo: Vec<&PlanItem> = plan
        .items
        .iter()
        .filter(|i| db.get(i.manifest.id()).map_or(true, |p| p.version != *i.manifest.version()))
        .collect();
    if todo.is_empty() {
        return Ok(());
    }
    check_targets(&db, &todo)?;

    let mut archives = Vec::new();
    for item in &todo {
        let m = &item.manifest;
        println!("Downloading {} {}", m.id(), m.version());
        let repo = ctx.repos.by_name(&item.repository)?;
        archives.push(download::fetch_archive(ctx, repo, m)?);
    }

    let mut tx = Transaction::begin(&ctx.layout)?;
    match stage_and_commit(ctx, &todo, &archives, &mut db, &mut tx) {
        Ok(()) => {
            if let Err(e) = tx.finish() {
                eprintln!("warning: {e}; it will be cleaned up on the next run");
            }
            for item in &todo {
                let m = &item.manifest;
                let action = if item.replaces.is_some() { "upgrade" } else { "install" };
                audit(&ctx.layout, action, &format!("{} {} from {}", m.id(), m.version(), item.repository));
                println!("Installed {} {}", m.id(), m.version());
            }
            Ok(())
        }
        Err(e) => {
            audit(&ctx.layout, "failed", &e.to_string().replace('\n', " "));
            match tx.rollback() {
                Ok(()) => Err(e),
                Err(re) => Err(ZpmError::Transaction(format!(
                    "{e}; rollback also failed ({re}), it will be retried on the next run"
                ))),
            }
        }
    }
}

fn check_targets(db: &Database, todo: &[&PlanItem]) -> Result<()> {
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for item in todo {
        let (id, target) = (item.manifest.id(), item.manifest.installation.target.as_str());
        let clash = db
            .packages
            .values()
            .find(|p| p.target == target && p.id != id)
            .map(|p| p.id.as_str())
            .or_else(|| seen.iter().find(|(_, t)| *t == target).map(|(i, _)| *i));
        if let Some(other) = clash {
            return Err(ZpmError::Transaction(format!("'{id}' and '{other}' would install into the same location '{target}'")));
        }
        seen.push((id, target));
    }
    Ok(())
}

fn stage_and_commit(
    ctx: &Context,
    todo: &[&PlanItem],
    archives: &[PathBuf],
    db: &mut Database,
    tx: &mut Transaction,
) -> Result<()> {
    let limits = ExtractLimits {
        max_files: ctx.config.max_files_per_package,
        max_unpacked_bytes: ctx.config.max_unpacked_bytes,
    };
    let system = ctx.layout.system();
    let mut records = Vec::new();

    for (item, archive) in todo.iter().zip(archives) {
        let m = &item.manifest;
        let id = m.id();
        let staged = tx.staging_dir(id)?;
        let files = extract_archive(archive, &staged, &limits)?;

        let entry = safe_relative(&m.installation.entrypoint)
            .map(|p| posix(&p))
            .ok_or_else(|| ZpmError::Manifest("invalid entrypoint".into()))?;
        if !files.iter().any(|f| f.path == entry) {
            return Err(ZpmError::Manifest(format!("entrypoint '{entry}' is not in the archive")));
        }

        let target = &m.installation.target;
        let dest_root = system.join(target);
        let old = db.get(id);
        let owned: BTreeSet<&str> = old
            .filter(|o| o.target == *target)
            .map(|o| o.files.iter().map(|f| f.path.as_str()).collect())
            .unwrap_or_default();

        for f in &files {
            let dest = dest_root.join(&f.path);
            if fs::symlink_metadata(&dest).is_ok() && !owned.contains(f.path.as_str()) {
                return Err(ZpmError::Transaction(format!(
                    "refusing to overwrite '{}': it exists and is not owned by {id}",
                    dest.display()
                )));
            }
            tx.place(staged.join(&f.path), dest);
        }

        if let Some(o) = old {
            let kept: BTreeSet<&str> = if o.target == *target { files.iter().map(|f| f.path.as_str()).collect() } else { BTreeSet::new() };
            let old_root = system.join(&o.target);
            for f in o.files.iter().filter(|f| !kept.contains(f.path.as_str())) {
                tx.remove(old_root.join(&f.path));
            }
        }
        tx.expect(id, Some(m.version()));

        let reason = match (item.reason, old.map(|o| o.reason)) {
            (Reason::Dependency, Some(Reason::Manual)) => Reason::Manual,
            (r, _) => r,
        };
        records.push(InstalledPackage {
            id: id.to_string(),
            name: m.package.name.clone(),
            version: m.version().clone(),
            category: m.package.category,
            target: target.clone(),
            entrypoint: entry,
            repository: item.repository.clone(),
            reason,
            installed_at: now_unix(),
            requires: m.dependencies.required.clone(),
            conflicts: m.dependencies.conflicts.clone(),
            files,
        });
    }

    tx.commit()?;
    for r in records {
        db.packages.insert(r.id.clone(), r);
    }
    db.save(&ctx.layout.db_file())
}