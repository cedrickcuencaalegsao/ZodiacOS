use crate::config::Context;
use crate::database::{audit, Database, InstalledPackage};
use crate::error::{Result, ZpmError};
use crate::transaction::Transaction;

/// Looks up the package and refuses if something installed depends on it
/// (unless `force`).
pub fn plan(ctx: &Context, id: &str, force: bool) -> Result<InstalledPackage> {
    let db = Database::load(&ctx.layout.db_file())?;
    let pkg = db.get(id).ok_or_else(|| ZpmError::NotInstalled(id.to_string()))?.clone();
    let dependents = db.dependents(id);
    if !force && !dependents.is_empty() {
        return Err(ZpmError::RequiredBy {
            id: id.to_string(),
            dependents: dependents.iter().map(|(p, req)| format!("{} ({req})", p.id)).collect::<Vec<_>>().join(", "),
        });
    }
    Ok(pkg)
}

/// Removes exactly the files recorded for the package. Anything else in its
/// directory, and anything owned by other packages, is left alone.
pub fn execute(ctx: &Context, pkg: &InstalledPackage) -> Result<()> {
    let db_path = ctx.layout.db_file();
    let mut db = Database::load(&db_path)?;
    if db.get(&pkg.id).is_none() {
        return Err(ZpmError::NotInstalled(pkg.id.clone()));
    }

    let root = ctx.layout.system().join(&pkg.target);
    let mut tx = Transaction::begin(&ctx.layout)?;
    for f in &pkg.files {
        tx.remove(root.join(&f.path));
    }
    tx.expect(&pkg.id, None);

    let outcome = tx.commit().and_then(|()| {
        db.packages.remove(&pkg.id);
        db.save(&db_path)
    });
    match outcome {
        Ok(()) => {
            if let Err(e) = tx.finish() {
                eprintln!("warning: {e}; it will be cleaned up on the next run");
            }
            audit(&ctx.layout, "remove", &format!("{} {}", pkg.id, pkg.version));
            println!("Removed {} {}", pkg.id, pkg.version);
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