//! End-to-end tests: the real `zpm` binary against a small in-process HTTP
//! store, installing into a temporary root. Nothing outside the temp
//! directory is touched.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use ed25519_dalek::{Signer, SigningKey};
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};

// ---------- fake package store ----------

#[derive(Clone)]
struct Entry {
    manifest: String,
    signature: String,
    key_id: String,
    archive: Vec<u8>,
}

type Store = Arc<Mutex<BTreeMap<(String, String), Entry>>>;

fn serve(store: Store) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let store = store.clone();
            thread::spawn(move || handle(stream, &store));
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn handle(mut stream: TcpStream, store: &Store) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).ok();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
        }
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap().to_string();
    let (status, ctype, body) = route(&path, store);
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

fn json(v: serde_json::Value) -> (&'static str, &'static str, Vec<u8>) {
    ("200 OK", "application/json", serde_json::to_vec(&v).unwrap())
}

fn route(path: &str, store: &Store) -> (&'static str, &'static str, Vec<u8>) {
    let store = store.lock().unwrap();
    let rest = path.strip_prefix("/api/v1/").unwrap_or("");
    let seg: Vec<&str> = rest.split('/').collect();
    match seg.as_slice() {
        ["repository"] => json(serde_json::json!({"name": "dev", "api_version": 1, "package_count": store.len()})),
        ["packages"] => {
            let items: Vec<_> = store
                .keys()
                .map(|(id, v)| serde_json::json!({"id": id, "name": id, "description": "test", "category": "application", "latest_version": v}))
                .collect();
            json(serde_json::json!({"items": items, "total": items.len(), "page": 1, "per_page": 100}))
        }
        ["packages", id, "versions"] => {
            let versions: Vec<_> = store.keys().filter(|(i, _)| i == id).map(|(_, v)| v.clone()).collect();
            if versions.is_empty() {
                ("404 Not Found", "application/json", b"{}".to_vec())
            } else {
                json(serde_json::json!({ "versions": versions }))
            }
        }
        ["packages", id, "versions", v] => match store.get(&(id.to_string(), v.to_string())) {
            Some(e) => json(serde_json::json!({"manifest_toml": e.manifest, "signature": e.signature, "key_id": e.key_id})),
            None => ("404 Not Found", "application/json", b"{}".to_vec()),
        },
        ["packages", id, "versions", v, "download"] => match store.get(&(id.to_string(), v.to_string())) {
            Some(e) => ("200 OK", "application/octet-stream", e.archive.clone()),
            None => ("404 Not Found", "application/json", b"{}".to_vec()),
        },
        _ => ("404 Not Found", "application/json", b"{}".to_vec()),
    }
}

// ---------- fixtures ----------

fn archive(files: &[(&str, &str)]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, body) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, name, body.as_bytes()).unwrap();
    }
    let tar_bytes = b.into_inner().unwrap();
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
}

fn manifest(id: &str, version: &str, deps: &[(&str, &str)], archive: &[u8]) -> String {
    let deps: String = deps.iter().map(|(i, r)| format!("\"{i}\" = \"{r}\"\n")).collect();
    format!(
        r#"schema_version = 1

[package]
id = "{id}"
name = "{id}"
version = "{version}"
description = "End-to-end test package."
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
checksum = "{}"

[installation]
target = "applications/{id}"
entrypoint = "bin/run"
"#,
        hex::encode(Sha256::digest(archive))
    )
}

fn publish(store: &Store, key: &SigningKey, key_id: &str, id: &str, version: &str, deps: &[(&str, &str)], files: &[(&str, &str)]) {
    let archive = archive(files);
    let manifest = manifest(id, version, deps, &archive);
    let signature = hex::encode(key.sign(manifest.as_bytes()).to_bytes());
    store.lock().unwrap().insert(
        (id.to_string(), version.to_string()),
        Entry { manifest, signature, key_id: key_id.to_string(), archive },
    );
}

struct Env {
    _dir: tempfile::TempDir,
    config: PathBuf,
    root: PathBuf,
    store: Store,
    key: SigningKey,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store: Store = Arc::default();
        let url = serve(store.clone());
        let key = SigningKey::from_bytes(&[42; 32]);
        let root = dir.path().join("root");
        let config = dir.path().join("zpm.toml");
        fs::write(
            &config,
            format!(
                "root = {:?}\n\n[[repositories]]\nname = \"dev\"\nurl = \"{url}\"\n\n[trusted_keys]\nofficial = \"{}\"\n",
                root,
                hex::encode(key.verifying_key().to_bytes())
            ),
        )
        .unwrap();
        Env { _dir: dir, config, root, store, key }
    }

    fn publish(&self, id: &str, version: &str, deps: &[(&str, &str)], files: &[(&str, &str)]) {
        publish(&self.store, &self.key, "official", id, version, deps, files);
    }

    fn zpm(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_zpm"))
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn app_dir(&self, id: &str) -> PathBuf {
        self.root.join("system/applications").join(id)
    }
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}
fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn files_under(dir: &Path) -> usize {
    fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

// ---------- tests ----------

#[test]
fn publish_search_install_verify_upgrade_remove() {
    let env = Env::new();
    env.publish("org.test.hello", "1.0.0", &[], &[("bin/run", "v1"), ("share/old.txt", "old")]);

    let out = env.zpm(&["search", "hello"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("org.test.hello"));

    let out = env.zpm(&["info", "org.test.hello"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("1.0.0"));

    let out = env.zpm(&["install", "org.test.hello", "-y"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let dir = env.app_dir("org.test.hello");
    assert_eq!(fs::read_to_string(dir.join("bin/run")).unwrap(), "v1");

    let out = env.zpm(&["list"]);
    assert!(stdout(&out).contains("org.test.hello") && stdout(&out).contains("1.0.0"));
    assert_eq!(code(&env.zpm(&["verify", "org.test.hello"])), 0);

    // Installing again is refused.
    assert_eq!(code(&env.zpm(&["install", "org.test.hello", "-y"])), 1);

    // A file ZPM does not own, dropped into the package directory.
    fs::write(dir.join("notes.txt"), "mine").unwrap();

    // Publish 1.1.0: bin/run changes, share/old.txt is dropped, a new file appears.
    env.publish("org.test.hello", "1.1.0", &[], &[("bin/run", "v2"), ("share/new.txt", "new")]);
    assert_eq!(code(&env.zpm(&["update"])), 0);
    let out = env.zpm(&["list", "--available"]);
    assert!(stdout(&out).contains("installed 1.0.0"), "{}", stdout(&out));

    let out = env.zpm(&["upgrade", "-y"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(fs::read_to_string(dir.join("bin/run")).unwrap(), "v2");
    assert!(!dir.join("share/old.txt").exists());
    assert!(dir.join("share/new.txt").exists());
    assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "mine");
    assert!(stdout(&env.zpm(&["list"])).contains("1.1.0"));
    assert_eq!(code(&env.zpm(&["upgrade", "-y"])), 0);
    assert!(stdout(&env.zpm(&["upgrade", "-y"])).contains("up to date"));

    // Tampering with an installed file is detected.
    fs::write(dir.join("bin/run"), "tampered").unwrap();
    assert_eq!(code(&env.zpm(&["verify", "org.test.hello"])), 4);
    fs::write(dir.join("bin/run"), "v2").unwrap();

    // Removal deletes owned files only.
    let out = env.zpm(&["remove", "org.test.hello", "-y"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!dir.join("bin").exists());
    assert!(!dir.join("share").exists());
    assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "mine");
    assert!(stdout(&env.zpm(&["list"])).contains("No packages installed"));

    // Nothing is left half-done.
    assert!(!env.root.join("system/database/transaction.json").exists());
    assert_eq!(files_under(&env.root.join("system/cache/staging")), 0);
    let audit = fs::read_to_string(env.root.join("system/database/audit.log")).unwrap();
    assert!(audit.contains("install") && audit.contains("upgrade") && audit.contains("remove"));
}

#[test]
fn dependencies_are_installed_and_protected() {
    let env = Env::new();
    env.publish("org.test.lib", "1.2.0", &[], &[("bin/run", "lib")]);
    env.publish("org.test.app", "1.0.0", &[("org.test.lib", ">=1.0.0")], &[("bin/run", "app")]);

    let out = env.zpm(&["install", "org.test.app", "-y"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(env.app_dir("org.test.lib").join("bin/run").exists());
    assert!(stdout(&env.zpm(&["list"])).contains("(dependency)"));

    // The library is still needed, so removing it is refused.
    let out = env.zpm(&["remove", "org.test.lib", "-y"]);
    assert_eq!(code(&out), 5, "{}", stderr(&out));
    assert!(env.app_dir("org.test.lib").join("bin/run").exists());

    assert_eq!(code(&env.zpm(&["remove", "org.test.app", "-y"])), 0);
    assert_eq!(code(&env.zpm(&["remove", "org.test.lib", "-y"])), 0);
}

#[test]
fn missing_dependency_installs_nothing() {
    let env = Env::new();
    env.publish("org.test.app", "1.0.0", &[("org.test.ghost", "*")], &[("bin/run", "app")]);
    let out = env.zpm(&["install", "org.test.app", "-y"]);
    assert_eq!(code(&out), 6, "{}", stderr(&out));
    assert!(!env.app_dir("org.test.app").exists());
}

#[test]
fn package_signed_by_an_untrusted_key_is_rejected() {
    let env = Env::new();
    let stranger = SigningKey::from_bytes(&[1; 32]);
    publish(&env.store, &stranger, "official", "org.test.evil", "1.0.0", &[], &[("bin/run", "x")]);
    let out = env.zpm(&["install", "org.test.evil", "-y"]);
    assert_eq!(code(&out), 4, "{}", stderr(&out));
    assert!(stderr(&out).contains("signature"));
    assert!(!env.app_dir("org.test.evil").exists());

    publish(&env.store, &stranger, "unknown-key", "org.test.evil2", "1.0.0", &[], &[("bin/run", "x")]);
    assert_eq!(code(&env.zpm(&["install", "org.test.evil2", "-y"])), 4);
}

#[test]
fn corrupted_archive_is_rejected_and_leaves_nothing_behind() {
    let env = Env::new();
    env.publish("org.test.app", "1.0.0", &[], &[("bin/run", "good")]);
    // Swap the served bytes after the manifest (and its checksum) were signed.
    env.store.lock().unwrap().get_mut(&("org.test.app".into(), "1.0.0".into())).unwrap().archive = archive(&[("bin/run", "evil")]);

    let out = env.zpm(&["install", "org.test.app", "-y"]);
    assert_eq!(code(&out), 4, "{}", stderr(&out));
    assert!(stderr(&out).contains("checksum"));
    assert!(!env.app_dir("org.test.app").exists());
    let archives = env.root.join("system/cache/archives");
    assert_eq!(files_under(&archives), 0, "no partial or bad archive may stay in the cache");
}

#[test]
fn archive_with_unsafe_paths_is_rejected() {
    let env = Env::new();
    // Build the hostile archive by hand: the tar crate refuses to write `..` itself.
    let mut b = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    let name = b"../escape.txt";
    h.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
    h.set_size(4);
    h.set_mode(0o644);
    h.set_cksum();
    b.append(&h, &b"evil"[..]).unwrap();
    let tar_bytes = b.into_inner().unwrap();
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    let archive = gz.finish().unwrap();

    let manifest = manifest("org.test.evil", "1.0.0", &[], &archive);
    let signature = hex::encode(env.key.sign(manifest.as_bytes()).to_bytes());
    env.store.lock().unwrap().insert(
        ("org.test.evil".into(), "1.0.0".into()),
        Entry { manifest, signature, key_id: "official".into(), archive },
    );

    let out = env.zpm(&["install", "org.test.evil", "-y"]);
    assert_eq!(code(&out), 4, "{}", stderr(&out));
    assert!(!env.root.join("system/applications/escape.txt").exists());
    assert!(!env.root.join("system/escape.txt").exists());
    assert!(!env.app_dir("org.test.evil").exists());
}

#[test]
fn existing_files_not_owned_by_the_package_are_never_overwritten() {
    let env = Env::new();
    env.publish("org.test.app", "1.0.0", &[], &[("bin/run", "app")]);
    let dir = env.app_dir("org.test.app");
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::write(dir.join("bin/run"), "someone else's file").unwrap();

    let out = env.zpm(&["install", "org.test.app", "-y"]);
    assert_eq!(code(&out), 7, "{}", stderr(&out));
    assert_eq!(fs::read_to_string(dir.join("bin/run")).unwrap(), "someone else's file");
    assert!(stdout(&env.zpm(&["list"])).contains("No packages installed"));
}

#[test]
fn unknown_package_and_bad_arguments() {
    let env = Env::new();
    assert_eq!(code(&env.zpm(&["install", "org.test.nothing", "-y"])), 6);
    assert_eq!(code(&env.zpm(&["install", "Not A Valid Id", "-y"])), 1);
    assert_eq!(code(&env.zpm(&["remove", "org.test.nothing", "-y"])), 6);
    assert_eq!(code(&env.zpm(&["install", "org.test.x@banana", "-y"])), 1);
}

#[test]
fn repo_and_cache_commands() {
    let env = Env::new();
    assert!(stdout(&env.zpm(&["repo", "list"])).contains("dev"));

    let out = env.zpm(&["repo", "add", "http://example.org/zpm"]);
    assert_eq!(code(&out), 4, "plain http to a non-local host must be refused");
    let out = env.zpm(&["repo", "add", "https://packages.example.org", "--name", "mirror"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&env.zpm(&["repo", "list"])).contains("mirror"));
    assert_eq!(code(&env.zpm(&["repo", "remove", "mirror"])), 0);
    assert_eq!(code(&env.zpm(&["repo", "remove", "mirror"])), 1);

    env.publish("org.test.app", "1.0.0", &[], &[("bin/run", "app")]);
    assert_eq!(code(&env.zpm(&["download", "org.test.app"])), 0);
    assert!(files_under(&env.root.join("system/cache/archives")) > 0);
    assert_eq!(code(&env.zpm(&["cache", "clear"])), 0);
    assert_eq!(files_under(&env.root.join("system/cache/archives")), 0);
}