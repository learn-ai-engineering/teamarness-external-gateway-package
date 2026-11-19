use std::fs;
use teamarness_generated_package_backend::{
    managed_install::{Admitted, ManagedInstaller},
    skill_install::SkillInstaller,
};
#[test]
fn skill_is_portable_secret_free_and_uses_one_literal_absolute_command() {
    let home = private_temp();
    let home = home.path().canonicalize().unwrap();
    let selected = home.join("agent's skills with spaces");
    private_dir(&selected).unwrap();
    let command = home.join(".local/bin/tmx");
    let managed = ManagedInstaller::for_home(&home).unwrap();
    let installer = SkillInstaller::new(&managed);
    let admission = Admitted::new("installation-a", "1.0.0", &"a".repeat(64)).unwrap();
    let plan = installer.preview(&selected, admission).unwrap();
    assert!(!selected.join("teamarness").exists());
    installer.apply(&plan).unwrap();
    let skill = fs::read_to_string(selected.join("teamarness/SKILL.md")).unwrap();
    assert!(skill.contains(&format!("'{}' workspaces", command.display())));
    for required in [
        "nextCursor",
        "empty",
        "--major",
        "schema",
        "guide",
        "outcomeUncertain",
        "reconcile",
        "tee",
        "pipefail",
    ] {
        assert!(skill.contains(required), "{required}");
    }
    for forbidden in [
        "eg1.",
        "Bearer",
        ".local/share",
        "baseVersion",
        "expectedRevision",
    ] {
        assert!(!skill.contains(forbidden), "{forbidden}");
    }
    let before = skill.into_bytes();
    fs::write(selected.join("teamarness/SKILL.md"), b"user edit").unwrap();
    assert!(
        installer
            .preview(
                &selected,
                Admitted::new("installation-a", "1.0.0", &"a".repeat(64)).unwrap()
            )
            .is_err()
    );
    assert_ne!(
        before,
        fs::read(selected.join("teamarness/SKILL.md")).unwrap()
    );
}

use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::Arc,
};
use teamarness_generated_package_backend::managed_install::{
    Approval, Boundary, InstallError, InstallFault, Recovery,
};
fn admission() -> Admitted {
    Admitted::new("installation-a", "1.1.0", &"b".repeat(64)).unwrap()
}
fn state(home: &Path) -> PathBuf {
    home.join(".local/share/teamarness/external-gateway/managed")
}
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let t = private_temp();
    let h = t.path().canonicalize().unwrap();
    let selected = h.join("selected");
    private_dir(&selected).unwrap();
    (t, h, selected)
}
fn install(home: &Path, selected: &Path) {
    let m = ManagedInstaller::for_home(home).unwrap();
    let i = SkillInstaller::new(&m);
    let p = i.preview(selected, admission()).unwrap();
    i.apply(&p).unwrap();
}
fn previous_skill(selected: &Path) {
    let d = selected.join("teamarness");
    fs::write(d.join("SKILL.md"), b"previous recognized portable skill").unwrap();
    let receipt = d.join(".teamarness-receipt.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    v["installedDigest"] = Sha256::digest(b"previous recognized portable skill")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .into();
    v["admission"]["version"] = "1.0.0".into();
    fs::write(receipt, serde_json::to_vec(&v).unwrap()).unwrap();
}
#[test]
fn offline_render_is_byte_identical_to_installed_skill_for_the_same_command() {
    use teamarness_generated_package_backend::skill_install::render;
    let (_t, base, _) = setup();
    let home = base.join("it's a home");
    private_dir(&home).unwrap();
    let selected = base.join("agent skills");
    private_dir(&selected).unwrap();
    install(&home, &selected);
    let managed = ManagedInstaller::for_home(&home).unwrap();
    let rendered = render(&managed.command_path()).unwrap();
    assert_eq!(
        rendered.as_bytes(),
        fs::read(selected.join("teamarness/SKILL.md")).unwrap()
    );
    assert!(!rendered.contains("{{"));
    assert!(render(Path::new("relative/tmx")).is_err());
    assert!(render(Path::new("/control\n/tmx")).is_err());
}
#[test]
fn command_with_spaces_and_literal_apostrophes_is_a_single_executable_word() {
    let (_t, base, _) = setup();
    let home = base.join("my home's space");
    private_dir(&home).unwrap();
    let selected = base.join("agent's space");
    private_dir(&selected).unwrap();
    install(&home, &selected);
    let content = fs::read_to_string(selected.join("teamarness/SKILL.md")).unwrap();
    let expected = format!(
        "'{}'\\''s space/.local/bin/tmx'",
        base.join("my home").display()
    );
    assert!(content.contains(&expected), "{expected}");
    fs::create_dir_all(home.join(".local/bin")).unwrap();
    fs::write(
        home.join(".local/bin/tmx"),
        b"#!/bin/sh\nprintf '%s' \"$1\"",
    )
    .unwrap();
    fs::set_permissions(
        home.join(".local/bin/tmx"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let line = content
        .lines()
        .find(|l| l.ends_with(" workspaces"))
        .unwrap();
    let out = std::process::Command::new("/bin/sh")
        .args(["-c", line])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"workspaces");
}
#[test]
fn unrelated_modified_unsafe_and_future_skill_state_is_preserved() {
    for kind in [
        "modified",
        "unrelated-file",
        "unrelated-dir",
        "future",
        "missing-receipt",
        "symlink",
        "hardlink",
        "unsafe-mode",
        "unsafe-parent",
    ] {
        let (_t, h, selected) = setup();
        install(&h, &selected);
        let dest = selected.join("teamarness");
        let skill = dest.join("SKILL.md");
        let receipt = dest.join(".teamarness-receipt.json");
        match kind {
            "modified" => fs::write(&skill, b"Person authored edits").unwrap(),
            "unrelated-file" => fs::write(dest.join("keep.txt"), b"keep").unwrap(),
            "unrelated-dir" => {
                fs::remove_file(&skill).unwrap();
                fs::remove_file(&receipt).unwrap();
            }
            "future" => fs::write(
                &receipt,
                fs::read_to_string(&receipt)
                    .unwrap()
                    .replace("\"formatVersion\":1", "\"formatVersion\":2"),
            )
            .unwrap(),
            "missing-receipt" => fs::remove_file(&receipt).unwrap(),
            "symlink" => {
                fs::rename(&skill, h.join("outside")).unwrap();
                symlink(h.join("outside"), &skill).unwrap();
            }
            "hardlink" => fs::hard_link(&skill, h.join("outside")).unwrap(),
            "unsafe-mode" => {
                fs::set_permissions(&skill, fs::Permissions::from_mode(0o666)).unwrap()
            }
            "unsafe-parent" => {
                fs::set_permissions(&selected, fs::Permissions::from_mode(0o777)).unwrap()
            }
            _ => unreachable!(),
        }
        let before = fs::read(&skill).ok();
        let r = fs::read(&receipt).ok();
        let m = ManagedInstaller::for_home(&h).unwrap();
        assert!(
            SkillInstaller::new(&m)
                .preview(&selected, admission())
                .is_err(),
            "{kind}"
        );
        assert_eq!(fs::read(skill).ok(), before);
        assert_eq!(fs::read(receipt).ok(), r);
    }
}
#[test]
fn replaced_selected_folder_or_destination_requires_new_preview() {
    for existing in [false, true] {
        for parent in [false, true] {
            let (_t, h, selected) = setup();
            if existing {
                install(&h, &selected);
                previous_skill(&selected);
            }
            let m = ManagedInstaller::for_home(&h).unwrap();
            let i = SkillInstaller::new(&m);
            let plan = i.preview(&selected, admission()).unwrap();
            if parent {
                fs::rename(&selected, h.join("previous-folder")).unwrap();
                private_dir(&selected).unwrap();
            } else if existing {
                fs::rename(
                    selected.join("teamarness"),
                    selected.join("previous-teamarness"),
                )
                .unwrap();
                private_dir(selected.join("teamarness")).unwrap();
            } else {
                private_dir(selected.join("teamarness")).unwrap();
            }
            assert!(i.apply(&plan).is_err());
            assert!(!selected.join("teamarness/SKILL.md").exists());
        }
    }
}
struct Fail(Boundary);
impl InstallFault for Fail {
    fn before(&self, b: Boundary) -> std::io::Result<()> {
        if b == self.0 {
            Err(std::io::Error::other(
                "injected secret-sentinel-do-not-echo",
            ))
        } else {
            Ok(())
        }
    }
}
#[test]
fn new_directory_and_recognized_update_recover_each_publication_boundary() {
    for existing in [false, true] {
        for boundary in [
            Boundary::JournalWrite,
            Boundary::JournalSync,
            Boundary::JournalRename,
            Boundary::JournalDirectorySync,
            Boundary::CopyWrite,
            Boundary::CopiedVerify,
            Boundary::FileSync,
            Boundary::OutputRename,
            Boundary::OutputDirectorySync,
            Boundary::ReceiptWrite,
            Boundary::ReceiptSync,
            Boundary::ReceiptRename,
            Boundary::ReceiptDirectorySync,
            Boundary::JournalRemove,
            Boundary::CompletionSync,
        ] {
            let (_t, h, selected) = setup();
            if existing {
                install(&h, &selected);
                previous_skill(&selected);
            }
            let m = ManagedInstaller::for_home(&h).unwrap();
            let p = SkillInstaller::new(&m)
                .preview(&selected, admission())
                .unwrap();
            let faulted = m.with_fault(Arc::new(Fail(boundary)));
            let result = SkillInstaller::new(&faulted).apply(&p);
            assert!(result.is_err(), "existing={existing} boundary={boundary:?}");
            drop(faulted);
            let m = ManagedInstaller::for_home(&h).unwrap();
            let i = SkillInstaller::new(&m);
            let recovery = i.recover().unwrap();
            assert!(matches!(
                recovery,
                Recovery::NothingPending | Recovery::Completed | Recovery::RepairRequired
            ));
            let p = i.preview(&selected, admission()).unwrap();
            i.apply(&p).unwrap();
            let content = fs::read_to_string(selected.join("teamarness/SKILL.md")).unwrap();
            assert!(content.contains("# Teamarness CLI"), "{boundary:?}");
            assert!(!state(&h).join("skill.journal").exists());
        }
    }
}
#[test]
fn command_success_skill_failure_is_truthful_and_never_authorizes_access() {
    let (_t, h, selected) = setup();
    let source = h.join("source");
    write_source(&source, b"native candidate");
    let d = Sha256::digest(b"native candidate")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let admitted = Admitted::new("installation-a", "1.1.0", &d).unwrap();
    let m = ManagedInstaller::for_home(&h).unwrap();
    let command = m.preview(&source, admitted.clone()).unwrap();
    m.apply(&command, Approval::Explicit).unwrap();
    let p = SkillInstaller::new(&m)
        .preview(&selected, admitted)
        .unwrap();
    let faulted = m.with_fault(Arc::new(Fail(Boundary::OutputRename)));
    let error = SkillInstaller::new(&faulted).apply(&p).unwrap_err();
    assert_eq!(error.code(), "external_gateway.output_storage_failed");
    assert_eq!(
        fs::read(h.join(".local/bin/tmx")).unwrap(),
        b"native candidate"
    );
    assert!(!selected.join("teamarness").exists());
    assert!(
        teamarness_generated_package_backend::client_store::ClientStore::for_home(&h)
            .unwrap()
            .connections()
            .unwrap()
            .is_empty()
    );
    let snapshot = fs::read(state(&h).join("skill.journal")).unwrap();
    assert!(
        !String::from_utf8(snapshot)
            .unwrap()
            .contains("secret-sentinel")
    );
    assert!(!format!("{error:?} {}", error.code()).contains("secret-sentinel"));
}
#[test]
fn exact_selected_output_only_no_folder_registry_or_search() {
    let (_t, h, selected) = setup();
    let other = h.join("other-harness");
    private_dir(&other).unwrap();
    fs::write(other.join("SKILL.md"), b"unrelated secret-sentinel").unwrap();
    install(&h, &selected);
    let before = fs::read(selected.join("teamarness/SKILL.md")).unwrap();
    let m = ManagedInstaller::for_home(&h).unwrap();
    let i = SkillInstaller::new(&m);
    let p = i.preview(&selected, admission()).unwrap();
    assert!(p.unchanged());
    i.apply(&p).unwrap();
    assert_eq!(
        fs::read(selected.join("teamarness/SKILL.md")).unwrap(),
        before
    );
    assert_eq!(
        fs::read(other.join("SKILL.md")).unwrap(),
        b"unrelated secret-sentinel"
    );
    for e in fs::read_dir(state(&h)).unwrap() {
        let e = e.unwrap();
        let bytes = fs::read(e.path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("selected"));
        assert!(!String::from_utf8_lossy(&bytes).contains("secret-sentinel"));
    }
}
#[test]
fn unknown_journal_version_is_untouched_and_never_follows_its_destination() {
    let (_t, h, selected) = setup();
    install(&h, &selected);
    let journal = state(&h).join("skill.journal");
    fs::write(
        &journal,
        b"{\"formatVersion\":99,\"output\":\"/unrelated\"}",
    )
    .unwrap();
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o600)).unwrap();
    let m = ManagedInstaller::for_home(&h).unwrap();
    let i = SkillInstaller::new(&m);
    assert_eq!(i.recover(), Err(InstallError::InvalidState));
    assert!(i.preview(&selected, admission()).is_err());
    assert_eq!(
        fs::read(journal).unwrap(),
        b"{\"formatVersion\":99,\"output\":\"/unrelated\"}"
    );
}

#[test]
fn non_utf8_and_noncanonical_selections_are_refused_without_writes() {
    use std::os::unix::ffi::OsStringExt;
    let (_t, h, selected) = setup();
    let m = ManagedInstaller::for_home(&h).unwrap();
    let i = SkillInstaller::new(&m);
    let invalid = selected.join(std::ffi::OsString::from_vec(vec![0xff]));
    assert!(i.preview(&invalid, admission()).is_err());
    for path in [
        format!("{}/./selected", h.display()),
        format!("{}/../selected", h.display()),
        format!("{}//selected", h.display()),
    ] {
        assert!(i.preview(Path::new(&path), admission()).is_err(), "{path}");
    }
    assert!(!h.join(".local").exists());
    assert!(!selected.join("teamarness").exists());
}

struct Crash(Boundary);
impl InstallFault for Crash {
    fn before(&self, b: Boundary) -> std::io::Result<()> {
        if b == self.0 {
            std::process::exit(77);
        }
        Ok(())
    }
}
fn boundaries() -> [Boundary; 15] {
    [
        Boundary::JournalWrite,
        Boundary::JournalSync,
        Boundary::JournalRename,
        Boundary::JournalDirectorySync,
        Boundary::CopyWrite,
        Boundary::CopiedVerify,
        Boundary::FileSync,
        Boundary::OutputRename,
        Boundary::OutputDirectorySync,
        Boundary::ReceiptWrite,
        Boundary::ReceiptSync,
        Boundary::ReceiptRename,
        Boundary::ReceiptDirectorySync,
        Boundary::JournalRemove,
        Boundary::CompletionSync,
    ]
}
#[test]
fn skill_crash_child() {
    let Some(root) = std::env::var_os("SKILL_CRASH_FIXTURE_HOME") else {
        return;
    };
    let home = PathBuf::from(root);
    let m = ManagedInstaller::for_home(&home).unwrap();
    let p = SkillInstaller::new(&m)
        .preview(&home.join("selected"), admission())
        .unwrap();
    let n = std::env::var("SKILL_CRASH_BOUNDARY")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let faulted = m.with_fault(Arc::new(Crash(boundaries()[n])));
    SkillInstaller::new(&faulted).apply(&p).unwrap();
    panic!("crash boundary not reached");
}
#[test]
fn actual_process_death_during_skill_update_is_recoverable_without_adopting_temps() {
    for existing in [false, true] {
        for (n, boundary) in boundaries().into_iter().enumerate() {
            let (_t, h, selected) = setup();
            if existing {
                install(&h, &selected);
                previous_skill(&selected);
            }
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "skill_crash_child"])
                .env("SKILL_CRASH_FIXTURE_HOME", &h)
                .env("SKILL_CRASH_BOUNDARY", n.to_string())
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(77),
                "{existing}/{boundary:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let m = ManagedInstaller::for_home(&h).unwrap();
            let i = SkillInstaller::new(&m);
            let recovered = i.recover().unwrap();
            let mut orphans = vec![];
            if !existing
                && recovered == Recovery::NothingPending
                && !selected.join("teamarness").exists()
            {
                for entry in fs::read_dir(&selected).unwrap() {
                    let entry = entry.unwrap();
                    if entry
                        .file_name()
                        .to_str()
                        .unwrap()
                        .starts_with(".teamarness-")
                    {
                        fs::write(
                            entry.path().join("unrelated-orphan-marker"),
                            b"must not adopt or reclaim",
                        )
                        .unwrap();
                        orphans.push(entry.path());
                    }
                }
                assert!(
                    !orphans.is_empty(),
                    "pre-journal crash retains an unrecognized orphan"
                );
            }
            let p = i.preview(&selected, admission()).unwrap();
            i.apply(&p).unwrap();
            for orphan in orphans {
                assert_eq!(
                    fs::read(orphan.join("unrelated-orphan-marker")).unwrap(),
                    b"must not adopt or reclaim"
                );
            }
            assert!(
                fs::read_to_string(selected.join("teamarness/SKILL.md"))
                    .unwrap()
                    .contains("# Teamarness CLI")
            );
        }
    }
}

#[test]
fn command_and_skill_share_one_nonwaiting_installer_owner() {
    use std::sync::{
        Barrier,
        atomic::{AtomicBool, Ordering},
    };
    struct Hold {
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
        once: AtomicBool,
    }
    impl InstallFault for Hold {
        fn before(&self, b: Boundary) -> std::io::Result<()> {
            if b == Boundary::CopyWrite && !self.once.swap(true, Ordering::SeqCst) {
                self.entered.wait();
                self.release.wait();
            }
            Ok(())
        }
    }
    let (_t, h, selected) = setup();
    let source = h.join("source");
    write_source(&source, b"native candidate");
    let d = Sha256::digest(b"native candidate")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let admitted = Admitted::new("installation-a", "1.1.0", &d).unwrap();
    let m = ManagedInstaller::for_home(&h).unwrap();
    let command = m.preview(&source, admitted.clone()).unwrap();
    let skill = SkillInstaller::new(&m)
        .preview(&selected, admitted)
        .unwrap();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let faulted = m.with_fault(Arc::new(Hold {
        entered: entered.clone(),
        release: release.clone(),
        once: AtomicBool::new(false),
    }));
    let thread = std::thread::spawn(move || faulted.apply(&command, Approval::Explicit));
    entered.wait();
    let other = ManagedInstaller::for_home(&h).unwrap();
    // A stale parent snapshot can also refuse before locking after another output created state.
    let skill = SkillInstaller::new(&other)
        .preview(&selected, admission())
        .unwrap_or(skill);
    assert_eq!(
        SkillInstaller::new(&other).apply(&skill),
        Err(InstallError::Busy)
    );
    release.wait();
    thread.join().unwrap().unwrap();
}

#[test]
fn unrelated_reserved_temp_is_not_deleted_by_a_recognized_skill_update() {
    let (_t, h, selected) = setup();
    install(&h, &selected);
    previous_skill(&selected);
    let temp = selected.join("teamarness/.skill.tmp");
    fs::write(&temp, b"Person's unrelated bytes").unwrap();
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).unwrap();
    let m = ManagedInstaller::for_home(&h).unwrap();
    let i = SkillInstaller::new(&m);
    if let Ok(p) = i.preview(&selected, admission()) {
        assert!(i.apply(&p).is_err());
    }
    assert_eq!(fs::read(temp).unwrap(), b"Person's unrelated bytes");
}

#[test]
fn edit_during_receipt_publication_is_preserved_and_never_reported_installed() {
    struct Edit(PathBuf);
    impl InstallFault for Edit {
        fn before(&self, b: Boundary) -> std::io::Result<()> {
            if b == Boundary::ReceiptRename {
                fs::write(&self.0, b"concurrent Person edit")?;
            }
            Ok(())
        }
    }
    let (_t, h, selected) = setup();
    install(&h, &selected);
    previous_skill(&selected);
    let m = ManagedInstaller::for_home(&h).unwrap();
    let p = SkillInstaller::new(&m)
        .preview(&selected, admission())
        .unwrap();
    let faulted = m.with_fault(Arc::new(Edit(selected.join("teamarness/SKILL.md"))));
    assert_eq!(
        SkillInstaller::new(&faulted).apply(&p),
        Err(InstallError::Collision)
    );
    assert_eq!(
        fs::read(selected.join("teamarness/SKILL.md")).unwrap(),
        b"concurrent Person edit"
    );
}
// Explicit owner-only modes keep fixtures independent of the process umask: Ubuntu's
// default 0002 would otherwise create group-writable homes, folders and sources that
// the installers correctly refuse. Tests that need unsafe modes set them explicitly.
fn private_temp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn private_dir(path: impl AsRef<Path>) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}
fn write_source(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
