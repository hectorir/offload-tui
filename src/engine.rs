//! The copy engine.
//!
//! Pipeline per file:
//!   reader thread  -> (hash source while reading) -> writer (this thread) -> `.<name>.part`
//!   verifier thread: fsync, re-read the `.part` bypassing the cache, compare BLAKE3, then rename.
//!
//! A file only ever gets its real name after its on-disk bytes hash identically to the source,
//! so a partial or corrupt file can never be mistaken for a good one. Sources are opened
//! read-only and never modified or deleted. Existing destination files are never overwritten.

use crate::config::{Config, Profile};
use crate::fsutil::{self, BUF};
use crate::scan::FileEntry;
use anyhow::{Result, bail};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// What happens to the original once its copy is verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Keep the original on the card.
    #[default]
    Copy,
    /// Delete the original from the card right after its copy is verified (a "move").
    Move,
}

impl Mode {
    pub fn from_config(s: &str) -> Mode {
        if s.eq_ignore_ascii_case("move") { Mode::Move } else { Mode::Copy }
    }
    pub fn toggled(self) -> Mode {
        match self {
            Mode::Copy => Mode::Move,
            Mode::Move => Mode::Copy,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Mode::Copy => "copy",
            Mode::Move => "move",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub source: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    pub name: String,
    /// Folder this file is written into (`<dest>/<date>/<subfolder>`).
    pub dest_dir: PathBuf,
    /// Which camera/profile it belongs to (index into the config's profiles).
    pub group: usize,
    /// Second-drive folder this file is also written into, when mirroring.
    pub mirror_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// Single camera: that camera's folder. Batch: the shared dated folder.
    pub dest_dir: PathBuf,
    pub items: Vec<PlanItem>,
    pub mode: Mode,
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    /// Distinct destination folders, in first-use order.
    pub fn dest_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = vec![];
        for i in &self.items {
            if !v.contains(&i.dest_dir) {
                v.push(i.dest_dir.clone());
            }
        }
        v
    }

    /// Distinct second-drive folders, in first-use order.
    pub fn mirror_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = vec![];
        for d in self.items.iter().filter_map(|i| i.mirror_dir.as_ref()) {
            if !v.contains(d) {
                v.push(d.clone());
            }
        }
        v
    }

    pub fn is_mirrored(&self) -> bool {
        self.items.iter().any(|i| i.mirror_dir.is_some())
    }
}

/// One camera's share of a batch.
pub struct BatchPart<'a> {
    pub profile_index: usize,
    pub profile: &'a Profile,
    pub files: Vec<&'a FileEntry>,
}

/// `<dest>/<date>/<subfolder>/` for one profile.
pub fn profile_dest_dir(cfg: &Config, profile: &Profile, date: &str) -> PathBuf {
    dated_dir(cfg.dest_base_for(profile), profile, date)
}

fn dated_dir(base: PathBuf, profile: &Profile, date: &str) -> PathBuf {
    let mut dir = base.join(date);
    if !profile.subfolder.is_empty() {
        dir = dir.join(&profile.subfolder);
    }
    dir
}

/// The dated folder name for one file: the day of the offload, or (when organizing by shoot date) the day it was recorded.
pub fn date_folder(cfg: &Config, run_date: &str, shot: chrono::NaiveDate) -> Result<String> {
    if !cfg.organize_by_shoot_date() {
        return Ok(run_date.to_string());
    }
    let d = cfg.render_naive_date(shot)?;
    crate::config::validate_component(&d)?;
    if d.is_empty() {
        bail!("date_format renders an empty name");
    }
    Ok(d)
}

/// Where a file lands on the main drive.
pub fn file_dest_dir(cfg: &Config, profile: &Profile, run_date: &str, shot: chrono::NaiveDate) -> Result<PathBuf> {
    Ok(profile_dest_dir(cfg, profile, &date_folder(cfg, run_date, shot)?))
}

/// Build one plan covering several cameras. Each camera keeps its own subfolder under the dated folder.
/// Honors `cfg.organize` (per-shoot-day folders) and `cfg.mirror` (a second drive written in the same pass).
pub fn build_batch_plan(cfg: &Config, parts: &[BatchPart], now: chrono::DateTime<chrono::Local>) -> Result<Plan> {
    let Some(first) = parts.first() else { bail!("no cameras in the batch") };
    let run_date = cfg.render_date(now)?;
    crate::config::validate_component(&run_date)?;
    let mirror = cfg.mirror_base();
    let mut items = vec![];
    for part in parts {
        part.profile.validate()?;
        for f in &part.files {
            let date = date_folder(cfg, &run_date, f.date)?;
            items.push(PlanItem {
                source: f.path.clone(),
                size: f.size,
                modified: f.modified,
                name: f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                dest_dir: dated_dir(cfg.dest_base_for(part.profile), part.profile, &date),
                group: part.profile_index,
                mirror_dir: mirror.clone().map(|m| dated_dir(m, part.profile, &date)),
            });
        }
    }
    // What the UI shows as "the" destination: the dated folder, or the base folder when files fan out by shoot day.
    let dest_dir = if cfg.organize_by_shoot_date() {
        cfg.dest_base_for(first.profile)
    } else if parts.len() == 1 {
        profile_dest_dir(cfg, first.profile, &run_date)
    } else {
        cfg.dest_base_for(first.profile).join(&run_date)
    };
    Ok(Plan { dest_dir, items, mode: Mode::Copy })
}

/// Build the plan for a single profile: `<dest>/<date>/<subfolder>/`.
pub fn build_plan(cfg: &Config, profile: &Profile, files: &[&FileEntry], now: chrono::DateTime<chrono::Local>) -> Result<Plan> {
    build_batch_plan(cfg, &[BatchPart { profile_index: 0, profile, files: files.to_vec() }], now)
}

/// Bytes that would actually need writing (files already present with the same size are assumed done;
/// the checksum decides for real during the copy).
pub fn bytes_needed(plan: &Plan) -> u64 {
    plan.items.iter().filter(|i| needs_write(i)).map(|i| i.size).sum()
}

fn needs_write(i: &PlanItem) -> bool {
    fs::symlink_metadata(i.dest_dir.join(&i.name)).map(|m| m.len() != i.size).unwrap_or(true)
}

fn mirror_needs_write(i: &PlanItem, dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(&i.name)).map(|m| m.len() != i.size).unwrap_or(true)
}

fn device_of(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let mut p = path.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return 0;
        }
    }
    fs::metadata(&p).map(|m| m.dev()).unwrap_or(0)
}

/// Checks that must pass before any byte is written.
pub fn preflight(plan: &Plan) -> std::result::Result<(), String> {
    if plan.items.is_empty() {
        return Err("Nothing selected to copy.".into());
    }
    for dir in plan.dest_dirs() {
        if !fsutil::destination_volume_present(&dir) {
            return Err(format!(
                "Destination drive is not mounted ({}). Plug it in and try again.",
                fsutil::volume_root(&dir).map(|p| p.display().to_string()).unwrap_or_default()
            ));
        }
    }
    for dir in plan.mirror_dirs() {
        if !fsutil::destination_volume_present(&dir) {
            return Err(format!(
                "The second (mirror) drive is not mounted ({}). Plug it in, or turn mirroring off.",
                fsutil::volume_root(&dir).map(|p| p.display().to_string()).unwrap_or_default()
            ));
        }
    }
    // Space is checked per physical filesystem, since cameras (and the mirror) may be on different drives.
    let mut per_device: Vec<(u64, PathBuf, u64)> = vec![];
    let mut add = |dir: &Path, size: u64| {
        let dev = device_of(dir);
        match per_device.iter_mut().find(|(d, _, _)| *d == dev) {
            Some(e) => e.2 += size,
            None => per_device.push((dev, dir.to_path_buf(), size)),
        }
    };
    for i in &plan.items {
        if needs_write(i) {
            add(&i.dest_dir, i.size);
        }
        if let Some(m) = &i.mirror_dir
            && mirror_needs_write(i, m)
        {
            add(m, i.size);
        }
    }
    for (_, dir, needed) in per_device {
        if let Some((_, free)) = fsutil::disk_space(&dir)
            && free < needed.saturating_add(1 << 20)
        {
            return Err(format!("Not enough space on the destination: need {}, only {} free.", fsutil::human_bytes(needed), fsutil::human_bytes(free)));
        }
    }
    Ok(())
}

/// How the second-drive copy of a file turned out (only relevant when mirroring).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mirror {
    Off,
    /// Written and checksum-verified there (or an identical file was already there).
    Ok(PathBuf),
    Failed(String),
}

/// What happened to the original on the card (only relevant in move mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    NotRequested,
    Removed,
    /// The copy is safe, but the original could not be deleted (and was left alone).
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Copied and verified. `renamed` = a different file already had that name.
    Copied {
        dest: PathBuf,
        renamed: bool,
        removed: Removal,
        mirror: Mirror,
    },
    /// Identical file (same checksum) already at the destination.
    Skipped {
        dest: PathBuf,
        removed: Removal,
        mirror: Mirror,
    },
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum Event {
    Started { index: usize },
    Progress { index: usize, bytes: u64 },
    Verifying { index: usize },
    VerifyProgress { index: usize, bytes: u64 },
    Done { index: usize, outcome: Outcome },
    Finished(Summary),
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub dest_dir: PathBuf,
    pub copied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub renamed: usize,
    pub bytes_copied: u64,
    pub failures: Vec<(String, String)>,
    /// Move mode: originals deleted from the card, and ones that could not be.
    pub moved: usize,
    pub move_failures: Vec<(String, String)>,
    /// Mirroring: files verified on the second drive, and ones that could not be.
    pub mirrored: usize,
    pub mirror_failures: Vec<(String, String)>,
    pub cancelled: bool,
    pub elapsed: Duration,
}

#[derive(Default)]
pub struct Control {
    paused: AtomicBool,
    cancelled: AtomicBool,
}

impl Control {
    pub fn new() -> Arc<Control> {
        Arc::new(Control::default())
    }
    pub fn set_paused(&self, v: bool) {
        self.paused.store(v, Ordering::SeqCst);
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    /// Block while paused; returns false if cancelled.
    fn checkpoint(&self) -> bool {
        while self.is_paused() && !self.is_cancelled() {
            std::thread::sleep(Duration::from_millis(80));
        }
        !self.is_cancelled()
    }
}

struct Tally {
    summary: Summary,
    tx: Sender<Event>,
}

impl Tally {
    fn count_removal(&mut self, name: &str, removed: &Removal) {
        match removed {
            Removal::Removed => self.summary.moved += 1,
            Removal::Failed(why) => self.summary.move_failures.push((name.to_string(), why.clone())),
            Removal::NotRequested => {}
        }
    }

    fn count_mirror(&mut self, name: &str, mirror: &Mirror) {
        match mirror {
            Mirror::Ok(_) => self.summary.mirrored += 1,
            Mirror::Failed(why) => self.summary.mirror_failures.push((name.to_string(), why.clone())),
            Mirror::Off => {}
        }
    }

    fn done(tally: &Mutex<Tally>, index: usize, name: &str, size: u64, outcome: Outcome) {
        let mut t = tally.lock().unwrap_or_else(|e| e.into_inner());
        match &outcome {
            Outcome::Copied { renamed, removed, mirror, .. } => {
                t.count_mirror(name, mirror);
                t.summary.copied += 1;
                t.summary.bytes_copied += size;
                if *renamed {
                    t.summary.renamed += 1;
                }
                t.count_removal(name, removed);
            }
            Outcome::Skipped { removed, mirror, .. } => {
                t.count_mirror(name, mirror);
                t.summary.skipped += 1;
                t.count_removal(name, removed);
            }
            Outcome::Failed(why) => {
                t.summary.failed += 1;
                t.summary.failures.push((name.to_string(), why.clone()));
            }
            Outcome::Cancelled => t.summary.cancelled = true,
        }
        let _ = t.tx.send(Event::Done { index, outcome });
    }
}

/// One destination of a file (the main drive, or the mirror) as it moves through the pipeline.
enum Slot {
    /// Not requested (no mirror).
    Off,
    /// An identical file is already there.
    Skip(PathBuf),
    /// Being written to `part`; becomes `final_path` once verified.
    Write {
        part: PathBuf,
        final_path: PathBuf,
        renamed: bool,
    },
    /// Verified and in place.
    Done {
        path: PathBuf,
        renamed: bool,
    },
    Failed(String),
}

struct VerifyJob {
    index: usize,
    primary: Slot,
    mirror: Slot,
    expected: blake3::Hash,
    written: u64,
    modified: SystemTime,
    name: String,
    size: u64,
    source: PathBuf,
}

enum Resolved {
    Skip(PathBuf),
    Use { final_path: PathBuf, renamed: bool },
}

/// Delete one original from the card, but only if everything checks out:
/// every verified copy (main drive, and the mirror if there is one) is still in place with the right size,
/// and the original is a plain file that has not changed since it was scanned. On any doubt the original is left alone.
pub fn remove_original(source: &Path, size: u64, modified: SystemTime, dests: &[&Path]) -> std::result::Result<(), String> {
    if dests.is_empty() {
        return Err("no verified copy to rely on; original kept".into());
    }
    for dest in dests {
        match fs::symlink_metadata(dest) {
            Ok(m) if m.is_file() && m.len() == size => {}
            Ok(_) => return Err("a copy on the destination has a different size; original kept".into()),
            Err(_) => return Err("a copy is no longer on the destination; original kept".into()),
        }
    }
    match fs::symlink_metadata(source) {
        Ok(m) if m.is_file() && m.len() == size && m.modified().ok() == Some(modified) => {}
        Ok(_) => return Err("the original changed since it was scanned; kept".into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err("the original is already gone".into()),
        Err(e) => return Err(format!("cannot inspect the original: {e}")),
    }
    fs::remove_file(source).map_err(|e| format!("could not delete the original: {e}"))
}

/// An original whose copy has been verified (candidate for deleting after the fact).
#[derive(Debug, Clone)]
pub struct Original {
    pub source: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    pub dest: PathBuf,
    /// The verified second-drive copy, when mirroring.
    pub mirror: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct DeleteReport {
    pub deleted: usize,
    pub bytes: u64,
    pub failures: Vec<(String, String)>,
}

/// Delete verified originals, each with the same safety checks as a move.
pub fn delete_originals(list: &[Original]) -> DeleteReport {
    let mut rep = DeleteReport::default();
    for o in list {
        let mut dests: Vec<&Path> = vec![&o.dest];
        if let Some(m) = &o.mirror {
            dests.push(m);
        }
        match remove_original(&o.source, o.size, o.modified, &dests) {
            Ok(()) => {
                rep.deleted += 1;
                rep.bytes += o.size;
            }
            Err(e) => rep.failures.push((o.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), e)),
        }
    }
    rep
}

fn candidate_name(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    let p = Path::new(name);
    match (p.file_stem(), p.extension()) {
        (Some(s), Some(e)) => format!("{}_{}.{}", s.to_string_lossy(), n, e.to_string_lossy()),
        _ => format!("{name}_{n}"),
    }
}

/// Decide where a file goes: skip it if an identical copy exists, otherwise pick a free name.
fn resolve_dest(dest_dir: &Path, item: &PlanItem, reserved: &mut HashSet<String>) -> io::Result<Resolved> {
    let mut src_hash: Option<blake3::Hash> = None;
    for n in 0..10_000u32 {
        let cand = candidate_name(&item.name, n);
        let path = dest_dir.join(&cand);
        // macOS volumes are usually case-insensitive; treat names case-insensitively to be safe.
        let key = path.to_string_lossy().to_lowercase();
        if reserved.contains(&key) {
            continue;
        }
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                reserved.insert(key);
                return Ok(Resolved::Use { final_path: path, renamed: n > 0 });
            }
            Err(e) => return Err(e),
            Ok(meta) => {
                if meta.is_file() && meta.len() == item.size {
                    let s = match src_hash {
                        Some(h) => h,
                        None => {
                            let h = fsutil::hash_file(&item.source, false, |_| true)?.0;
                            src_hash = Some(h);
                            h
                        }
                    };
                    let (d, _) = fsutil::hash_file(&path, true, |_| true)?;
                    if s == d {
                        reserved.insert(key);
                        return Ok(Resolved::Skip(path));
                    }
                }
                // Different content (or not a regular file): keep both, try the next name.
            }
        }
    }
    Err(io::Error::other("could not find a free destination name"))
}

fn removal(r: std::result::Result<(), String>) -> Removal {
    match r {
        Ok(()) => Removal::Removed,
        Err(e) => Removal::Failed(e),
    }
}

/// Turn the final state of both destinations into the file's outcome, deleting the original in move mode
/// only when every requested copy is verified.
fn outcome_for(primary: Slot, mirror: Slot, mode: Mode, source: &Path, size: u64, modified: SystemTime) -> Outcome {
    let mirror = match mirror {
        Slot::Off => Mirror::Off,
        Slot::Skip(p) | Slot::Done { path: p, .. } => Mirror::Ok(p),
        Slot::Failed(e) => Mirror::Failed(e),
        Slot::Write { .. } => Mirror::Failed("second-drive copy was never verified".into()),
    };
    let (dest, renamed, skipped) = match primary {
        Slot::Skip(p) => (p, false, true),
        Slot::Done { path, renamed } => (path, renamed, false),
        Slot::Failed(e) => return Outcome::Failed(e),
        Slot::Off | Slot::Write { .. } => return Outcome::Failed("copy was never verified".into()),
    };
    let removed = if mode == Mode::Move {
        match &mirror {
            Mirror::Failed(_) => Removal::Failed("the second-drive copy failed; original kept".into()),
            Mirror::Ok(m) => removal(remove_original(source, size, modified, &[&dest, m])),
            Mirror::Off => removal(remove_original(source, size, modified, &[&dest])),
        }
    } else {
        Removal::NotRequested
    };
    if skipped { Outcome::Skipped { dest, removed, mirror } } else { Outcome::Copied { dest, renamed, removed, mirror } }
}

fn part_path(final_path: &Path) -> PathBuf {
    let name = final_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    final_path.with_file_name(format!(".{name}.part"))
}

enum CopyErr {
    Cancelled,
    /// The source could not be read (every destination fails).
    Io(String),
    /// Every destination failed to write (one message per destination, in order).
    Dests(Vec<String>),
}

/// Stream `src` into fresh `.part` files (one per destination), hashing the source bytes as they are read,
/// so a file read once from the card can land on two drives. A destination that fails is dropped on its own;
/// the others carry on. Returns (hash, bytes written, per-destination error).
fn copy_stream(
    src: &Path,
    parts: &[&Path],
    ctrl: &Control,
    mut on_bytes: impl FnMut(u64),
) -> std::result::Result<(blake3::Hash, u64, Vec<Option<String>>), CopyErr> {
    let io = |what: &str, e: io::Error| format!("{what}: {e}");
    let mut input = File::open(src).map_err(|e| CopyErr::Io(io("open source", e)))?;
    if !input.metadata().map_err(|e| CopyErr::Io(io("stat source", e)))?.is_file() {
        return Err(CopyErr::Io("source is not a regular file".into()));
    }
    let mut errs: Vec<Option<String>> = vec![None; parts.len()];
    let mut outs: Vec<Option<File>> = vec![];
    for (i, part) in parts.iter().enumerate() {
        let opened = (|| -> std::result::Result<File, String> {
            // A stale .part from an interrupted run is removed (never followed if it is a symlink).
            match fs::remove_file(part) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io("remove stale .part", e)),
            }
            OpenOptions::new()
                .write(true)
                .create_new(true) // O_EXCL: never write through a pre-existing file or symlink
                .custom_flags(libc::O_NOFOLLOW)
                .open(part)
                .map_err(|e| io("create destination", e))
        })();
        match opened {
            Ok(f) => outs.push(Some(f)),
            Err(e) => {
                errs[i] = Some(e);
                outs.push(None);
            }
        }
    }
    if outs.iter().all(Option::is_none) {
        return Err(CopyErr::Dests(errs.into_iter().map(|e| e.unwrap_or_default()).collect()));
    }

    let mut written = 0u64;
    let result: std::result::Result<blake3::Hash, CopyErr> = std::thread::scope(|s| {
        let (btx, brx) = sync_channel::<Vec<u8>>(4);
        let reader = s.spawn(move || -> io::Result<blake3::Hash> {
            let mut h = blake3::Hasher::new();
            loop {
                let mut buf = vec![0u8; BUF];
                let mut n = 0;
                while n < BUF {
                    // Fill the whole buffer where possible so chunk sizes stay large.
                    let r = input.read(&mut buf[n..])?;
                    if r == 0 {
                        break;
                    }
                    n += r;
                }
                if n == 0 {
                    break;
                }
                buf.truncate(n);
                h.update(&buf);
                if btx.send(buf).is_err() {
                    break; // writer gone (error or cancel)
                }
            }
            Ok(h.finalize())
        });

        let mut failure: Option<CopyErr> = None;
        while let Ok(buf) = brx.recv() {
            if !ctrl.checkpoint() {
                failure = Some(CopyErr::Cancelled);
                break;
            }
            for (i, slot) in outs.iter_mut().enumerate() {
                if let Some(f) = slot
                    && let Err(e) = f.write_all(&buf)
                {
                    errs[i] = Some(io("write", e));
                    *slot = None;
                }
            }
            if outs.iter().all(Option::is_none) {
                break;
            }
            written += buf.len() as u64;
            on_bytes(buf.len() as u64);
        }
        drop(brx);
        let read = reader.join();
        if let Some(f) = failure {
            return Err(f);
        }
        if outs.iter().all(Option::is_none) {
            return Err(CopyErr::Dests(errs.iter().map(|e| e.clone().unwrap_or_default()).collect()));
        }
        match read {
            Ok(Ok(h)) => Ok(h),
            Ok(Err(e)) => Err(CopyErr::Io(io("read source", e))),
            Err(_) => Err(CopyErr::Io("reader thread panicked".into())),
        }
    });
    drop(outs);
    match result {
        Ok(h) => {
            // A destination that failed midway leaves a partial file; remove it.
            for (part, e) in parts.iter().zip(&errs) {
                if e.is_some() {
                    let _ = fs::remove_file(part);
                }
            }
            Ok((h, written, errs))
        }
        Err(e) => {
            for part in parts {
                let _ = fs::remove_file(part);
            }
            Err(e)
        }
    }
}

/// Verify one written slot: flush, re-read bypassing the cache, compare to the source hash, then give it its real name.
/// Only the main copy reports progress, so the verify bar still adds up to the plan's size.
fn verify_slot(slot: Slot, job: &VerifyJob, ctrl: &Control, tx: &Sender<Event>, report: bool) -> Slot {
    let Slot::Write { part, final_path, renamed } = slot else { return slot };
    let res = (|| -> std::result::Result<(), String> {
        File::open(&part).and_then(|f| fsutil::flush_best_effort(&f)).map_err(|e| format!("flush: {e}"))?;
        let index = job.index;
        let (got, len) = fsutil::hash_file(&part, true, |b| {
            if report {
                let _ = tx.send(Event::VerifyProgress { index, bytes: b });
            }
            !ctrl.is_cancelled()
        })
        .map_err(|e| if e.kind() == io::ErrorKind::Interrupted { "cancelled".to_string() } else { format!("verify read: {e}") })?;
        if len != job.written || got != job.expected {
            return Err("checksum mismatch: the copy on disk does not match the source".into());
        }
        if fs::symlink_metadata(&final_path).is_ok() {
            return Err("a file appeared at the destination during the copy; not overwriting".into());
        }
        fs::rename(&part, &final_path).map_err(|e| format!("finalize: {e}"))?;
        // Preserve the camera's modification time (best effort).
        if let Ok(f) = File::options().read(true).open(&final_path) {
            let _ = f.set_modified(job.modified);
        }
        Ok(())
    })();
    match res {
        Ok(()) => Slot::Done { path: final_path, renamed },
        Err(e) => {
            let _ = fs::remove_file(&part);
            Slot::Failed(e)
        }
    }
}

fn is_cancelled_slot(s: &Slot) -> bool {
    matches!(s, Slot::Failed(e) if e == "cancelled")
}

/// Run the plan to completion (blocking). Progress is reported through `tx`.
pub fn run(plan: &Plan, ctrl: &Arc<Control>, tx: Sender<Event>) -> Summary {
    let start = Instant::now();
    let tally = Arc::new(Mutex::new(Tally { summary: Summary { dest_dir: plan.dest_dir.clone(), ..Default::default() }, tx: tx.clone() }));

    // Create every destination folder up front; a folder that can't be made fails only its own files.
    let mut bad_dirs: std::collections::HashMap<PathBuf, String> = std::collections::HashMap::new();
    for dir in plan.dest_dirs().into_iter().chain(plan.mirror_dirs()) {
        if let Err(e) = prepare_dest_dir(&dir) {
            bad_dirs.insert(dir, e.to_string());
        }
    }

    let (vtx, vrx): (_, Receiver<VerifyJob>) = sync_channel(8);
    std::thread::scope(|s| {
        let vt = tally.clone();
        let vctrl = ctrl.clone();
        let vevents = tx.clone();
        let mode = plan.mode;
        s.spawn(move || {
            while let Ok(mut job) = vrx.recv() {
                let _ = vevents.send(Event::Verifying { index: job.index });
                let primary = std::mem::replace(&mut job.primary, Slot::Off);
                let mirror = std::mem::replace(&mut job.mirror, Slot::Off);
                if vctrl.is_cancelled() {
                    for slot in [&primary, &mirror] {
                        if let Slot::Write { part, .. } = slot {
                            let _ = fs::remove_file(part);
                        }
                    }
                    Tally::done(&vt, job.index, &job.name, job.size, Outcome::Cancelled);
                    continue;
                }
                let primary = verify_slot(primary, &job, &vctrl, &vevents, true);
                let mirror = verify_slot(mirror, &job, &vctrl, &vevents, false);
                let outcome = if is_cancelled_slot(&primary) || is_cancelled_slot(&mirror) {
                    Outcome::Cancelled
                } else {
                    outcome_for(primary, mirror, mode, &job.source, job.size, job.modified)
                };
                Tally::done(&vt, job.index, &job.name, job.size, outcome);
            }
        });

        let mut reserved = HashSet::new();
        let mut reserved_mirror = HashSet::new();
        for (index, item) in plan.items.iter().enumerate() {
            if ctrl.is_cancelled() || !ctrl.checkpoint() {
                Tally::done(&tally, index, &item.name, item.size, Outcome::Cancelled);
                continue;
            }
            let _ = tx.send(Event::Started { index });
            let slot_for = |dir: &Path, reserved: &mut HashSet<String>| -> Slot {
                if let Some(e) = bad_dirs.get(dir) {
                    return Slot::Failed(format!("cannot create destination: {e}"));
                }
                match resolve_dest(dir, item, reserved) {
                    Ok(Resolved::Skip(p)) => Slot::Skip(p),
                    Ok(Resolved::Use { final_path, renamed }) => Slot::Write { part: part_path(&final_path), final_path, renamed },
                    Err(e) => Slot::Failed(format!("checking destination: {e}")),
                }
            };
            let mut primary = slot_for(&item.dest_dir, &mut reserved);
            if let Slot::Failed(e) = primary {
                Tally::done(&tally, index, &item.name, item.size, Outcome::Failed(e));
                continue;
            }
            let mut mirror = match &item.mirror_dir {
                Some(d) => slot_for(d, &mut reserved_mirror),
                None => Slot::Off,
            };

            // Which destinations need bytes written?
            let mut parts: Vec<PathBuf> = vec![];
            let mut which: Vec<bool> = vec![]; // true = primary, false = mirror
            for (is_primary, slot) in [(true, &primary), (false, &mirror)] {
                if let Slot::Write { part, .. } = slot {
                    parts.push(part.clone());
                    which.push(is_primary);
                }
            }
            if parts.is_empty() {
                // Everything needed is already in place (or the mirror failed to even start).
                let _ = tx.send(Event::Progress { index, bytes: item.size });
                let _ = tx.send(Event::VerifyProgress { index, bytes: item.size });
                let outcome = outcome_for(primary, mirror, plan.mode, &item.source, item.size, item.modified);
                Tally::done(&tally, index, &item.name, item.size, outcome);
                continue;
            }
            let refs: Vec<&Path> = parts.iter().map(|p| p.as_path()).collect();
            match copy_stream(&item.source, &refs, ctrl, |b| {
                let _ = tx.send(Event::Progress { index, bytes: b });
            }) {
                Ok((hash, written, errs)) => {
                    for (e, is_primary) in errs.into_iter().zip(&which) {
                        if let Some(e) = e {
                            *(if *is_primary { &mut primary } else { &mut mirror }) = Slot::Failed(e);
                        }
                    }
                    if let Slot::Failed(e) = primary {
                        // The main copy failed: the file counts as failed, and a mirror copy (if any) is discarded.
                        if let Slot::Write { part, .. } = &mirror {
                            let _ = fs::remove_file(part);
                        }
                        Tally::done(&tally, index, &item.name, item.size, Outcome::Failed(e));
                        continue;
                    }
                    let job = VerifyJob {
                        index,
                        primary,
                        mirror,
                        expected: hash,
                        written,
                        modified: item.modified,
                        name: item.name.clone(),
                        size: item.size,
                        source: item.source.clone(),
                    };
                    if vtx.send(job).is_err() {
                        Tally::done(&tally, index, &item.name, item.size, Outcome::Failed("verifier stopped unexpectedly".into()));
                    }
                }
                Err(CopyErr::Cancelled) => Tally::done(&tally, index, &item.name, item.size, Outcome::Cancelled),
                Err(CopyErr::Io(e)) => Tally::done(&tally, index, &item.name, item.size, Outcome::Failed(e)),
                Err(CopyErr::Dests(errs)) => {
                    // Every destination that needed bytes failed to take them.
                    for (e, is_primary) in errs.into_iter().zip(&which) {
                        *(if *is_primary { &mut primary } else { &mut mirror }) = Slot::Failed(e);
                    }
                    let outcome = outcome_for(primary, mirror, plan.mode, &item.source, item.size, item.modified);
                    Tally::done(&tally, index, &item.name, item.size, outcome);
                }
            }
        }
        drop(vtx); // lets the verifier finish its queue and exit
    });

    // Persist the directory entries of the finalized files.
    for dir in plan.dest_dirs().into_iter().chain(plan.mirror_dirs()) {
        if let Ok(d) = File::open(&dir) {
            let _ = fsutil::flush_best_effort(&d);
        }
    }
    finish(tally, start, &tx, &plan.dest_dir)
}

fn finish(tally: Arc<Mutex<Tally>>, start: Instant, tx: &Sender<Event>, dest: &Path) -> Summary {
    let mut t = tally.lock().unwrap_or_else(|e| e.into_inner());
    t.summary.elapsed = start.elapsed();
    t.summary.dest_dir = dest.to_path_buf();
    let s = t.summary.clone();
    let _ = tx.send(Event::Finished(s.clone()));
    s
}

/// Create the dated destination folder (never through a symlink).
fn prepare_dest_dir(dir: &Path) -> Result<()> {
    if !fsutil::destination_volume_present(dir) {
        bail!("destination volume is not mounted");
    }
    fs::create_dir_all(dir)?;
    let meta = fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        bail!("{} is not a real directory", dir.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_names() {
        assert_eq!(candidate_name("C0001.MP4", 0), "C0001.MP4");
        assert_eq!(candidate_name("C0001.MP4", 2), "C0001_2.MP4");
        assert_eq!(candidate_name("noext", 1), "noext_1");
    }

    #[test]
    fn part_names_are_hidden() {
        assert_eq!(part_path(Path::new("/a/b/C1.MP4")), PathBuf::from("/a/b/.C1.MP4.part"));
    }
}

#[cfg(test)]
mod verify_tests {
    use super::*;

    fn job(expected: blake3::Hash, written: u64) -> VerifyJob {
        VerifyJob {
            index: 0,
            primary: Slot::Off,
            mirror: Slot::Off,
            expected,
            written,
            modified: SystemTime::now(),
            name: "x.mp4".into(),
            size: written,
            source: PathBuf::new(),
        }
    }

    fn slot(dir: &Path) -> Slot {
        Slot::Write { part: dir.join(".x.mp4.part"), final_path: dir.join("x.mp4"), renamed: false }
    }

    fn failed(s: &Slot) -> &str {
        match s {
            Slot::Failed(e) => e,
            _ => panic!("expected a failure"),
        }
    }

    #[test]
    fn corrupted_copy_is_rejected_and_never_finalized() {
        let d = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        std::fs::write(d.path().join(".x.mp4.part"), b"hello w0rld").unwrap(); // one flipped byte
        let j = job(blake3::hash(b"hello world"), 11);
        let r = verify_slot(slot(d.path()), &j, &Control::default(), &tx, true);
        assert!(failed(&r).contains("checksum mismatch"), "{}", failed(&r));
        assert!(!d.path().join("x.mp4").exists());
    }

    #[test]
    fn truncated_copy_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        std::fs::write(d.path().join(".x.mp4.part"), b"hello").unwrap();
        let j = job(blake3::hash(b"hello world"), 11);
        assert!(matches!(verify_slot(slot(d.path()), &j, &Control::default(), &tx, true), Slot::Failed(_)));
        assert!(!d.path().join("x.mp4").exists());
    }

    #[test]
    fn good_copy_is_finalized_and_does_not_clobber() {
        let d = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        std::fs::write(d.path().join(".x.mp4.part"), b"hello world").unwrap();
        let j = job(blake3::hash(b"hello world"), 11);
        assert!(matches!(verify_slot(slot(d.path()), &j, &Control::default(), &tx, true), Slot::Done { .. }));
        assert_eq!(std::fs::read(d.path().join("x.mp4")).unwrap(), b"hello world");
        // a file that appeared in the meantime is never overwritten
        std::fs::write(d.path().join(".x.mp4.part"), b"hello world").unwrap();
        std::fs::write(d.path().join("x.mp4"), b"other").unwrap();
        assert!(matches!(verify_slot(slot(d.path()), &j, &Control::default(), &tx, true), Slot::Failed(_)));
        assert_eq!(std::fs::read(d.path().join("x.mp4")).unwrap(), b"other");
    }

    #[test]
    fn move_deletes_only_when_every_requested_copy_is_verified() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("card.mp4");
        std::fs::write(&src, b"footage!").unwrap();
        let mtime = std::fs::metadata(&src).unwrap().modified().unwrap();
        let main = d.path().join("main.mp4");
        std::fs::write(&main, b"footage!").unwrap();
        let ok = |p: &Path| Slot::Done { path: p.to_path_buf(), renamed: false };
        // mirror failed: the original must stay
        let o = outcome_for(ok(&main), Slot::Failed("disk full".into()), Mode::Move, &src, 8, mtime);
        assert!(matches!(o, Outcome::Copied { removed: Removal::Failed(_), mirror: Mirror::Failed(_), .. }), "{o:?}");
        assert!(src.exists());
        // mirror verified: now it goes
        let mir = d.path().join("mirror.mp4");
        std::fs::write(&mir, b"footage!").unwrap();
        let o = outcome_for(ok(&main), ok(&mir), Mode::Move, &src, 8, mtime);
        assert!(matches!(o, Outcome::Copied { removed: Removal::Removed, mirror: Mirror::Ok(_), .. }), "{o:?}");
        assert!(!src.exists());
    }
}

#[cfg(test)]
mod removal_tests {
    use super::*;

    struct Fx {
        _d: tempfile::TempDir,
        src: PathBuf,
        dest: PathBuf,
        mtime: SystemTime,
    }

    fn fx() -> Fx {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("card.mp4");
        let dest = d.path().join("backup.mp4");
        std::fs::write(&src, b"footage!").unwrap();
        std::fs::write(&dest, b"footage!").unwrap();
        let mtime = std::fs::metadata(&src).unwrap().modified().unwrap();
        Fx { _d: d, src, dest, mtime }
    }

    #[test]
    fn deletes_when_everything_checks_out() {
        let f = fx();
        remove_original(&f.src, 8, f.mtime, &[&f.dest]).unwrap();
        assert!(!f.src.exists());
        assert!(f.dest.exists(), "the backup is never touched");
    }

    #[test]
    fn keeps_original_if_the_copy_is_missing_or_wrong_size() {
        let f = fx();
        std::fs::remove_file(&f.dest).unwrap();
        assert!(remove_original(&f.src, 8, f.mtime, &[&f.dest]).unwrap_err().contains("no longer on the destination"));
        assert!(f.src.exists());
        std::fs::write(&f.dest, b"short").unwrap();
        assert!(remove_original(&f.src, 8, f.mtime, &[&f.dest]).unwrap_err().contains("different size"));
        assert!(f.src.exists());
    }

    #[test]
    fn keeps_original_if_it_changed_since_the_scan() {
        let f = fx();
        // different mtime
        let later = f.mtime + Duration::from_secs(5);
        assert!(remove_original(&f.src, 8, later, &[&f.dest]).unwrap_err().contains("changed"));
        // different size
        std::fs::write(&f.src, b"footage!!!").unwrap();
        assert!(remove_original(&f.src, 8, f.mtime, &[&f.dest]).unwrap_err().contains("changed"));
        assert!(f.src.exists());
    }

    #[test]
    fn never_deletes_through_a_symlink() {
        let f = fx();
        let link = f.src.with_file_name("link.mp4");
        std::os::unix::fs::symlink(&f.src, &link).unwrap();
        assert!(remove_original(&link, 8, f.mtime, &[&f.dest]).is_err());
        assert!(f.src.exists() && link.exists());
        // and a symlinked "copy" doesn't count as a backup
        let dlink = f.dest.with_file_name("dlink.mp4");
        std::os::unix::fs::symlink(&f.dest, &dlink).unwrap();
        assert!(remove_original(&f.src, 8, f.mtime, &[&dlink]).is_err());
        assert!(f.src.exists());
    }

    #[test]
    fn already_gone_is_reported_not_fatal() {
        let f = fx();
        std::fs::remove_file(&f.src).unwrap();
        assert!(remove_original(&f.src, 8, f.mtime, &[&f.dest]).unwrap_err().contains("already gone"));
    }

    #[test]
    fn delete_originals_reports_each_outcome() {
        let f = fx();
        let d2 = f.src.with_file_name("two.mp4");
        std::fs::write(&d2, b"zzzz").unwrap();
        let m2 = std::fs::metadata(&d2).unwrap().modified().unwrap();
        let list = vec![
            Original { source: f.src.clone(), size: 8, modified: f.mtime, dest: f.dest.clone(), mirror: None },
            Original { source: d2.clone(), size: 4, modified: m2, dest: f.src.with_file_name("no-copy.mp4"), mirror: None }, // copy missing
        ];
        let rep = delete_originals(&list);
        assert_eq!((rep.deleted, rep.bytes, rep.failures.len()), (1, 8, 1));
        assert!(!f.src.exists());
        assert!(d2.exists(), "the original with no verified copy survives");
        assert_eq!(rep.failures[0].0, "two.mp4");
    }

    #[test]
    fn mode_helpers() {
        assert_eq!(Mode::from_config("MOVE"), Mode::Move);
        assert_eq!(Mode::from_config("copy"), Mode::Copy);
        assert_eq!(Mode::from_config("whatever"), Mode::Copy, "unknown values never default to deleting");
        assert_eq!(Mode::Copy.toggled(), Mode::Move);
        assert_eq!(Mode::default(), Mode::Copy);
    }
}
