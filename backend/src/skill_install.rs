//! Selected portable skill only: no searchable folder registry, secret or Provider-schema copy.
use crate::managed_install::{Admitted, ManagedInstaller, Recovery, Result};
use crate::managed_install::{
    Boundary, InstallError, Journal, Kind, MAX_STATE, Observed, Pin, Receipt, SKILL_RECEIPT,
    create, digest, directory_safe, discard_temp, encode, ensure_old, hash_file, hex, io, observed,
    open_dir, parse, pending_journal, read_at,
};
use rustix::fs::{self as sys, Mode};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};
pub const TEMPLATE: &str = include_str!("../resources/teamarness/SKILL.md");
/// Single-quote one literal POSIX shell word, including embedded apostrophes.
pub fn quoted_command(path: &Path) -> Result<String> {
    let raw = path
        .to_str()
        .ok_or(crate::managed_install::InstallError::Unsafe)?;
    if !path.is_absolute() || raw.len() > 4096 || raw.chars().any(char::is_control) {
        return Err(crate::managed_install::InstallError::Unsafe);
    }
    Ok(format!("'{}'", raw.replace('\'', "'\\''")))
}
pub struct SkillInstaller<'a> {
    managed: &'a ManagedInstaller,
}
impl<'a> SkillInstaller<'a> {
    pub fn new(managed: &'a ManagedInstaller) -> Self {
        Self { managed }
    }
    pub fn preview(&self, selected_folder: &Path, admitted: Admitted) -> Result<SkillPlan> {
        let content = TEMPLATE.replace(
            "{{COMMAND}}",
            &quoted_command(&self.managed.command_path())?,
        );
        self.managed
            .preview_skill(selected_folder, admitted, content.into_bytes())
    }
    pub fn apply(&self, plan: &SkillPlan) -> Result<()> {
        self.managed.apply_skill(plan)
    }
    pub fn recover(&self) -> Result<Recovery> {
        self.managed.recover_skill()
    }
}

#[derive(Clone)]
pub struct SkillPlan {
    selected: Pin,
    destination: Pin,
    state: Pin,
    old: Observed,
    new_receipt: Vec<u8>,
    content: Vec<u8>,
    command: PathBuf,
    pending: Option<Vec<u8>>,
}
impl SkillPlan {
    pub fn destination(&self) -> &Path {
        &self.destination.path
    }
    pub fn unchanged(&self) -> bool {
        self.old.digest.as_deref() == Some(&digest(&self.content))
            && self.old.receipt.as_ref() == Some(&self.new_receipt)
    }
}
impl ManagedInstaller {
    pub(crate) fn preview_skill(
        &self,
        selected: &Path,
        admitted: Admitted,
        content: Vec<u8>,
    ) -> Result<SkillPlan> {
        self.home_pin.check()?;
        admitted.validate()?;
        let selected = Pin::capture(selected)?;
        selected.open()?;
        let destination = Pin::capture(&selected.path.join("teamarness"))?;
        if destination.complete {
            check_skill_entries(&destination.open()?)?;
        }
        let state = Pin::capture(&self.state_path())?;
        let pending = pending_journal(&state, "skill.journal")?;
        if destination.complete && pending.is_none() {
            let dir = destination.open()?;
            for name in [".skill.tmp", ".teamarness-receipt.json.tmp"] {
                if read_at(&dir, name, 0o600, 65536)?.is_some() {
                    return Err(InstallError::Collision);
                }
            }
        }
        let old = observed(
            &destination.path.join("SKILL.md"),
            &destination.path.join(SKILL_RECEIPT),
            Kind::Skill,
        )?;
        // An existing empty/unrelated directory is not an install target.
        if destination.complete && old.receipt.is_none() {
            return Err(InstallError::Collision);
        }
        if let Some(bytes) = &pending {
            let j: Journal = parse(bytes)?;
            j.validate()?;
            if j.kind != Kind::Skill || j.output != destination.path.join("SKILL.md") {
                return Err(InstallError::RepairRequired);
            }
            j.pins[0].check()?;
            ensure_old(&j)?;
            if destination.complete {
                for pin in &j.pins {
                    pin.check()?;
                }
            }
        }
        let new_receipt = encode(&Receipt::new(admitted, digest(&content), Kind::Skill))?;
        Ok(SkillPlan {
            selected,
            destination,
            state,
            old,
            new_receipt,
            content,
            command: self.command_path(),
            pending,
        })
    }
    pub(crate) fn apply_skill(&self, plan: &SkillPlan) -> Result<()> {
        if plan.command != self.command_path() {
            return Err(InstallError::StalePreview);
        }
        plan.selected.check()?;
        plan.destination.check()?;
        plan.state.check()?;
        let (state, _lock) = self.lock()?;
        plan.selected.check()?;
        plan.destination.check()?;
        plan.state.existing_check()?;
        if read_at(&state, "skill.journal", 0o600, MAX_STATE as u64)? != plan.pending {
            return Err(InstallError::StalePreview);
        }
        let output = plan.destination.path.join("SKILL.md");
        let receipt = plan.destination.path.join(SKILL_RECEIPT);
        if observed(&output, &receipt, Kind::Skill)? != plan.old {
            return Err(InstallError::StalePreview);
        }
        if plan.unchanged() && plan.pending.is_none() {
            return Ok(());
        }
        let parent = plan.selected.open()?;
        if !plan.destination.complete {
            // One whole new directory is staged privately; no half-populated destination is visible.
            if let Some(bytes) = &plan.pending {
                let j: Journal = parse(bytes)?;
                discard_skill_stage(&parent, &plan.selected.path, &j)?;
            }
            let stage_name = create_stage(&parent)?;
            let stage_path = plan.selected.path.join(&stage_name);
            let staged = open_dir(&stage_path, false, true)?;
            let mut pin = Pin::capture(&stage_path)?;
            pin.path = plan.destination.path.clone();
            let mut j = Journal::new(
                Kind::Skill,
                output,
                receipt,
                vec![plan.selected.clone(), pin],
                plan.old.clone(),
                plan.new_receipt.clone(),
            );
            j.stage_name = Some(stage_name.clone());
            if let Err(error) = self.write_journal(&state, "skill.journal", &j) {
                // If the journal was not renamed, remove only this still-empty staged directory.
                if read_at(&state, "skill.journal", 0o600, MAX_STATE as u64)? != Some(encode(&j)?) {
                    let _ = sys::unlinkat(&parent, &stage_name, sys::AtFlags::REMOVEDIR);
                }
                return Err(error);
            }
            let mut f = create(&staged, "SKILL.md")?;
            self.boundary(Boundary::CopyWrite)?;
            io(f.write_all(&plan.content))?;
            self.boundary(Boundary::CopiedVerify)?;
            if hash_file(&mut f)? != digest(&plan.content) {
                return Err(InstallError::Collision);
            }
            self.boundary(Boundary::FileSync)?;
            io(f.sync_all())?;
            let mut r = create(&staged, SKILL_RECEIPT)?;
            self.boundary(Boundary::ReceiptWrite)?;
            io(r.write_all(&plan.new_receipt))?;
            self.boundary(Boundary::ReceiptSync)?;
            io(r.sync_all())?;
            io(staged.sync_all())?;
            plan.selected.check()?;
            plan.destination.check()?;
            self.boundary(Boundary::OutputRename)?;
            plan.selected.check()?;
            plan.destination.check()?;
            io(sys::renameat_with(
                &parent,
                &stage_name,
                &parent,
                "teamarness",
                sys::RenameFlags::NOREPLACE,
            ))?;
            self.boundary(Boundary::OutputDirectorySync)?;
            io(parent.sync_all())?;
            self.finish(&state, "skill.journal", &j)
        } else {
            let dest = plan.destination.open()?;
            check_skill_entries(&dest)?;
            let j = Journal::new(
                Kind::Skill,
                output,
                receipt,
                vec![plan.selected.clone(), plan.destination.clone()],
                plan.old.clone(),
                plan.new_receipt.clone(),
            );
            self.write_journal(&state, "skill.journal", &j)?;
            discard_temp(&dest, ".skill.tmp", 0o600)?;
            let mut f = create(&dest, ".skill.tmp")?;
            self.boundary(Boundary::CopyWrite)?;
            io(f.write_all(&plan.content))?;
            self.boundary(Boundary::CopiedVerify)?;
            if hash_file(&mut f)? != digest(&plan.content) {
                return Err(InstallError::Collision);
            }
            self.boundary(Boundary::FileSync)?;
            io(f.sync_all())?;
            plan.selected.check()?;
            plan.destination.check()?;
            ensure_old(&j)?;
            self.boundary(Boundary::OutputRename)?;
            plan.selected.check()?;
            plan.destination.check()?;
            ensure_old(&j)?;
            io(sys::renameat(&dest, ".skill.tmp", &dest, "SKILL.md"))?;
            self.boundary(Boundary::OutputDirectorySync)?;
            io(dest.sync_all())?;
            self.finish(&state, "skill.journal", &j)
        }
    }
}
fn check_skill_entries(dir: &File) -> Result<()> {
    for e in io(sys::Dir::read_from(dir))? {
        let e = io(e)?;
        let name = e.file_name().to_str().map_err(|_| InstallError::Unsafe)?;
        if ![
            ".",
            "..",
            "SKILL.md",
            SKILL_RECEIPT,
            ".skill.tmp",
            ".teamarness-receipt.json.tmp",
        ]
        .contains(&name)
        {
            return Err(InstallError::Collision);
        }
    }
    Ok(())
}

fn discard_skill_stage(parent: &File, selected: &Path, j: &Journal) -> Result<()> {
    if j.old_digest.is_some() || j.pins.len() != 2 {
        return Err(InstallError::InvalidState);
    }
    let mut pin = j.pins[1].clone();
    let stage_name = j.stage_name.as_deref().ok_or(InstallError::InvalidState)?;
    pin.path = selected.join(stage_name);
    pin.check()?;
    let stage = pin.open()?;
    directory_safe(&stage, true, true)?;
    for entry in io(sys::Dir::read_from(&stage))? {
        let entry = io(entry)?;
        let name = entry
            .file_name()
            .to_str()
            .map_err(|_| InstallError::Unsafe)?;
        if [".", ".."].contains(&name) {
            continue;
        }
        if !["SKILL.md", SKILL_RECEIPT].contains(&name) {
            return Err(InstallError::Collision);
        }
        read_at(&stage, name, 0o600, 65536)?;
    }
    for name in ["SKILL.md", SKILL_RECEIPT] {
        match sys::unlinkat(&stage, name, sys::AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(_) => return Err(InstallError::Io),
        }
    }
    io(sys::unlinkat(parent, stage_name, sys::AtFlags::REMOVEDIR))?;
    io(parent.sync_all())
}

fn create_stage(parent: &File) -> Result<String> {
    for _ in 0..4 {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| InstallError::Io)?;
        let name = format!(".teamarness-{}.tmp", hex(&random));
        match sys::mkdirat(parent, &name, Mode::RWXU) {
            Ok(()) => return Ok(name),
            Err(rustix::io::Errno::EXIST) => {}
            Err(_) => return Err(InstallError::Io),
        }
    }
    Err(InstallError::Collision)
}
