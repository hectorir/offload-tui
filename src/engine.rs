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

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub source: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub dest_dir: PathBuf,
    pub items: Vec<PlanItem>,
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }
}

/// Build the plan for a profile: `<dest>/<date>/<subfolder>/`.
pub fn build_plan(cfg: &Config, profile: &Profile, files: &[&FileEntry], now: chrono::DateTime<chrono::Local>) -> Result<Plan> {
    profile.validate()?;
    let date = cfg.render_date(now)?;
    crate::config::validate_component(&date)?;
    let mut dest_dir = cfg.dest_base_for(profile).join(&date);
    if !profile.subfolder.is_empty() {
        dest_dir = dest_dir.join(&profile.subfolder);
    }
    let items = files
        .iter()
        .map(|f| PlanItem {
            source: f.path.clone(),
            size: f.size,
            modified: f.modified,
            name: f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        })
        .collect();
    Ok(Plan { dest_dir, items })
}

/// Bytes that would actually need writing (files already present with the same size are assumed done;
/// the checksum decides for real during the copy).
pub fn bytes_needed(plan: &Plan) -> u64 {
    plan.items.iter().filter(|i| fs::symlink_metadata(plan.dest_dir.join(&i.name)).map(|m| m.len() != i.size).unwrap_or(true)).map(|i| i.size).sum()
}

/// Checks that must pass before any byte is written.
pub fn preflight(plan: &Plan) -> std::result::Result<(), String> {
    if plan.items.is_empty() {
        return Err("Nothing selected to copy.".into());
    }
    if !fsutil::destination_volume_present(&plan.dest_dir) {
        return Err(format!(
            "Destination drive is not mounted ({}). Plug it in and try again.",
            fsutil::volume_root(&plan.dest_dir).map(|p| p.display().to_string()).unwrap_or_default()
        ));
    }
    let needed = bytes_needed(plan);
    match fsutil::disk_space(&plan.dest_dir) {
        Some((_, free)) if free < needed.saturating_add(1 << 20) => {
            Err(format!("Not enough space on the destination: need {}, only {} free.", fsutil::human_bytes(needed), fsutil::human_bytes(free)))
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Copied and verified. `renamed` = a different file already had that name.
    Copied {
        dest: PathBuf,
        renamed: bool,
    },
    /// Identical file (same checksum) already at the destination.
    Skipped {
        dest: PathBuf,
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
    fn done(tally: &Mutex<Tally>, index: usize, name: &str, size: u64, outcome: Outcome) {
        let mut t = tally.lock().unwrap_or_else(|e| e.into_inner());
        match &outcome {
            Outcome::Copied { renamed, .. } => {
                t.summary.copied += 1;
                t.summary.bytes_copied += size;
                if *renamed {
                    t.summary.renamed += 1;
                }
            }
            Outcome::Skipped { .. } => t.summary.skipped += 1,
            Outcome::Failed(why) => {
                t.summary.failed += 1;
                t.summary.failures.push((name.to_string(), why.clone()));
            }
            Outcome::Cancelled => t.summary.cancelled = true,
        }
        let _ = t.tx.send(Event::Done { index, outcome });
    }
}

struct VerifyJob {
    index: usize,
    part: PathBuf,
    final_path: PathBuf,
    expected: blake3::Hash,
    written: u64,
    modified: SystemTime,
    renamed: bool,
    name: String,
    size: u64,
}

enum Resolved {
    Skip(PathBuf),
    Use { final_path: PathBuf, renamed: bool },
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
        // macOS volumes are usually case-insensitive; treat names case-insensitively to be safe.
        let key = cand.to_lowercase();
        if reserved.contains(&key) {
            continue;
        }
        let path = dest_dir.join(&cand);
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

fn part_path(final_path: &Path) -> PathBuf {
    let name = final_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    final_path.with_file_name(format!(".{name}.part"))
}

enum CopyErr {
    Cancelled,
    Io(String),
}

/// Stream `src` into a fresh `part` file, hashing the source bytes as they are read.
/// Returns (hash, bytes written).
fn copy_stream(src: &Path, part: &Path, ctrl: &Control, mut on_bytes: impl FnMut(u64)) -> std::result::Result<(blake3::Hash, u64), CopyErr> {
    let io = |what: &str, e: io::Error| CopyErr::Io(format!("{what}: {e}"));
    let mut input = File::open(src).map_err(|e| io("open source", e))?;
    if !input.metadata().map_err(|e| io("stat source", e))?.is_file() {
        return Err(CopyErr::Io("source is not a regular file".into()));
    }
    // A stale .part from an interrupted run is removed (never followed if it is a symlink).
    match fs::remove_file(part) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io("remove stale .part", e)),
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true) // O_EXCL: never write through a pre-existing file or symlink
        .custom_flags(libc::O_NOFOLLOW)
        .open(part)
        .map_err(|e| io("create destination", e))?;

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
            if let Err(e) = out.write_all(&buf) {
                failure = Some(io("write", e));
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
        match read {
            Ok(Ok(h)) => Ok(h),
            Ok(Err(e)) => Err(io("read source", e)),
            Err(_) => Err(CopyErr::Io("reader thread panicked".into())),
        }
    });
    match result {
        Ok(h) => Ok((h, written)),
        Err(e) => {
            drop(out);
            let _ = fs::remove_file(part);
            Err(e)
        }
    }
}

fn verify_job(job: &VerifyJob, ctrl: &Control, tx: &Sender<Event>) -> std::result::Result<(), String> {
    // Flush to the physical disk, then re-read bypassing the cache.
    File::open(&job.part).and_then(|f| fsutil::flush_best_effort(&f)).map_err(|e| format!("flush: {e}"))?;
    let index = job.index;
    let (got, len) = fsutil::hash_file(&job.part, true, |b| {
        let _ = tx.send(Event::VerifyProgress { index, bytes: b });
        !ctrl.is_cancelled()
    })
    .map_err(|e| if e.kind() == io::ErrorKind::Interrupted { "cancelled".to_string() } else { format!("verify read: {e}") })?;
    if len != job.written || got != job.expected {
        return Err("checksum mismatch: the copy on disk does not match the source".into());
    }
    if fs::symlink_metadata(&job.final_path).is_ok() {
        return Err("a file appeared at the destination during the copy; not overwriting".into());
    }
    fs::rename(&job.part, &job.final_path).map_err(|e| format!("finalize: {e}"))?;
    // Preserve the camera's modification time (best effort).
    if let Ok(f) = File::options().read(true).open(&job.final_path) {
        let _ = f.set_modified(job.modified);
    }
    Ok(())
}

/// Run the plan to completion (blocking). Progress is reported through `tx`.
pub fn run(plan: &Plan, ctrl: &Arc<Control>, tx: Sender<Event>) -> Summary {
    let start = Instant::now();
    let tally = Arc::new(Mutex::new(Tally { summary: Summary { dest_dir: plan.dest_dir.clone(), ..Default::default() }, tx: tx.clone() }));

    if let Err(e) = prepare_dest_dir(&plan.dest_dir) {
        for (i, item) in plan.items.iter().enumerate() {
            Tally::done(&tally, i, &item.name, item.size, Outcome::Failed(format!("cannot create destination: {e}")));
        }
        return finish(tally, start, &tx, &plan.dest_dir);
    }

    let (vtx, vrx): (_, Receiver<VerifyJob>) = sync_channel(8);
    std::thread::scope(|s| {
        let vt = tally.clone();
        let vctrl = ctrl.clone();
        let vevents = tx.clone();
        s.spawn(move || {
            while let Ok(job) = vrx.recv() {
                let _ = vevents.send(Event::Verifying { index: job.index });
                if vctrl.is_cancelled() {
                    let _ = fs::remove_file(&job.part);
                    Tally::done(&vt, job.index, &job.name, job.size, Outcome::Cancelled);
                    continue;
                }
                match verify_job(&job, &vctrl, &vevents) {
                    Ok(()) => Tally::done(&vt, job.index, &job.name, job.size, Outcome::Copied { dest: job.final_path.clone(), renamed: job.renamed }),
                    Err(e) if e == "cancelled" => {
                        let _ = fs::remove_file(&job.part);
                        Tally::done(&vt, job.index, &job.name, job.size, Outcome::Cancelled);
                    }
                    Err(e) => {
                        let _ = fs::remove_file(&job.part);
                        Tally::done(&vt, job.index, &job.name, job.size, Outcome::Failed(e));
                    }
                }
            }
        });

        let mut reserved = HashSet::new();
        for (index, item) in plan.items.iter().enumerate() {
            if ctrl.is_cancelled() || !ctrl.checkpoint() {
                Tally::done(&tally, index, &item.name, item.size, Outcome::Cancelled);
                continue;
            }
            let _ = tx.send(Event::Started { index });
            let (final_path, renamed) = match resolve_dest(&plan.dest_dir, item, &mut reserved) {
                Ok(Resolved::Skip(dest)) => {
                    let _ = tx.send(Event::Progress { index, bytes: item.size });
                    let _ = tx.send(Event::VerifyProgress { index, bytes: item.size });
                    Tally::done(&tally, index, &item.name, item.size, Outcome::Skipped { dest });
                    continue;
                }
                Ok(Resolved::Use { final_path, renamed }) => (final_path, renamed),
                Err(e) => {
                    Tally::done(&tally, index, &item.name, item.size, Outcome::Failed(format!("checking destination: {e}")));
                    continue;
                }
            };
            let part = part_path(&final_path);
            match copy_stream(&item.source, &part, ctrl, |b| {
                let _ = tx.send(Event::Progress { index, bytes: b });
            }) {
                Ok((hash, written)) => {
                    let job = VerifyJob {
                        index,
                        part,
                        final_path,
                        expected: hash,
                        written,
                        modified: item.modified,
                        renamed,
                        name: item.name.clone(),
                        size: item.size,
                    };
                    if vtx.send(job).is_err() {
                        Tally::done(&tally, index, &item.name, item.size, Outcome::Failed("verifier stopped unexpectedly".into()));
                    }
                }
                Err(CopyErr::Cancelled) => Tally::done(&tally, index, &item.name, item.size, Outcome::Cancelled),
                Err(CopyErr::Io(e)) => Tally::done(&tally, index, &item.name, item.size, Outcome::Failed(e)),
            }
        }
        drop(vtx); // lets the verifier finish its queue and exit
    });

    // Persist the directory entries of the finalized files.
    if let Ok(d) = File::open(&plan.dest_dir) {
        let _ = fsutil::flush_best_effort(&d);
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

    fn job(dir: &Path, expected: blake3::Hash, written: u64) -> VerifyJob {
        VerifyJob {
            index: 0,
            part: dir.join(".x.mp4.part"),
            final_path: dir.join("x.mp4"),
            expected,
            written,
            modified: SystemTime::now(),
            renamed: false,
            name: "x.mp4".into(),
            size: written,
        }
    }

    #[test]
    fn corrupted_copy_is_rejected_and_never_finalized() {
        let d = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let ctrl = Control::default();
        std::fs::write(d.path().join(".x.mp4.part"), b"hello w0rld").unwrap(); // one flipped byte
        let j = job(d.path(), blake3::hash(b"hello world"), 11);
        let err = verify_job(&j, &ctrl, &tx).unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(!d.path().join("x.mp4").exists());
    }

    #[test]
    fn truncated_copy_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        std::fs::write(d.path().join(".x.mp4.part"), b"hello").unwrap();
        let j = job(d.path(), blake3::hash(b"hello world"), 11);
        assert!(verify_job(&j, &Control::default(), &tx).is_err());
        assert!(!d.path().join("x.mp4").exists());
    }

    #[test]
    fn good_copy_is_finalized_and_does_not_clobber() {
        let d = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        std::fs::write(d.path().join(".x.mp4.part"), b"hello world").unwrap();
        let j = job(d.path(), blake3::hash(b"hello world"), 11);
        verify_job(&j, &Control::default(), &tx).unwrap();
        assert_eq!(std::fs::read(d.path().join("x.mp4")).unwrap(), b"hello world");
        // a file that appeared in the meantime is never overwritten
        std::fs::write(d.path().join(".x.mp4.part"), b"hello world").unwrap();
        std::fs::write(d.path().join("x.mp4"), b"other").unwrap();
        assert!(verify_job(&j, &Control::default(), &tx).is_err());
        assert_eq!(std::fs::read(d.path().join("x.mp4")).unwrap(), b"other");
    }
}
