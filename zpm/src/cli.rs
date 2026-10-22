use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use semver::VersionReq;

use crate::config::{Config, Context, RepoConfig};
use crate::database::{write_atomic, Database};
use crate::dependency::{self, Catalog, Environment, Plan};
use crate::download;
use crate::error::{Result, ZpmError};
use crate::install;
use crate::package::PackageSpec;
use crate::remove;
use crate::repository::{check_url, PackageSummary, RemoteCatalog};
use crate::upgrade;
use crate::verify;

#[derive(Parser)]
#[command(name = "zpm", version, about = "ZPM, the Zodiac Package Manager")]
pub struct Cli {
    /// Path to zpm.toml (default: $ZPM_CONFIG, then ./zpm.toml)
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Installation root; overrides `root` from the config file
    #[arg(long, global = true, value_name = "DIR")]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Search the repositories
    Search {
        query: String,
        #[arg(long, value_parser = ["application", "driver", "plugin"])]
        category: Option<String>,
    },
    /// Show details of a package
    Info { id: String },
    /// List installed packages (or, with --available, those in the cached index)
    List {
        #[arg(long)]
        available: bool,
    },
    /// Install a package, or `<id>@<version>`, with its dependencies
    Install {
        spec: String,
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
    /// Remove an installed package
    Remove {
        id: String,
        #[arg(short, long)]
        yes: bool,
        /// Remove even if other installed packages depend on it
        #[arg(long)]
        force: bool,
    },
    /// Refresh the repository indexes (does not install anything)
    Update,
    /// Upgrade installed packages according to the upgrade policy
    Upgrade {
        #[arg(short, long)]
        yes: bool,
    },
    /// Download and verify a package archive into the cache
    Download { spec: String },
    /// Check an installed package's files against their recorded checksums
    Verify { id: String },
    /// Manage repositories
    Repo {
        #[command(subcommand)]
        cmd: RepoCmd,
    },
    /// Manage the cache
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
}

#[derive(Subcommand)]
enum RepoCmd {
    List,
    Add {
        url: String,
        /// Repository name (default: derived from the host)
        #[arg(long)]
        name: Option<String>,
    },
    Remove { name: String },
}

#[derive(Subcommand)]
enum CacheCmd {
    /// Delete cached archives and indexes
    Clear,
}

pub fn run(cli: Cli) -> Result<()> {
    let Cli { config: config_path, root, command } = cli;
    let mut config = Config::load(config_path.as_deref())?;
    config.override_root(root)?;
    match command {
        Command::Repo { cmd } => repo_command(config, cmd),
        Command::Cache { cmd } => cache_command(&config, cmd),
        other => run_with_context(Context::open(config)?, other),
    }
}

fn run_with_context(ctx: Context, command: Command) -> Result<()> {
    let env = Environment { os_version: &ctx.config.os_version, architecture: &ctx.config.architecture };
    match command {
        Command::Search { query, category } => {
            if ctx.repos.is_empty() {
                return Err(ZpmError::Config("no repositories configured (see `zpm repo add`)".into()));
            }
            let mut found = 0;
            for repo in ctx.repos.iter() {
                let res = repo.search(&query, category.as_deref(), Some(&ctx.config.architecture), 1, 50)?;
                for item in &res.items {
                    print_summary(&repo.name, item);
                    found += 1;
                }
                if res.total as usize > res.items.len() {
                    println!("({} more in '{}'; narrow the search)", res.total as usize - res.items.len(), repo.name);
                }
            }
            if found == 0 {
                println!("No packages found.");
            }
            Ok(())
        }
        Command::Info { id } => {
            let catalog = RemoteCatalog::new(&ctx.repos, &ctx.trust);
            let any = VersionReq::STAR;
            let m = dependency::pick_version(&catalog, &env, &id, &[&any], None)?;
            let mut versions = catalog.versions(&id)?;
            versions.sort();
            let db = Database::load(&ctx.layout.db_file())?;
            println!("{} ({})", m.package.name, m.id());
            println!("  latest installable: {}", m.version());
            println!(
                "  all versions:       {}",
                versions.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
            );
            println!("  category:           {}", m.package.category);
            println!("  publisher:          {}", m.package.publisher);
            println!("  license:            {}", m.package.license);
            println!("  architectures:      {}", m.package.architecture.join(", "));
            println!("  repository:         {}", catalog.repository_of(&id)?);
            println!("  description:        {}", m.package.description);
            if m.dependencies.required.is_empty() {
                println!("  requires:           nothing");
            } else {
                for (dep, req) in &m.dependencies.required {
                    println!("  requires:           {dep} {req}");
                }
            }
            match db.get(&id) {
                Some(p) => println!("  installed:          {}", p.version),
                None => println!("  installed:          no"),
            }
            Ok(())
        }
        Command::List { available } => {
            let db = Database::load(&ctx.layout.db_file())?;
            if available {
                list_available(&ctx, &db)
            } else {
                if db.packages.is_empty() {
                    println!("No packages installed.");
                }
                for p in db.packages.values() {
                    let auto = if p.reason == crate::database::Reason::Dependency { "  (dependency)" } else { "" };
                    println!("{:<36} {:<12} {:<11}{}", p.id, p.version, p.category, auto);
                }
                Ok(())
            }
        }
        Command::Install { spec, yes } => {
            let spec = PackageSpec::parse(&spec)?;
            let plan = install::plan(&ctx, &spec)?;
            print_plan("installed", &plan);
            if !confirm("Proceed?", yes) {
                return Err(ZpmError::Aborted);
            }
            install::execute(&ctx, &plan)
        }
        Command::Remove { id, yes, force } => {
            let pkg = remove::plan(&ctx, &id, force)?;
            println!("The following package will be removed:\n  {} {} ({} files)", pkg.id, pkg.version, pkg.files.len());
            if !confirm("Proceed?", yes) {
                return Err(ZpmError::Aborted);
            }
            remove::execute(&ctx, &pkg)
        }
        Command::Update => update_indexes(&ctx),
        Command::Upgrade { yes } => {
            let set = upgrade::plan(&ctx)?;
            for (id, why) in &set.held_back {
                eprintln!("held back: {id}: {why}");
            }
            if set.upgrades.is_empty() {
                println!("Everything is up to date.");
                return Ok(());
            }
            println!("The following packages will be upgraded:");
            for u in &set.upgrades {
                println!("  {} {} -> {}", u.id, u.from, u.to);
                for item in u.plan.items.iter().filter(|i| i.manifest.id() != u.id && i.replaces.is_none()) {
                    println!("    + new dependency {} {}", item.manifest.id(), item.manifest.version());
                }
            }
            if !confirm("Proceed?", yes) {
                return Err(ZpmError::Aborted);
            }
            upgrade::execute(&ctx, &set)
        }
        Command::Download { spec } => {
            let spec = PackageSpec::parse(&spec)?;
            let catalog = RemoteCatalog::new(&ctx.repos, &ctx.trust);
            let req = match &spec.version {
                Some(v) => VersionReq::parse(&format!("={v}")).map_err(|e| ZpmError::InvalidVersion(e.to_string()))?,
                None => VersionReq::STAR,
            };
            let m = dependency::pick_version(&catalog, &env, &spec.id, &[&req], None)?;
            let repo = ctx.repos.by_name(&catalog.repository_of(&spec.id)?)?;
            let path = download::fetch_archive(&ctx, repo, &m)?;
            println!("Downloaded and verified {} {} -> {}", m.id(), m.version(), path.display());
            Ok(())
        }
        Command::Verify { id } => {
            let db = Database::load(&ctx.layout.db_file())?;
            let pkg = db.get(&id).ok_or_else(|| ZpmError::NotInstalled(id.clone()))?;
            let report = verify::verify_installed(&ctx.layout, pkg)?;
            if report.is_clean() {
                println!("{} {}: {} files verified", pkg.id, pkg.version, report.checked);
                Ok(())
            } else {
                for f in &report.missing {
                    eprintln!("missing:  {f}");
                }
                for f in &report.modified {
                    eprintln!("modified: {f}");
                }
                Err(ZpmError::VerifyFailed(format!(
                    "{} has {} missing and {} modified files",
                    pkg.id,
                    report.missing.len(),
                    report.modified.len()
                )))
            }
        }
        Command::Repo { .. } | Command::Cache { .. } => unreachable!("handled before opening the installation"),
    }
}

fn print_summary(repo: &str, item: &PackageSummary) {
    println!(
        "{:<36} {:<10} {:<11} [{}] {}",
        item.id,
        item.latest_version.as_deref().unwrap_or("-"),
        item.category,
        repo,
        item.description
    );
}

fn print_plan(verb: &str, plan: &Plan) {
    println!("The following packages will be {verb}:");
    for item in &plan.items {
        let m = &item.manifest;
        let note = match &item.replaces {
            Some(old) => format!("upgrade from {old}"),
            None => "new".to_string(),
        };
        println!("  {} {} ({}, {note}) from {}", m.id(), m.version(), m.package.category, item.repository);
    }
}

fn confirm(prompt: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    print!("{prompt} [y/N] ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(n) if n > 0 => matches!(line.trim(), "y" | "Y" | "yes" | "Yes"),
        _ => false,
    }
}

fn index_path(ctx: &Context, repo: &str) -> PathBuf {
    ctx.layout.index_dir().join(format!("{repo}.json"))
}

fn update_indexes(ctx: &Context) -> Result<()> {
    if ctx.repos.is_empty() {
        return Err(ZpmError::Config("no repositories configured (see `zpm repo add`)".into()));
    }
    for repo in ctx.repos.iter() {
        let info = repo.metadata()?;
        if info.api_version != 1 {
            return Err(ZpmError::Repository(format!(
                "'{}' speaks API version {}; this client supports 1",
                repo.name, info.api_version
            )));
        }
        let mut all: Vec<PackageSummary> = Vec::new();
        for page in 1..=1000u32 {
            let res = repo.search("", None, None, page, 100)?;
            if res.items.is_empty() {
                break;
            }
            all.extend(res.items);
            if all.len() as u64 >= res.total {
                break;
            }
        }
        let bytes = serde_json::to_vec_pretty(&all).map_err(|e| ZpmError::Repository(e.to_string()))?;
        write_atomic(&index_path(ctx, &repo.name), &bytes)?;
        println!("{}: {} packages", repo.name, all.len());
    }
    Ok(())
}

fn list_available(ctx: &Context, db: &Database) -> Result<()> {
    let mut shown = 0;
    for repo in ctx.repos.iter() {
        let path = index_path(ctx, &repo.name);
        let bytes = fs::read(&path).map_err(|_| {
            ZpmError::Config(format!("no cached index for '{}'; run `zpm update` first", repo.name))
        })?;
        let items: Vec<PackageSummary> = serde_json::from_slice(&bytes)
            .map_err(|e| ZpmError::Config(format!("cached index for '{}' is corrupt ({e}); run `zpm update`", repo.name)))?;
        for item in &items {
            let status = match db.get(&item.id) {
                Some(p) => format!("installed {}", p.version),
                None => "available".to_string(),
            };
            println!(
                "{:<36} {:<10} {:<11} {}",
                item.id,
                item.latest_version.as_deref().unwrap_or("-"),
                item.category,
                status
            );
            shown += 1;
        }
    }
    if shown == 0 {
        println!("The cached indexes are empty.");
    }
    Ok(())
}

fn repo_command(mut config: Config, cmd: RepoCmd) -> Result<()> {
    match cmd {
        RepoCmd::List => {
            if config.repositories.is_empty() {
                println!("No repositories configured.");
            }
            for r in &config.repositories {
                println!("{:<20} {}", r.name, r.url);
            }
            Ok(())
        }
        RepoCmd::Add { url, name } => {
            let url = check_url(&url)?;
            let name = name.unwrap_or_else(|| derive_name(&url));
            config.repositories.push(RepoConfig { name: name.clone(), url });
            config.validate()?;
            config.save()?;
            println!("Added repository '{name}'.");
            println!("Packages from it will not install until a key you trust signs them: add that key under [trusted_keys] in {}.", config.path.display());
            Ok(())
        }
        RepoCmd::Remove { name } => {
            let before = config.repositories.len();
            config.repositories.retain(|r| r.name != name);
            if config.repositories.len() == before {
                return Err(ZpmError::Config(format!("repository '{name}' is not configured")));
            }
            config.save()?;
            let _ = fs::remove_file(config.layout().index_dir().join(format!("{name}.json")));
            println!("Removed repository '{name}'.");
            Ok(())
        }
    }
}

fn derive_name(url: &str) -> String {
    let host = url.split_once("://").map_or(url, |(_, r)| r).split('/').next().unwrap_or("repo");
    let host = host.split(':').next().unwrap_or("repo");
    let name: String = host
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if name.is_empty() { "repo".to_string() } else { name }
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn cache_command(config: &Config, cmd: CacheCmd) -> Result<()> {
    match cmd {
        CacheCmd::Clear => {
            let layout = config.layout();
            let mut freed = 0;
            for dir in [layout.archives_dir(), layout.index_dir()] {
                freed += dir_size(&dir);
                if dir.exists() {
                    fs::remove_dir_all(&dir).map_err(|e| ZpmError::io(&dir, e))?;
                }
            }
            layout.ensure()?;
            println!("Cache cleared ({freed} bytes freed).");
            Ok(())
        }
    }
}