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
