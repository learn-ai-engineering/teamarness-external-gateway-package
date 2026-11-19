use teamarness_generated_package_backend::client_store::{ClientStore, Connection};
#[test]
fn durable_connection_and_local_summary_never_disclose_credentials() {
    let home = private_home();
    let store = ClientStore::for_home(&home.path().canonicalize().unwrap()).unwrap();
    let c = connection("workspace-a", "Acme");
    store.publish(&c).unwrap();
    assert_eq!(store.resolve("Acme").unwrap().workspace_id, "workspace-a");
    let summary = serde_json::to_string(&store.summaries().unwrap()).unwrap();
    assert_eq!(
        summary,
        r#"[{"id":"workspace-a","name":"Acme","port":43127}]"#
    );
    assert!(!summary.contains("eg1."));
}
pub fn connection(id: &str, name: &str) -> Connection {
    Connection {
        format_version: 1,
        workspace_id: id.into(),
        workspace_name: name.into(),
        port: 43127,
        installation_id: "installation-a".into(),
        continuity_id: "continuity-a".into(),
        origin_nonce: "a".repeat(64),
        token_id: "AAAAAAAAAAAAAAAAAAAAAA".into(),
        generation: 1,
        bearer: format!("eg1.{}.{}", "AAAAAAAAAAAAAAAAAAAAAA", "A".repeat(43)),
        operation_id: "operation-a".into(),
    }
}
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::Arc,
};
use teamarness_generated_package_backend::config::{StorageFault, WriteStage};
fn root(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".local/share/teamarness/external-gateway")
}
fn record(home: &std::path::Path, id: &str) -> std::path::PathBuf {
    root(home).join(ClientStore::filename(id))
}
#[test]
fn filename_identity_duplicate_origin_and_future_invalid_state_are_preserved() {
    for kind in ["filename", "origin", "future", "malformed", "duplicate-key"] {
        let home = private_home();
        let path = home.path().canonicalize().unwrap();
        let store = ClientStore::for_home(&path).unwrap();
        let c = connection("workspace-a", "Acme");
        store.publish(&c).unwrap();
        let file = record(&path, "workspace-a");
        match kind {
            "filename" => fs::rename(&file, record(&path, "workspace-b")).unwrap(),
            "future" => fs::write(
                &file,
                fs::read_to_string(&file)
                    .unwrap()
                    .replace("\"formatVersion\":1", "\"formatVersion\":2"),
            )
            .unwrap(),
            "malformed" => fs::write(&file, "broken").unwrap(),
            "duplicate-key" => fs::write(
                &file,
                fs::read_to_string(&file).unwrap().replace(
                    "\"formatVersion\":1",
                    "\"formatVersion\":1,\"formatVersion\":1",
                ),
            )
            .unwrap(),
            _ => {}
        }
        let actual = if kind == "filename" {
            record(&path, "workspace-b")
        } else {
            file
        };
        let before = fs::read(&actual).unwrap();
        let mut next = c.clone();
        if kind == "origin" {
            next.continuity_id = "other".into();
        } else {
            next.workspace_name = "New".into();
        }
        assert!(store.publish(&next).is_err(), "{kind}");
        assert_eq!(fs::read(actual).unwrap(), before);
    }
}
#[test]
fn record_and_count_boundaries_are_exact() {
    let home = private_home();
    let path = home.path().canonicalize().unwrap();
    let store = ClientStore::for_home(&path).unwrap();
    for n in 0..128 {
        store
            .publish(&connection(&format!("workspace-{n}"), "Name"))
            .unwrap();
    }
    assert_eq!(store.summaries().unwrap().len(), 128);
    assert!(store.publish(&connection("workspace-128", "Name")).is_err());
    let file = record(&path, "workspace-0");
    let mut bytes = fs::read(&file).unwrap();
    bytes.resize(16384, b' ');
    fs::write(&file, &bytes).unwrap();
    assert!(store.connections().is_ok());
    bytes.push(b' ');
    fs::write(&file, bytes).unwrap();
    assert!(store.connections().is_err());
    fs::remove_file(file).unwrap();
    let extra = record(&path, "workspace-128");
    fs::write(
        &extra,
        serde_json::to_vec(&connection("workspace-128", "Name")).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&extra, fs::Permissions::from_mode(0o600)).unwrap();
    // Restore record zero independently to create 129 physical valid records.
    fs::write(
        record(&path, "workspace-0"),
        serde_json::to_vec(&connection("workspace-0", "Name")).unwrap(),
    )
    .unwrap();
    fs::set_permissions(
        record(&path, "workspace-0"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(store.connections().is_err());
}
#[test]
fn symlink_hardlink_modes_ownership_and_unsafe_traversal_fail_closed() {
    for kind in [
        "symlink",
        "hardlink",
        "mode",
        "directory",
        "temp-symlink",
        "lock-symlink",
        "private-mode",
    ] {
        let home = private_home();
        let path = home.path().canonicalize().unwrap();
        let store = ClientStore::for_home(&path).unwrap();
        let c = connection("workspace-a", "Acme");
        store.publish(&c).unwrap();
        let file = record(&path, "workspace-a");
        let before = fs::read(&file).unwrap();
        match kind {
            "symlink" => {
                fs::rename(&file, path.join("outside")).unwrap();
                symlink(path.join("outside"), &file).unwrap();
            }
            "hardlink" => fs::hard_link(&file, path.join("outside")).unwrap(),
            "mode" => fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap(),
            "private-mode" => {
                fs::set_permissions(root(&path), fs::Permissions::from_mode(0o755)).unwrap()
            }
            "directory" => {
                fs::remove_file(&file).unwrap();
                fs::create_dir(&file).unwrap();
            }
            "temp-symlink" => symlink(&file, root(&path).join("record.tmp")).unwrap(),
            "lock-symlink" => {
                fs::remove_file(root(&path).join("store.lock")).unwrap();
                symlink(&file, root(&path).join("store.lock")).unwrap();
            }
            _ => {}
        }
        assert!(store.publish(&c).is_err(), "{kind}");
        if kind != "directory" {
            assert_eq!(fs::read(&file).unwrap(), before);
        }
    }
    let home = private_home();
    let path = home.path().canonicalize().unwrap();
    let other = private_home();
    symlink(other.path(), path.join(".local")).unwrap();
    assert!(ClientStore::for_home(&path).unwrap().summaries().is_err());
    assert!(ClientStore::for_home(&path.join("..")).is_err());
    assert!(ClientStore::for_home(std::path::Path::new("/usr")).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(ClientStore::for_home(&path).is_err());
}
#[test]
fn existing_intermediate_modes_and_unrelated_files_are_unchanged() {
    let home = private_home();
    let path = home.path().canonicalize().unwrap();
    fs::create_dir(path.join(".local")).unwrap();
    fs::set_permissions(path.join(".local"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(path.join(".local/unrelated"), "keep").unwrap();
    let store = ClientStore::for_home(&path).unwrap();
    store.publish(&connection("workspace-a", "Acme")).unwrap();
    assert_eq!(
        fs::metadata(path.join(".local"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        fs::read_to_string(path.join(".local/unrelated")).unwrap(),
        "keep"
    );
    assert_eq!(
        fs::metadata(root(&path)).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(record(&path, "workspace-a"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
struct Fail(WriteStage);
impl StorageFault for Fail {
    fn before(&self, stage: WriteStage) -> std::io::Result<()> {
        if stage == self.0 {
            Err(std::io::Error::other("injected"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn durability_faults_never_acknowledge_or_change_unrelated_records() {
    for stage in [
        WriteStage::Write,
        WriteStage::FileSync,
        WriteStage::Rename,
        WriteStage::DirectorySync,
    ] {
        let home = private_home();
        let path = home.path().canonicalize().unwrap();
        let store = ClientStore::for_home(&path).unwrap();
        let c = connection("workspace-a", "Acme");
        store.publish(&c).unwrap();
        store.publish(&connection("workspace-b", "Other")).unwrap();
        let unrelated = fs::read(record(&path, "workspace-b")).unwrap();
        let faulted = store.with_fault(Arc::new(Fail(stage)));
        let mut next = c.clone();
        next.port = 54321;
        assert!(faulted.publish(&next).is_err());
        let fresh = ClientStore::for_home(&path).unwrap();
        assert_eq!(
            fresh.resolve("workspace-a").unwrap().port,
            if stage == WriteStage::DirectorySync {
                54321
            } else {
                43127
            }
        );
        assert_eq!(fs::read(record(&path, "workspace-b")).unwrap(), unrelated);
    }
}
#[test]
fn concurrent_name_publication_cannot_resurrect_rotated_credentials() {
    let home = private_home();
    let path = home.path().canonicalize().unwrap();
    let store = ClientStore::for_home(&path).unwrap();
    let c = connection("workspace-a", "Old");
    store.publish(&c).unwrap();
    let mut rotated = c.clone();
    rotated.generation = 2;
    rotated.operation_id = "rotation".into();
    store.publish(&rotated).unwrap();
    assert!(store.refresh_name(&c, "Stale").is_err());
    assert_eq!(store.resolve("workspace-a").unwrap().generation, 2);
    let a = path.clone();
    let b = path.clone();
    let c1 = rotated.clone();
    let c2 = rotated.clone();
    let t1 = std::thread::spawn(move || {
        ClientStore::for_home(&a)
            .unwrap()
            .refresh_name(&c1, "First")
    });
    let t2 = std::thread::spawn(move || {
        ClientStore::for_home(&b)
            .unwrap()
            .refresh_name(&c2, "Second")
    });
    let results = [t1.join().unwrap(), t2.join().unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(store.resolve("workspace-a").unwrap().generation, 2);
}
#[test]
fn lock_child() {
    let Some(root) = std::env::var_os("STORE_LOCK_FIXTURE_ROOT") else {
        return;
    };
    let f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(std::path::PathBuf::from(root).join("store.lock"))
        .unwrap();
    rustix::fs::flock(&f, rustix::fs::FlockOperation::LockExclusive).unwrap();
    println!("LOCK_READY");
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}
#[test]
fn lock_contention_and_process_death_release_the_os_lock() {
    use std::io::{BufRead, BufReader};
    let home = private_home();
    let path = home.path().canonicalize().unwrap();
    let store = ClientStore::for_home(&path).unwrap();
    let c = connection("workspace-a", "Acme");
    store.publish(&c).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_child", "--nocapture"])
        .env("STORE_LOCK_FIXTURE_ROOT", root(&path))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line.contains("LOCK_READY") {
            break;
        }
    }
    assert!(store.publish(&c).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    store.publish(&c).unwrap();
}
#[test]
fn abandoned_temporary_bytes_are_never_promoted_or_reclaimed_by_reads() {
    let home = private_home();
    let path = home.path().canonicalize().unwrap();
    let store = ClientStore::for_home(&path).unwrap();
    let c = connection("workspace-a", "Acme");
    store.publish(&c).unwrap();
    let temporary = root(&path).join("record.tmp");
    fs::write(&temporary, b"uncommitted credential proposal").unwrap();
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(store.resolve("workspace-a").unwrap().port, 43127);
    assert!(temporary.exists());
    store.refresh_name(&c, "Renamed").unwrap();
    assert!(!temporary.exists());
    assert_eq!(
        store.resolve("workspace-a").unwrap().workspace_name,
        "Renamed"
    );
}
/// An owner-only injected home, independent of the process umask (Ubuntu's default
/// 0002 would otherwise create a group-writable home the store refuses).
fn private_home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
