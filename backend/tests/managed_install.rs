use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use teamarness_generated_package_backend::managed_install::{
    Admitted, Approval, Disposition, ManagedInstaller,
};
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let source = home.join("package-backend");
    fs::write(&source, b"#!/bin/sh\nprintf copied-cli").unwrap();
    (temp, home, source)
}
fn admitted(source: &Path, version: &str, owner: &str) -> Admitted {
    Admitted::new(owner, version, &digest(&fs::read(source).unwrap())).unwrap()
}
#[test]
fn preview_is_read_only_and_initial_publication_requires_explicit_consent() {
    let (_temp, home, source) = fixture();
    let installer = ManagedInstaller::for_home(&home).unwrap();
    let plan = installer
        .preview(&source, admitted(&source, "1.0.0", "installation-a"))
        .unwrap();
    assert_eq!(plan.disposition(), Disposition::Install);
    assert!(!home.join(".local").exists());
    assert!(installer.apply(&plan, Approval::Automatic).is_err());
    installer.apply(&plan, Approval::Explicit).unwrap();
    assert_eq!(
        fs::read(home.join(".local/bin/tmx")).unwrap(),
        fs::read(source).unwrap()
    );
    let next = installer
        .preview(
            &home.join("package-backend"),
            admitted(&home.join("package-backend"), "1.0.0", "installation-a"),
        )
        .unwrap();
    assert_eq!(next.disposition(), Disposition::Unchanged);
    installer.apply(&next, Approval::Automatic).unwrap();
}
#[test]
fn admitted_source_mismatch_is_reload_not_installation() {
    let (_temp, home, source) = fixture();
    let installer = ManagedInstaller::for_home(&home).unwrap();
    let admission = admitted(&source, "1.0.0", "installation-a");
    fs::write(&source, b"replaced registered artifact").unwrap();
    let error = installer.preview(&source, admission).err().unwrap();
    assert_eq!(error.code(), "external_gateway.reload_required");
    assert!(!home.join(".local").exists());
}

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::fs::{PermissionsExt, symlink},
    sync::Arc,
};
use teamarness_generated_package_backend::managed_install::{
    Boundary, InstallError, InstallFault, Recovery,
};
fn state(home: &Path) -> PathBuf {
    home.join(".local/share/teamarness/external-gateway/managed")
}
fn install(home: &Path, source: &Path) {
    let i = ManagedInstaller::for_home(home).unwrap();
    let p = i
        .preview(source, admitted(source, "1.0.0", "installation-a"))
        .unwrap();
    i.apply(&p, Approval::Explicit).unwrap();
}
#[test]
fn versions_owners_and_compatible_majors_control_refresh_not_byte_ownership() {
    for (version, owner, changed, expected) in [
        ("1.0.0", "installation-a", false, Disposition::Unchanged),
        ("1.1.0", "installation-a", true, Disposition::Refresh),
        ("0.9.0", "installation-a", true, Disposition::ExplicitRepair),
        ("1.0.0", "installation-a", true, Disposition::ExplicitRepair),
        (
            "1.2.0",
            "installation-b",
            false,
            Disposition::ExplicitRepair,
        ),
        (
            "1.0.0+rebuild",
            "installation-a",
            true,
            Disposition::ExplicitRepair,
        ),
    ] {
        let (_t, home, source) = fixture();
        install(&home, &source);
        if changed {
            fs::write(&source, b"new admitted executable").unwrap();
        }
        let i = ManagedInstaller::for_home(&home).unwrap();
        let p = i
            .preview(&source, admitted(&source, version, owner))
            .unwrap();
        assert_eq!(p.disposition(), expected);
        if expected == Disposition::ExplicitRepair {
            assert_eq!(
                i.apply(&p, Approval::Automatic),
                Err(InstallError::ConsentRequired)
            );
            i.apply(&p, Approval::Explicit).unwrap();
        } else {
            i.apply(&p, Approval::Automatic).unwrap();
        }
        assert_eq!(
            fs::read(i.command_path()).unwrap(),
            fs::read(source).unwrap()
        );
    }
    for key in ["cliMajor", "storeMajor"] {
        let (_t, home, source) = fixture();
        install(&home, &source);
        let path = state(&home).join("command.receipt");
        let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        v[key] = 2.into();
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            ManagedInstaller::for_home(&home)
                .unwrap()
                .preview(&source, admitted(&source, "2.0.0", "installation-a"))
                .is_err()
        );
        assert_eq!(fs::read(path).unwrap(), before);
    }
}
#[test]
fn collisions_and_unknown_receipts_are_never_overwritten_even_explicitly() {
    for kind in [
        "unrelated",
        "altered",
        "missing",
        "missing-command",
        "future",
        "malformed",
        "duplicate-key",
        "hardlink",
        "symlink",
        "mode",
        "parent-symlink",
        "parent-writable",
    ] {
        let (_t, home, source) = fixture();
        install(&home, &source);
        let command = home.join(".local/bin/tmx");
        let receipt = state(&home).join("command.receipt");
        match kind {
            "unrelated" => {
                fs::remove_file(&receipt).unwrap();
                fs::write(&command, b"unrelated command").unwrap();
            }
            "altered" => fs::write(&command, b"user modified").unwrap(),
            "missing" => fs::remove_file(&receipt).unwrap(),
            "missing-command" => fs::remove_file(&command).unwrap(),
            "future" => fs::write(
                &receipt,
                fs::read_to_string(&receipt)
                    .unwrap()
                    .replace("\"formatVersion\":1", "\"formatVersion\":2"),
            )
            .unwrap(),
            "malformed" => fs::write(&receipt, b"not json").unwrap(),
            "duplicate-key" => fs::write(
                &receipt,
                fs::read_to_string(&receipt).unwrap().replace(
                    "\"formatVersion\":1",
                    "\"formatVersion\":1,\"formatVersion\":1",
                ),
            )
            .unwrap(),
            "hardlink" => fs::hard_link(&command, home.join("other-link")).unwrap(),
            "symlink" => {
                fs::rename(&command, home.join("outside")).unwrap();
                symlink(home.join("outside"), &command).unwrap();
            }
            "mode" => fs::set_permissions(&command, fs::Permissions::from_mode(0o777)).unwrap(),
            "parent-symlink" => {
                fs::rename(home.join(".local/bin"), home.join("other-bin")).unwrap();
                symlink(home.join("other-bin"), home.join(".local/bin")).unwrap();
            }
            "parent-writable" => {
                fs::set_permissions(home.join(".local/bin"), fs::Permissions::from_mode(0o777))
                    .unwrap()
            }
            _ => unreachable!(),
        }
        let before = fs::read(&command).ok();
        let r = fs::read(&receipt).ok();
        let i = ManagedInstaller::for_home(&home).unwrap();
        assert!(i.observe_command().is_err(), "{kind}");
        assert!(
            i.preview(&source, admitted(&source, "1.1.0", "installation-a"))
                .is_err(),
            "{kind}"
        );
        assert_eq!(fs::read(command).ok(), before, "{kind}");
        assert_eq!(fs::read(receipt).ok(), r, "{kind}");
    }
}
#[test]
fn stale_parent_source_and_copy_changes_fail_before_command_publication() {
    for kind in ["parent", "source", "copy"] {
        let (_t, home, source) = fixture();
        install(&home, &source);
        fs::write(&source, b"new executable").unwrap();
        let mut i = ManagedInstaller::for_home(&home).unwrap();
        let p = i
            .preview(&source, admitted(&source, "1.1.0", "installation-a"))
            .unwrap();
        let before = fs::read(i.command_path()).unwrap();
        match kind {
            "parent" => {
                fs::rename(home.join(".local/bin"), home.join("previous-bin")).unwrap();
                fs::create_dir(home.join(".local/bin")).unwrap();
                fs::write(i.command_path(), &before).unwrap();
                fs::set_permissions(i.command_path(), fs::Permissions::from_mode(0o755)).unwrap();
            }
            "source" => fs::write(&source, b"substituted source").unwrap(),
            "copy" => i = i.with_fault(Arc::new(Tamper(home.join(".local/bin/.tmx.tmp")))),
            _ => unreachable!(),
        }
        assert!(i.apply(&p, Approval::Automatic).is_err(), "{kind}");
        assert_eq!(fs::read(i.command_path()).unwrap(), before);
    }
}
struct Tamper(PathBuf);
impl InstallFault for Tamper {
    fn before(&self, b: Boundary) -> std::io::Result<()> {
        if b == Boundary::CopiedVerify {
            fs::write(&self.0, b"tampered copy")?;
        }
        Ok(())
    }
}
pub struct Fail(pub Boundary);
impl InstallFault for Fail {
    fn before(&self, b: Boundary) -> std::io::Result<()> {
        if b == self.0 {
            Err(std::io::Error::other("injected"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn every_durable_boundary_restarts_only_from_exact_old_or_new_bytes() {
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
        let (_t, home, source) = fixture();
        install(&home, &source);
        let old = fs::read(home.join(".local/bin/tmx")).unwrap();
        fs::write(&source, b"new admitted candidate").unwrap();
        let fresh = ManagedInstaller::for_home(&home).unwrap();
        let plan = fresh
            .preview(&source, admitted(&source, "1.1.0", "installation-a"))
            .unwrap();
        let faulted = fresh.with_fault(Arc::new(Fail(boundary)));
        assert!(
            faulted.apply(&plan, Approval::Automatic).is_err(),
            "{boundary:?}"
        );
        drop(faulted);
        let i = ManagedInstaller::for_home(&home).unwrap();
        let actual = fs::read(i.command_path()).unwrap();
        let new = fs::read(&source).unwrap();
        assert!(actual == old || actual == new);
        let journal_before = fs::read(state(&home).join("command.journal")).ok();
        let receipt_before = fs::read(state(&home).join("command.receipt")).unwrap();
        let observation = i.observe_command().unwrap();
        assert_eq!(
            observation.version.as_deref(),
            Some(if actual == new { "1.1.0" } else { "1.0.0" })
        );
        assert_eq!(observation.pending, journal_before.is_some());
        assert_eq!(
            observation.published_pending,
            journal_before.is_some() && actual == new
        );
        assert_eq!(
            fs::read(state(&home).join("command.journal")).ok(),
            journal_before
        );
        assert_eq!(
            fs::read(state(&home).join("command.receipt")).unwrap(),
            receipt_before
        );
        let recovery = i.recover_command().unwrap();
        if actual == new {
            assert!(
                matches!(recovery, Recovery::Completed | Recovery::NothingPending),
                "{boundary:?}: {recovery:?}"
            );
        } else {
            assert!(
                matches!(
                    recovery,
                    Recovery::RepairRequired | Recovery::NothingPending
                ),
                "{boundary:?}"
            );
        }
        let p = i
            .preview(&source, admitted(&source, "1.1.0", "installation-a"))
            .unwrap();
        i.apply(&p, Approval::Explicit).unwrap();
        assert_eq!(fs::read(i.command_path()).unwrap(), new, "{boundary:?}");
        assert!(!state(&home).join("command.journal").exists());
    }
}
#[test]
fn recovery_never_adopts_arbitrary_bytes_or_abandoned_temporary_files() {
    let (_t, home, source) = fixture();
    install(&home, &source);
    fs::write(&source, b"new admitted executable").unwrap();
    let i = ManagedInstaller::for_home(&home).unwrap();
    let p = i
        .preview(&source, admitted(&source, "1.1.0", "installation-a"))
        .unwrap();
    i.with_fault(Arc::new(Fail(Boundary::ReceiptWrite)))
        .apply(&p, Approval::Automatic)
        .unwrap_err();
    let command = home.join(".local/bin/tmx");
    fs::write(&command, b"arbitrary post-crash bytes").unwrap();
    let fresh = ManagedInstaller::for_home(&home).unwrap();
    assert_eq!(fresh.recover_command(), Err(InstallError::Collision));
    assert_eq!(fs::read(&command).unwrap(), b"arbitrary post-crash bytes");
    let journal = state(&home).join("command.journal");
    let before = fs::read(&journal).unwrap();
    fs::write(&journal, b"{\"formatVersion\":900}").unwrap();
    assert_eq!(fresh.recover_command(), Err(InstallError::InvalidState));
    assert_eq!(fs::read(journal).unwrap(), b"{\"formatVersion\":900}");
    assert!(!before.is_empty());
}
#[test]
fn moved_package_directory_does_not_affect_copied_command_or_later_refresh() {
    let (_t, home, source) = fixture();
    install(&home, &source);
    fs::rename(&source, home.join("moved-source")).unwrap();
    let output = std::process::Command::new(home.join(".local/bin/tmx"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"copied-cli");
    let moved = home.join("moved-source");
    fs::write(&moved, b"#!/bin/sh\nprintf refreshed").unwrap();
    let i = ManagedInstaller::for_home(&home).unwrap();
    let p = i
        .preview(&moved, admitted(&moved, "1.1.0", "installation-a"))
        .unwrap();
    i.apply(&p, Approval::Automatic).unwrap();
    assert_eq!(
        std::process::Command::new(i.command_path())
            .output()
            .unwrap()
            .stdout,
        b"refreshed"
    );
}
#[test]
fn installer_lock_child() {
    let Some(root) = std::env::var_os("INSTALL_LOCK_FIXTURE_ROOT") else {
        return;
    };
    let f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PathBuf::from(root).join("install.lock"))
        .unwrap();
    rustix::fs::flock(&f, rustix::fs::FlockOperation::LockExclusive).unwrap();
    println!("LOCK_READY");
    std::io::stdout().flush().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}
#[test]
fn two_processes_do_not_wait_and_death_releases_the_installer_lock() {
    let (_t, home, source) = fixture();
    install(&home, &source);
    fs::write(&source, b"updated").unwrap();
    let i = ManagedInstaller::for_home(&home).unwrap();
    let p = i
        .preview(&source, admitted(&source, "1.1.0", "installation-a"))
        .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "installer_lock_child", "--nocapture"])
        .env("INSTALL_LOCK_FIXTURE_ROOT", state(&home))
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
    let start = std::time::Instant::now();
    assert_eq!(i.apply(&p, Approval::Automatic), Err(InstallError::Busy));
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    child.kill().unwrap();
    child.wait().unwrap();
    i.apply(&p, Approval::Automatic).unwrap();
}
#[test]
fn concurrent_previews_cannot_overwrite_a_completed_refresh() {
    let (_t, home, source) = fixture();
    install(&home, &source);
    fs::write(&source, b"updated").unwrap();
    let i = ManagedInstaller::for_home(&home).unwrap();
    let a = i
        .preview(&source, admitted(&source, "1.1.0", "installation-a"))
        .unwrap();
    let b = i
        .preview(&source, admitted(&source, "1.1.0", "installation-a"))
        .unwrap();
    let root = home.clone();
    let t = std::thread::spawn(move || {
        ManagedInstaller::for_home(&root)
            .unwrap()
            .apply(&a, Approval::Automatic)
    });
    let second = i.apply(&b, Approval::Automatic);
    let first = t.join().unwrap();
    assert_eq!([first, second].iter().filter(|r| r.is_ok()).count(), 1);
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
fn installer_crash_child() {
    let Some(root) = std::env::var_os("INSTALL_CRASH_FIXTURE_HOME") else {
        return;
    };
    let home = PathBuf::from(root);
    let source = home.join("package-backend");
    let i = ManagedInstaller::for_home(&home).unwrap();
    let p = i
        .preview(&source, admitted(&source, "1.1.0", "installation-a"))
        .unwrap();
    let n = std::env::var("INSTALL_CRASH_BOUNDARY")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    i.with_fault(Arc::new(Crash(boundaries()[n])))
        .apply(&p, Approval::Automatic)
        .unwrap();
    panic!("crash boundary not reached");
}
#[test]
fn process_restart_at_every_boundary_completes_or_requires_explicit_recognized_repair() {
    for (n, boundary) in boundaries().into_iter().enumerate() {
        let (_t, home, source) = fixture();
        install(&home, &source);
        fs::write(&source, b"new admitted executable after process death").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "installer_crash_child"])
            .env("INSTALL_CRASH_FIXTURE_HOME", &home)
            .env("INSTALL_CRASH_BOUNDARY", n.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(77),
            "{boundary:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fresh = ManagedInstaller::for_home(&home).unwrap();
        let recovery = fresh.recover_command().unwrap();
        let p = fresh
            .preview(&source, admitted(&source, "1.1.0", "installation-a"))
            .unwrap();
        if recovery == Recovery::RepairRequired {
            assert_eq!(p.disposition(), Disposition::ExplicitRepair);
            assert_eq!(
                fresh.apply(&p, Approval::Automatic),
                Err(InstallError::ConsentRequired)
            );
        }
        fresh.apply(&p, Approval::Explicit).unwrap();
        assert_eq!(
            fs::read(fresh.command_path()).unwrap(),
            b"new admitted executable after process death"
        );
    }
}

#[test]
fn unrelated_command_temporary_file_is_a_collision_not_cleanup_authority() {
    let (_t, home, source) = fixture();
    install(&home, &source);
    let temp = home.join(".local/bin/.tmx.tmp");
    fs::write(&temp, b"unrelated private bytes").unwrap();
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&source, b"updated").unwrap();
    let i = ManagedInstaller::for_home(&home).unwrap();
    let p = i
        .preview(&source, admitted(&source, "1.1.0", "installation-a"))
        .unwrap();
    assert_eq!(
        i.apply(&p, Approval::Automatic),
        Err(InstallError::Collision)
    );
    assert_eq!(fs::read(temp).unwrap(), b"unrelated private bytes");
}
