//! Verifies: TEAM.SYS.PKG-R05. Real files, descriptor-relative admission, atomic durability.
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use teamarness_generated_package_backend::config::{Config, FileStore, StorageFault, WriteStage};
struct Fail(WriteStage);
impl StorageFault for Fail {
    fn before(&self, stage: WriteStage) -> Result<(), std::io::Error> {
        if stage == self.0 {
            Err(std::io::Error::other("injected storage failure"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn missing_defaults_and_durable_reopen_have_private_modes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let store = FileStore::open(&path).unwrap();
    assert!(!store.read().unwrap().desired_enabled);
    let config = Config {
        port: Some(43210),
        ..Config::default()
    };
    store.write(&config).unwrap();
    assert_eq!(
        FileStore::open(&path).unwrap().read().unwrap().port,
        Some(43210)
    );
    assert_eq!(
        fs::metadata(path.join("config"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(path.join("config/state.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn corrupt_future_oversized_and_unsafe_files_are_never_overwritten() {
    for content in [
        b"broken".to_vec(),
        b"{\"configVersion\":2}".to_vec(),
        vec![b' '; 1024 * 1024 + 1],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let store = FileStore::open(&path).unwrap();
        let file = path.join("config/state.json");
        fs::write(&file, &content).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(store.read().is_err());
        assert!(store.write(&Config::default()).is_err());
        assert_eq!(fs::read(file).unwrap(), content);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let store = FileStore::open(&path).unwrap();
    let external = path.join("external");
    fs::write(&external, "unchanged").unwrap();
    symlink(&external, path.join("config/state.json")).unwrap();
    assert!(store.read().is_err());
    assert!(store.write(&Config::default()).is_err());
    assert_eq!(fs::read_to_string(external).unwrap(), "unchanged");
    fs::remove_file(path.join("config/state.json")).unwrap();
    fs::create_dir(path.join("config/state.json")).unwrap();
    assert!(store.read().is_err());
    let alias = path.join("alias");
    symlink(path.join("config"), &alias).unwrap();
    assert!(FileStore::open(&alias).is_err());
}
#[test]
fn io_faults_are_reported_and_uncertain_rename_is_read_back() {
    for stage in [
        WriteStage::Write,
        WriteStage::FileSync,
        WriteStage::Rename,
        WriteStage::DirectorySync,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let store = FileStore::open(&path).unwrap();
        store.write(&Config::default()).unwrap();
        let store = store.with_fault(std::sync::Arc::new(Fail(stage)));
        let next = Config {
            port: Some(1234),
            ..Config::default()
        };
        assert!(store.write(&next).is_err());
        assert_eq!(
            store.read().unwrap().port,
            if stage == WriteStage::DirectorySync {
                Some(1234)
            } else {
                None
            }
        );
    }
}

#[test]
fn exact_config_contract_tool_token_and_file_size_boundaries() {
    use std::collections::BTreeMap;
    use teamarness_generated_package_backend::{
        config::{ContractSelection, ToolSelection},
        tokens::{OsEntropy, issue},
    };
    let mut c = Config::default();
    for _ in 0..32 {
        c.tokens
            .push(issue("client", 1, None, &OsEntropy).unwrap().0);
    }
    c.validate().unwrap();
    c.tokens
        .push(issue("overflow", 1, None, &OsEntropy).unwrap().0);
    assert!(c.validate().is_err());
    c.tokens.pop();
    for n in 0..256 {
        c.contracts.insert(
            format!("dev.test.contract{n}"),
            ContractSelection {
                major: 1,
                provider: "dev.test.provider".into(),
                enabled: true,
                tools: BTreeMap::new(),
            },
        );
    }
    c.validate().unwrap();
    c.contracts.insert(
        "dev.test.overflow".into(),
        c.contracts.values().next().unwrap().clone(),
    );
    assert!(c.validate().is_err());
    c.contracts.remove("dev.test.overflow");
    let contract = c.contracts.values_mut().next().unwrap();
    for n in 0..1024 {
        contract.tools.insert(
            format!("read.item{n}"),
            ToolSelection {
                fingerprint: "a".repeat(64),
            },
        );
    }
    c.validate().unwrap();
    c.contracts.values_mut().next().unwrap().tools.insert(
        "read.extra".into(),
        ToolSelection {
            fingerprint: "a".repeat(64),
        },
    );
    assert!(c.validate().is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let store = FileStore::open(&path).unwrap();
    store.write(&Config::default()).unwrap();
    let file = path.join("config/state.json");
    let mut bytes = std::fs::read(&file).unwrap();
    bytes.resize(1024 * 1024, b' ');
    std::fs::write(&file, &bytes).unwrap();
    assert!(store.read().is_ok());
    bytes.push(b' ');
    std::fs::write(&file, bytes).unwrap();
    assert!(store.read().is_err());
}
#[test]
fn symlinked_subdirectory_hardlinks_fifo_and_wrong_modes_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let other = tempfile::tempdir().unwrap();
    symlink(other.path(), path.join("config")).unwrap();
    assert!(FileStore::open(&path).is_err());
    fs::remove_file(path.join("config")).unwrap();
    let store = FileStore::open(&path).unwrap();
    store.write(&Config::default()).unwrap();
    let file = path.join("config/state.json");
    fs::hard_link(&file, path.join("linked")).unwrap();
    assert!(store.read().is_err());
    fs::remove_file(path.join("linked")).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.read().is_err());
    assert!(store.write(&Config::default()).is_err());
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fs::remove_file(&file).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg("-m")
            .arg("600")
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    assert!(store.read().is_err());
    // The macOS filesystem root is not owned by this non-root test user. No file is created.
    assert_ne!(rustix::process::geteuid().as_raw(), 0);
    assert!(FileStore::open(std::path::Path::new("/")).is_err());
}
#[test]
fn duplicate_selection_keys_are_malformed_and_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let store = FileStore::open(&path).unwrap();
    store.write(&Config::default()).unwrap();
    let selection = r#"{"major":1,"provider":"dev.test.provider","enabled":true,"tools":{}}"#;
    let content = format!(
        r#"{{"configVersion":1,"desiredEnabled":false,"port":null,"tokens":[],"contracts":{{"dev.test.reads":{selection},"dev.test.reads":{selection}}}}}"#
    );
    let file = path.join("config/state.json");
    fs::write(&file, &content).unwrap();
    assert!(store.read().is_err());
    assert!(store.write(&Config::default()).is_err());
    assert_eq!(fs::read_to_string(file).unwrap(), content);
}

#[test]
fn abandoned_temp_is_discarded_without_promoting_or_parsing_its_contents() {
    for contents in [
        b"uncommitted partial JSON".to_vec(),
        vec![0; 1024 * 1024 + 1],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let store = FileStore::open(&path).unwrap();
        let temp = path.join("config/state.tmp");
        fs::write(&temp, contents).unwrap();
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!store.read().unwrap().desired_enabled);
        assert!(temp.exists());
        let next = Config {
            port: Some(43210),
            ..Config::default()
        };
        store.write(&next).unwrap();
        assert!(!temp.exists());
        assert_eq!(store.read().unwrap().port, Some(43210));
        assert_eq!(
            fs::metadata(path.join("config/state.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn unsafe_temp_paths_and_invalid_committed_state_are_preserved() {
    for kind in [
        "symlink",
        "directory",
        "hardlink",
        "mode",
        "fifo",
        "committed-invalid",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let store = FileStore::open(&path).unwrap();
        store.write(&Config::default()).unwrap();
        let committed = path.join("config/state.json");
        let temp = path.join("config/state.tmp");
        match kind {
            "symlink" => symlink(&committed, &temp).unwrap(),
            "directory" => fs::create_dir(&temp).unwrap(),
            "hardlink" => fs::hard_link(&committed, &temp).unwrap(),
            "fifo" => assert!(
                std::process::Command::new("mkfifo")
                    .args(["-m", "600"])
                    .arg(&temp)
                    .status()
                    .unwrap()
                    .success()
            ),
            _ => {
                fs::write(&temp, "uncommitted").unwrap();
                fs::set_permissions(
                    &temp,
                    fs::Permissions::from_mode(if kind == "mode" { 0o644 } else { 0o600 }),
                )
                .unwrap();
            }
        }
        if kind == "committed-invalid" {
            fs::write(&committed, "invalid").unwrap();
        }
        let before = fs::read(&committed).unwrap();
        let temp_before = fs::symlink_metadata(&temp).unwrap();
        assert!(store.write(&Config::default()).is_err(), "{kind}");
        assert_eq!(fs::read(&committed).unwrap(), before);
        use std::os::unix::fs::MetadataExt;
        let temp_after = fs::symlink_metadata(&temp).unwrap();
        assert_eq!(temp_after.ino(), temp_before.ino(), "{kind}");
        assert_eq!(temp_after.mode(), temp_before.mode(), "{kind}");
    }
}

#[test]
fn origin_failed_first_write_child() {
    let Some(path) = std::env::var_os("TMX_ORIGIN_WRITE_TEST") else {
        return;
    };
    let store = FileStore::open(std::path::Path::new(&path)).unwrap();
    let existing = store.read_origin().unwrap();
    let result = store.setup_origin(&teamarness_generated_package_backend::tokens::OsEntropy);
    if let Some(existing) = existing {
        assert_eq!(result.unwrap(), existing);
    } else {
        assert_eq!(
            result,
            Err(teamarness_generated_package_backend::config::StorageError::Io)
        );
    }
}

#[test]
fn origin_real_first_write_failure_then_restored_io_and_reopen_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    FileStore::open(&path).unwrap();
    let output = std::process::Command::new("/bin/bash")
        .args([
            "-c",
            "trap '' XFSZ; ulimit -f 0; exec \"$@\"",
            "origin-write-test",
        ])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "origin_failed_first_write_child", "--nocapture"])
        .env("TMX_ORIGIN_WRITE_TEST", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !path.join("config/origin").exists(),
        "failed first write published an incomplete origin"
    );
    let store = FileStore::open(&path).unwrap();
    let origin = store
        .setup_origin(&teamarness_generated_package_backend::tokens::OsEntropy)
        .unwrap();
    assert_eq!(origin.len(), 64);
    assert_eq!(
        FileStore::open(&path).unwrap().read_origin().unwrap(),
        Some(origin.clone())
    );
    // Established identity needs no file write, even under the same real IO failure.
    let output = std::process::Command::new("/bin/bash")
        .args([
            "-c",
            "trap '' XFSZ; ulimit -f 0; exec \"$@\"",
            "origin-established-test",
        ])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "origin_failed_first_write_child", "--nocapture"])
        .env("TMX_ORIGIN_WRITE_TEST", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(path.join("config/origin")).unwrap(),
        origin
    );
}

struct OriginFail(WriteStage);
impl StorageFault for OriginFail {
    fn before(&self, _: WriteStage) -> std::io::Result<()> {
        Ok(())
    }
    fn before_origin(&self, stage: WriteStage) -> std::io::Result<()> {
        if stage == self.0 {
            Err(std::io::Error::other("origin durability failure"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn origin_durability_faults_retry_without_replacing_published_identity() {
    use teamarness_generated_package_backend::tokens::OsEntropy;
    for stage in [
        WriteStage::Write,
        WriteStage::FileSync,
        WriteStage::Rename,
        WriteStage::DirectorySync,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let store = FileStore::open(&path)
            .unwrap()
            .with_fault(std::sync::Arc::new(OriginFail(stage)));
        assert!(store.setup_origin(&OsEntropy).is_err());
        let published = store.read_origin().unwrap();
        assert_eq!(published.is_some(), stage == WriteStage::DirectorySync);
        let store = FileStore::open(&path).unwrap();
        let origin = store.setup_origin(&OsEntropy).unwrap();
        if let Some(published) = published {
            assert_eq!(published, origin);
        }
        assert_eq!(store.setup_origin(&OsEntropy).unwrap(), origin);
        assert_eq!(
            fs::read_to_string(path.join("config/origin")).unwrap(),
            origin
        );
        assert_eq!(
            fs::metadata(path.join("config/origin"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(store.read().unwrap().tokens.is_empty());
    }
}

#[test]
fn origin_unknown_stages_and_invalid_existing_finals_are_never_reclaimed() {
    use teamarness_generated_package_backend::tokens::{Entropy, EntropyError, OsEntropy};
    struct Fixed;
    impl Entropy for Fixed {
        fn fill(&self, b: &mut [u8]) -> Result<(), EntropyError> {
            b.fill(0xab);
            Ok(())
        }
    }
    for kind in ["regular", "symlink", "directory"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let store = FileStore::open(&path).unwrap();
        let temp = path.join(format!("config/origin-{}.tmp", "ab".repeat(16)));
        let sentinel = path.join("sentinel");
        fs::write(&sentinel, "unrelated").unwrap();
        match kind {
            "symlink" => symlink(&sentinel, &temp).unwrap(),
            "directory" => fs::create_dir(&temp).unwrap(),
            _ => fs::write(&temp, "unknown staged data").unwrap(),
        }
        use std::os::unix::fs::MetadataExt;
        let before = fs::symlink_metadata(&temp).unwrap();
        assert!(store.setup_origin(&Fixed).is_err());
        assert!(store.read_origin().unwrap().is_none());
        assert_eq!(fs::symlink_metadata(&temp).unwrap().ino(), before.ino());
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "unrelated");
        let origin = store.setup_origin(&OsEntropy).unwrap();
        assert_ne!(origin, "ab".repeat(32));
        assert_eq!(fs::symlink_metadata(&temp).unwrap().ino(), before.ino());
    }
    for contents in ["", "partial", &"A".repeat(64), &"a".repeat(65)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let store = FileStore::open(&path).unwrap();
        let final_path = path.join("config/origin");
        fs::write(&final_path, contents).unwrap();
        fs::set_permissions(&final_path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(store.setup_origin(&OsEntropy).is_err());
        assert_eq!(fs::read_to_string(final_path).unwrap(), contents);
    }
}

#[test]
fn concurrent_origin_publication_is_no_replace_and_returns_one_identity() {
    use std::sync::{Arc, Barrier};
    struct Race(Barrier);
    impl StorageFault for Race {
        fn before(&self, _: WriteStage) -> std::io::Result<()> {
            Ok(())
        }
        fn before_origin(&self, s: WriteStage) -> std::io::Result<()> {
            if s == WriteStage::Rename {
                self.0.wait();
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let fault = Arc::new(Race(Barrier::new(2)));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let store = FileStore::open(&path).unwrap().with_fault(fault.clone());
            std::thread::spawn(move || {
                store
                    .setup_origin(&teamarness_generated_package_backend::tokens::OsEntropy)
                    .unwrap()
            })
        })
        .collect();
    let origins: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(origins[0], origins[1]);
    assert_eq!(
        FileStore::open(&path).unwrap().read_origin().unwrap(),
        Some(origins[0].clone())
    );
    assert_eq!(fs::read_dir(path.join("config")).unwrap().count(), 1);
}

#[test]
fn origin_unknown_final_arriving_at_publication_is_preserved_and_refused() {
    struct Collision(std::path::PathBuf);
    impl StorageFault for Collision {
        fn before(&self, _: WriteStage) -> std::io::Result<()> {
            Ok(())
        }
        fn before_origin(&self, stage: WriteStage) -> std::io::Result<()> {
            if stage == WriteStage::Rename {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&self.0)?;
                f.write_all(b"unknown final")?;
                fs::set_permissions(&self.0, fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let final_path = path.join("config/origin");
    let store = FileStore::open(&path)
        .unwrap()
        .with_fault(std::sync::Arc::new(Collision(final_path.clone())));
    assert_eq!(
        store.setup_origin(&teamarness_generated_package_backend::tokens::OsEntropy),
        Err(teamarness_generated_package_backend::config::StorageError::Invalid)
    );
    assert_eq!(fs::read(final_path).unwrap(), b"unknown final");
    assert_eq!(fs::read_dir(path.join("config")).unwrap().count(), 1);
}
