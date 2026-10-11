#![allow(clippy::field_reassign_with_default)]
use offload::config::Config;
use offload::engine::{self, Control, Event, Outcome, Plan, Summary};
use offload::scan::scan_profile;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

struct Rig {
    _tmp: tempfile::TempDir,
    cfg: Config,
    src: PathBuf,
    dest: PathBuf,
}

fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card/DCIM");
    let dest = tmp.path().join("backup");
    fs::create_dir_all(&src).unwrap();
    let mut cfg = Config::default();
    cfg.destination = dest.to_string_lossy().into();
    cfg.profiles[0].sources = vec![src.to_string_lossy().into()];
    Rig { _tmp: tmp, cfg, src, dest }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

fn plan_for(r: &Rig) -> Plan {
    let p = &r.cfg.profiles[0];
    let scan = scan_profile(p);
    let refs: Vec<_> = scan.files.iter().collect();
    engine::build_plan(&r.cfg, p, &refs, chrono::Local::now()).unwrap()
}

fn run(plan: &Plan) -> (Summary, Vec<Event>) {
    let (tx, rx) = mpsc::channel();
    let ctrl = Control::new();
    let s = engine::run(plan, &ctrl, tx);
    (s, rx.try_iter().collect())
}

/// Point a whole plan at a different folder.
fn retarget(plan: &mut Plan, dir: &str) {
    plan.dest_dir = PathBuf::from(dir);
    for it in &mut plan.items {
        it.dest_dir = PathBuf::from(dir);
    }
}

fn list(dir: &Path) -> Vec<String> {
    let mut v: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn copies_filters_and_verifies_everything() {
    let r = rig();
    let big = pattern(5 * 1024 * 1024 + 123, 7); // spans several chunks
    fs::write(r.src.join("clip.MP4"), &big).unwrap();
    fs::write(r.src.join("audio.wav"), pattern(1000, 1)).unwrap();
    fs::write(r.src.join("proxy.LRF"), pattern(500, 2)).unwrap();
    fs::write(r.src.join("photo.jpg"), pattern(500, 3)).unwrap();
    fs::write(r.src.join("empty.mp4"), b"").unwrap();
    let plan = plan_for(&r);
    assert_eq!(plan.items.len(), 3);
    assert!(plan.dest_dir.ends_with(format!("{}/DJI", chrono::Local::now().format("%Y-%m-%d"))));

    let (s, ev) = run(&plan);
    assert_eq!((s.copied, s.skipped, s.failed, s.renamed), (3, 0, 0, 0), "{:?}", s.failures);
    assert!(!s.cancelled);
    assert_eq!(list(&plan.dest_dir), ["audio.wav", "clip.MP4", "empty.mp4"], "no LRF, no jpg, no .part leftovers");
    assert_eq!(fs::read(plan.dest_dir.join("clip.MP4")).unwrap(), big);
    assert_eq!(fs::read(plan.dest_dir.join("empty.mp4")).unwrap().len(), 0);

    // mtime preserved
    let sm = fs::metadata(r.src.join("clip.MP4")).unwrap().modified().unwrap();
    let dm = fs::metadata(plan.dest_dir.join("clip.MP4")).unwrap().modified().unwrap();
    assert_eq!(sm, dm);

    // progress accounting matches the plan exactly
    let copied: u64 = ev.iter().filter_map(|e| if let Event::Progress { bytes, .. } = e { Some(*bytes) } else { None }).sum();
    let verified: u64 = ev.iter().filter_map(|e| if let Event::VerifyProgress { bytes, .. } = e { Some(*bytes) } else { None }).sum();
    assert_eq!(copied, plan.total_bytes());
    assert_eq!(verified, plan.total_bytes());
    assert!(matches!(ev.last(), Some(Event::Finished(_))));
    assert_eq!(ev.iter().filter(|e| matches!(e, Event::Done { .. })).count(), 3);
}

#[test]
fn source_files_are_never_modified_or_removed() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(4096, 9)).unwrap();
    let before = fs::read(r.src.join("a.mp4")).unwrap();
    let mt = fs::metadata(r.src.join("a.mp4")).unwrap().modified().unwrap();
    run(&plan_for(&r));
    assert_eq!(fs::read(r.src.join("a.mp4")).unwrap(), before);
    assert_eq!(fs::metadata(r.src.join("a.mp4")).unwrap().modified().unwrap(), mt);
}

#[test]
fn rerun_skips_identical_files_by_checksum() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(300_000, 1)).unwrap();
    fs::write(r.src.join("b.wav"), pattern(200_000, 2)).unwrap();
    let plan = plan_for(&r);
    let (s1, _) = run(&plan);
    assert_eq!(s1.copied, 2);
    let (s2, ev) = run(&plan);
    assert_eq!((s2.copied, s2.skipped, s2.failed), (0, 2, 0));
    assert_eq!(list(&plan.dest_dir), ["a.mp4", "b.wav"], "no duplicates created");
    assert!(ev.iter().any(|e| matches!(e, Event::Done { outcome: Outcome::Skipped { .. }, .. })));
}

#[test]
fn same_name_different_content_keeps_both_and_never_overwrites() {
    let r = rig();
    fs::write(r.src.join("C0001.MP4"), pattern(10_000, 5)).unwrap();
    let plan = plan_for(&r);
    fs::create_dir_all(&plan.dest_dir).unwrap();
    let precious = pattern(10_000, 99); // same size, different bytes
    fs::write(plan.dest_dir.join("C0001.MP4"), &precious).unwrap();

    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.renamed, s.failed), (1, 1, 0));
    assert_eq!(fs::read(plan.dest_dir.join("C0001.MP4")).unwrap(), precious, "existing file untouched");
    assert_eq!(fs::read(plan.dest_dir.join("C0001_1.MP4")).unwrap(), pattern(10_000, 5));

    // Running again recognizes the renamed copy as already done.
    let (s2, _) = run(&plan);
    assert_eq!((s2.copied, s2.skipped), (0, 1));
    assert_eq!(list(&plan.dest_dir), ["C0001.MP4", "C0001_1.MP4"]);
}

#[test]
fn colliding_names_from_different_folders_both_survive() {
    let r = rig();
    fs::create_dir_all(r.src.join("100")).unwrap();
    fs::create_dir_all(r.src.join("200")).unwrap();
    fs::create_dir_all(r.src.join("300")).unwrap();
    fs::write(r.src.join("100/DJI_0001.MP4"), pattern(1000, 1)).unwrap();
    fs::write(r.src.join("200/DJI_0001.MP4"), pattern(1000, 2)).unwrap();
    fs::write(r.src.join("300/dji_0001.mp4"), pattern(1000, 3)).unwrap(); // differs only by case
    let (s, _) = run(&plan_for(&r));
    assert_eq!((s.copied, s.failed), (3, 0), "{:?}", s.failures);
    let plan = plan_for(&r);
    let names = list(&plan.dest_dir);
    assert_eq!(names.len(), 3);
    let mut contents: Vec<_> = names.iter().map(|n| fs::read(plan.dest_dir.join(n)).unwrap()).collect();
    contents.sort();
    let mut want = vec![pattern(1000, 1), pattern(1000, 2), pattern(1000, 3)];
    want.sort();
    assert_eq!(contents, want, "every distinct source file is preserved");
}

#[test]
fn stale_part_files_from_a_crash_are_replaced() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(50_000, 4)).unwrap();
    let plan = plan_for(&r);
    fs::create_dir_all(&plan.dest_dir).unwrap();
    fs::write(plan.dest_dir.join(".a.mp4.part"), b"half-written garbage").unwrap();
    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.failed), (1, 0));
    assert_eq!(list(&plan.dest_dir), ["a.mp4"]);
    assert_eq!(fs::read(plan.dest_dir.join("a.mp4")).unwrap(), pattern(50_000, 4));
}

#[test]
fn corrupt_partial_final_file_is_not_trusted() {
    // A truncated file at the final name (e.g. from another tool) must not be treated as "already copied".
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(50_000, 4)).unwrap();
    let plan = plan_for(&r);
    fs::create_dir_all(&plan.dest_dir).unwrap();
    fs::write(plan.dest_dir.join("a.mp4"), &pattern(50_000, 4)[..30_000]).unwrap();
    let (s, _) = run(&plan);
    assert_eq!((s.skipped, s.copied, s.renamed), (0, 1, 1));
    assert_eq!(fs::read(plan.dest_dir.join("a_1.mp4")).unwrap(), pattern(50_000, 4));
}

#[test]
fn cancel_before_start_writes_nothing() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(1000, 1)).unwrap();
    fs::write(r.src.join("b.mp4"), pattern(1000, 2)).unwrap();
    let plan = plan_for(&r);
    let (tx, _rx) = mpsc::channel();
    let ctrl = Control::new();
    ctrl.cancel();
    let s = engine::run(&plan, &ctrl, tx);
    assert!(s.cancelled);
    assert_eq!(s.copied, 0);
    assert!(list(&plan.dest_dir).is_empty());
}

#[test]
fn cancel_mid_copy_leaves_no_partial_or_unverified_files() {
    let r = rig();
    for i in 0..6u8 {
        fs::write(r.src.join(format!("f{i}.mp4")), pattern(8 * 1024 * 1024, i)).unwrap();
    }
    let plan = plan_for(&r);
    let (tx, rx) = mpsc::channel();
    let ctrl = Control::new();
    let c2 = ctrl.clone();
    let watcher = std::thread::spawn(move || {
        let mut n = 0;
        let mut evs = vec![];
        while let Ok(e) = rx.recv() {
            if matches!(e, Event::Progress { .. }) {
                n += 1;
                if n == 3 {
                    c2.cancel();
                }
            }
            let fin = matches!(e, Event::Finished(_));
            evs.push(e);
            if fin {
                break;
            }
        }
        evs
    });
    let s = engine::run(&plan, &ctrl, tx);
    watcher.join().unwrap();
    assert!(s.cancelled);
    for name in list(&plan.dest_dir) {
        assert!(!name.ends_with(".part"), "leftover partial: {name}");
        // any file that exists under a real name must be complete and correct
        let i: u8 = name[1..2].parse().unwrap();
        assert_eq!(fs::read(plan.dest_dir.join(&name)).unwrap(), pattern(8 * 1024 * 1024, i), "{name}");
    }
    assert!(s.copied < 6);
}

#[test]
fn pause_blocks_progress_until_resumed() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(6 * 1024 * 1024, 1)).unwrap();
    let plan = plan_for(&r);
    let (tx, rx) = mpsc::channel();
    let ctrl = Control::new();
    ctrl.set_paused(true);
    let c2 = ctrl.clone();
    let h = std::thread::spawn(move || engine::run(&plan, &c2, tx));
    std::thread::sleep(std::time::Duration::from_millis(400));
    let progressed: u64 = rx.try_iter().filter_map(|e| if let Event::Progress { bytes, .. } = e { Some(bytes) } else { None }).sum();
    assert_eq!(progressed, 0, "no bytes may move while paused");
    ctrl.set_paused(false);
    let s = h.join().unwrap();
    assert_eq!((s.copied, s.failed), (1, 0));
}

#[test]
fn space_guard_and_missing_volume() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(1000, 1)).unwrap();
    let mut plan = plan_for(&r);
    assert!(engine::preflight(&plan).is_ok());

    plan.items[0].size = u64::MAX / 4; // absurdly large
    let err = engine::preflight(&plan).unwrap_err();
    assert!(err.contains("Not enough space"), "{err}");

    let mut missing = plan_for(&r);
    retarget(&mut missing, "/Volumes/definitely-not-mounted-zz/Backup/2026-01-01/DJI");
    let err = engine::preflight(&missing).unwrap_err();
    assert!(err.contains("not mounted"), "{err}");

    let empty = Plan { dest_dir: plan.dest_dir.clone(), items: vec![], mode: engine::Mode::Copy };
    assert!(engine::preflight(&empty).is_err());
}

#[test]
fn unmounted_destination_volume_creates_nothing() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(1000, 1)).unwrap();
    let mut plan = plan_for(&r);
    retarget(&mut plan, "/Volumes/definitely-not-mounted-zz/Backup/2026-01-01/DJI");
    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.failed), (0, 1));
    assert!(!Path::new("/Volumes/definitely-not-mounted-zz").exists());
}

#[test]
fn refuses_to_write_through_a_symlinked_destination() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(1000, 1)).unwrap();
    let plan = plan_for(&r);
    let elsewhere = r._tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(plan.dest_dir.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &plan.dest_dir).unwrap();
    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.failed), (0, 1));
    assert!(list(&elsewhere).is_empty(), "nothing may leak through the symlink");
}

#[test]
fn symlinked_part_file_is_not_followed() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(1000, 1)).unwrap();
    let plan = plan_for(&r);
    fs::create_dir_all(&plan.dest_dir).unwrap();
    let victim = r._tmp.path().join("victim.txt");
    fs::write(&victim, b"keep me").unwrap();
    std::os::unix::fs::symlink(&victim, plan.dest_dir.join(".a.mp4.part")).unwrap();
    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.failed), (1, 0));
    assert_eq!(fs::read(&victim).unwrap(), b"keep me");
}

#[test]
fn unreadable_source_fails_only_that_file() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig();
    fs::write(r.src.join("good.mp4"), pattern(1000, 1)).unwrap();
    fs::write(r.src.join("bad.mp4"), pattern(1000, 2)).unwrap();
    let plan = plan_for(&r);
    fs::set_permissions(r.src.join("bad.mp4"), fs::Permissions::from_mode(0o000)).unwrap();
    let (s, _) = run(&plan);
    fs::set_permissions(r.src.join("bad.mp4"), fs::Permissions::from_mode(0o644)).unwrap();
    if unsafe { libc_geteuid() } == 0 {
        return; // root ignores permissions
    }
    assert_eq!((s.copied, s.failed), (1, 1));
    assert_eq!(s.failures[0].0, "bad.mp4");
    assert_eq!(list(&plan.dest_dir), ["good.mp4"]);
}

unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

#[test]
fn custom_date_and_subfolder_layout() {
    let mut r = rig();
    r.cfg.date_format = "Shoot %d-%m-%Y".into();
    r.cfg.profiles[0].subfolder = String::new();
    fs::write(r.src.join("a.mp4"), pattern(10, 1)).unwrap();
    let plan = plan_for(&r);
    let want = r.dest.join(chrono::Local::now().format("Shoot %d-%m-%Y").to_string());
    assert_eq!(plan.dest_dir, want);
}

// ---------- batches: several cameras in one run ----------

fn batch_rig() -> (Rig, PathBuf, PathBuf) {
    let mut r = rig();
    let sony = r._tmp.path().join("sony/DCIM");
    let clip = r._tmp.path().join("sony/CLIP");
    fs::create_dir_all(&sony).unwrap();
    fs::create_dir_all(&clip).unwrap();
    r.cfg.profiles[1].sources = vec![sony.to_string_lossy().into(), clip.to_string_lossy().into()];
    (r, sony, clip)
}

fn batch_plan(r: &Rig) -> Plan {
    let scans: Vec<_> = (0..2).map(|i| (i, scan_profile(&r.cfg.profiles[i]))).collect();
    let merged = offload::scan::merge_scans(scans);
    let parts: Vec<engine::BatchPart> = (0..2)
        .map(|i| engine::BatchPart { profile_index: i, profile: &r.cfg.profiles[i], files: merged.files.iter().filter(|f| f.profile == i).collect() })
        .collect();
    engine::build_batch_plan(&r.cfg, &parts, chrono::Local::now()).unwrap()
}

#[test]
fn batch_copies_each_camera_into_its_own_subfolder() {
    let (r, sony, clip) = batch_rig();
    fs::write(r.src.join("DJI_0001.MP4"), pattern(70_000, 1)).unwrap();
    fs::write(r.src.join("DJI_0001.WAV"), pattern(9_000, 2)).unwrap();
    fs::write(r.src.join("DJI_0001.LRF"), pattern(100, 3)).unwrap();
    fs::write(sony.join("DSC0001.ARW"), pattern(40_000, 4)).unwrap();
    fs::write(sony.join("DSC0002.JPG"), pattern(40_000, 5)).unwrap(); // not wanted by the Sony profile
    fs::write(clip.join("C0001.MP4"), pattern(120_000, 6)).unwrap();
    // identical file names on both cameras must not collide: they go to different folders
    fs::write(r.src.join("C0001.MP4"), pattern(500, 7)).unwrap();

    let plan = batch_plan(&r);
    assert_eq!(plan.items.len(), 5, "DJI: mp4+wav+C0001.MP4, Sony: arw+clip");
    assert_eq!(plan.dest_dirs().len(), 2);
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert_eq!(plan.dest_dir, r.dest.join(&day), "batch root is the shared dated folder");
    engine::preflight(&plan).unwrap();

    let (s, ev) = run(&plan);
    assert_eq!((s.copied, s.failed, s.renamed), (5, 0, 0), "{:?}", s.failures);
    let dji = r.dest.join(&day).join("DJI");
    let son = r.dest.join(&day).join("Sony");
    assert_eq!(list(&dji), ["C0001.MP4", "DJI_0001.MP4", "DJI_0001.WAV"]);
    assert_eq!(list(&son), ["C0001.MP4", "DSC0001.ARW"]);
    assert_eq!(fs::read(dji.join("C0001.MP4")).unwrap(), pattern(500, 7));
    assert_eq!(fs::read(son.join("C0001.MP4")).unwrap(), pattern(120_000, 6));
    assert_eq!(fs::read(son.join("DSC0001.ARW")).unwrap(), pattern(40_000, 4));
    // every event is attributed to a valid plan index, and totals add up
    let done = ev.iter().filter(|e| matches!(e, Event::Done { .. })).count();
    assert_eq!(done, 5);
    let copied: u64 = ev.iter().filter_map(|e| if let Event::Progress { bytes, .. } = e { Some(*bytes) } else { None }).sum();
    assert_eq!(copied, plan.total_bytes());

    // re-running the batch skips everything
    let (s2, _) = run(&plan);
    assert_eq!((s2.copied, s2.skipped), (0, 5));
}

#[test]
fn batch_failure_in_one_cameras_folder_does_not_stop_the_other() {
    let (r, sony, _clip) = batch_rig();
    fs::write(r.src.join("a.mp4"), pattern(1000, 1)).unwrap();
    fs::write(sony.join("b.arw"), pattern(1000, 2)).unwrap();
    let plan = batch_plan(&r);
    // Sabotage the DJI folder: make it a symlink to somewhere else.
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let elsewhere = r._tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(r.dest.join(&day)).unwrap();
    std::os::unix::fs::symlink(&elsewhere, r.dest.join(&day).join("DJI")).unwrap();

    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.failed), (1, 1), "{:?}", s.failures);
    assert!(list(&elsewhere).is_empty(), "nothing leaks through the symlink");
    assert_eq!(list(&r.dest.join(&day).join("Sony")), ["b.arw"]);
}

#[test]
fn batch_preflight_blocks_missing_drive_and_empty_batches() {
    let (r, sony, _) = batch_rig();
    fs::write(r.src.join("a.mp4"), pattern(10, 1)).unwrap();
    fs::write(sony.join("b.arw"), pattern(10, 2)).unwrap();
    let mut plan = batch_plan(&r);
    retarget(&mut plan, "/Volumes/definitely-not-mounted-zz/x");
    assert!(engine::preflight(&plan).unwrap_err().contains("not mounted"));
    assert!(engine::build_batch_plan(&r.cfg, &[], chrono::Local::now()).is_err());
}

#[test]
fn merge_scans_tags_files_and_sources_by_camera() {
    let (r, sony, clip) = batch_rig();
    fs::write(r.src.join("a.mp4"), b"1").unwrap();
    fs::write(sony.join("b.arw"), b"22").unwrap();
    fs::write(clip.join("c.mp4"), b"333").unwrap();
    let m = offload::scan::merge_scans((0..2).map(|i| (i, scan_profile(&r.cfg.profiles[i]))).collect());
    assert_eq!(m.files.len(), 3);
    assert_eq!(m.files.iter().filter(|f| f.profile == 0).count(), 1);
    assert_eq!(m.files.iter().filter(|f| f.profile == 1).count(), 2);
    for f in &m.files {
        assert_eq!(m.sources[f.source].profile, f.profile, "source index must point at the owning camera's source");
        assert!(f.path.starts_with(&m.sources[f.source].path));
    }
    assert_eq!(m.sources.len(), r.cfg.profiles[0].sources.len() + 2);
}

// ---------- move mode: originals are deleted only once their copy is verified ----------

fn run_mode(plan: &Plan, mode: engine::Mode) -> (Summary, Vec<Event>) {
    let mut p = plan.clone();
    p.mode = mode;
    run(&p)
}

#[test]
fn move_deletes_each_original_after_verification_and_keeps_the_backup() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(3_000_000, 1)).unwrap();
    fs::write(r.src.join("b.wav"), pattern(10_000, 2)).unwrap();
    fs::write(r.src.join("keep.LRF"), b"not selected").unwrap(); // never in the plan -> never touched
    fs::write(r.src.join("photo.jpg"), b"not wanted").unwrap();
    let plan = plan_for(&r);
    let (s, ev) = run_mode(&plan, engine::Mode::Move);
    assert_eq!((s.copied, s.failed, s.moved), (2, 0, 2), "{:?} {:?}", s.failures, s.move_failures);
    assert!(!r.src.join("a.mp4").exists() && !r.src.join("b.wav").exists(), "originals deleted after verification");
    assert!(r.src.join("keep.LRF").exists() && r.src.join("photo.jpg").exists(), "files outside the plan are never touched");
    assert_eq!(fs::read(plan.dest_dir.join("a.mp4")).unwrap(), pattern(3_000_000, 1));
    assert_eq!(fs::read(plan.dest_dir.join("b.wav")).unwrap(), pattern(10_000, 2));
    assert!(ev.iter().filter(|e| matches!(e, Event::Done { outcome: Outcome::Copied { removed: engine::Removal::Removed, .. }, .. })).count() == 2);
}

#[test]
fn copy_mode_never_deletes_anything() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(5000, 1)).unwrap();
    let plan = plan_for(&r);
    let (s, _) = run_mode(&plan, engine::Mode::Copy);
    assert_eq!((s.copied, s.moved), (1, 0));
    assert!(r.src.join("a.mp4").exists());
}

#[test]
fn move_keeps_the_original_when_the_copy_fails() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(5000, 1)).unwrap();
    let plan = plan_for(&r);
    // sabotage: the destination folder is a symlink, so the copy is refused
    let elsewhere = r._tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(plan.dest_dir.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &plan.dest_dir).unwrap();
    let (s, _) = run_mode(&plan, engine::Mode::Move);
    assert_eq!((s.copied, s.failed, s.moved), (0, 1, 0));
    assert!(r.src.join("a.mp4").exists(), "a failed copy must never cost you the original");
}

#[test]
fn move_keeps_the_original_when_cancelled() {
    let r = rig();
    for i in 0..4u8 {
        fs::write(r.src.join(format!("f{i}.mp4")), pattern(6 * 1024 * 1024, i)).unwrap();
    }
    let mut plan = plan_for(&r);
    plan.mode = engine::Mode::Move;
    let (tx, rx) = mpsc::channel();
    let ctrl = Control::new();
    let c2 = ctrl.clone();
    let watcher = std::thread::spawn(move || {
        let mut n = 0;
        while let Ok(e) = rx.recv() {
            if matches!(e, Event::Progress { .. }) {
                n += 1;
                if n == 2 {
                    c2.cancel();
                }
            }
            if matches!(e, Event::Finished(_)) {
                break;
            }
        }
    });
    let s = engine::run(&plan, &ctrl, tx);
    watcher.join().unwrap();
    assert!(s.cancelled);
    // every original is either safely backed up (and then may be gone) or still on the card
    for i in 0..4u8 {
        let name = format!("f{i}.mp4");
        let orig = r.src.join(&name);
        let backup = plan.dest_dir.join(&name);
        if backup.exists() {
            assert_eq!(fs::read(&backup).unwrap(), pattern(6 * 1024 * 1024, i), "{name}: anything at its real name is complete");
        } else {
            assert!(orig.exists(), "{name}: not backed up, so the original must still exist");
        }
    }
}

#[test]
fn move_keeps_the_original_if_it_changed_after_the_scan() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(5000, 1)).unwrap();
    let plan = plan_for(&r);
    // the camera/user touches the file after scanning but before the copy
    fs::write(r.src.join("a.mp4"), pattern(5000, 9)).unwrap();
    let (s, _) = run_mode(&plan, engine::Mode::Move);
    assert_eq!((s.copied, s.moved), (1, 0), "{:?}", s.failures);
    assert_eq!(s.move_failures.len(), 1, "the copy is safe but the original is kept and reported");
    assert!(s.move_failures[0].1.contains("changed"));
    assert!(r.src.join("a.mp4").exists());
}

#[test]
fn move_of_an_already_backed_up_file_deletes_the_original_only_after_checksum_match() {
    let r = rig();
    fs::write(r.src.join("a.mp4"), pattern(20_000, 1)).unwrap();
    fs::write(r.src.join("b.mp4"), pattern(20_000, 2)).unwrap();
    let plan = plan_for(&r);
    run_mode(&plan, engine::Mode::Copy); // first, back both up
    // b's backup gets silently corrupted (same size, different bytes)
    fs::write(plan.dest_dir.join("b.mp4"), pattern(20_000, 77)).unwrap();
    let (s, _) = run_mode(&plan, engine::Mode::Move);
    assert_eq!(s.skipped, 1, "a.mp4 matches by checksum");
    assert_eq!(s.copied, 1, "b.mp4's backup is corrupt, so it is copied again (renamed), not trusted");
    assert!(!r.src.join("a.mp4").exists(), "identical backup confirmed -> original removed");
    assert!(!r.src.join("b.mp4").exists(), "fresh verified copy exists -> original removed");
    assert_eq!(fs::read(plan.dest_dir.join("b_1.mp4")).unwrap(), pattern(20_000, 2));
}

#[test]
fn batch_move_deletes_from_each_card() {
    let (r, sony, _clip) = batch_rig();
    fs::write(r.src.join("a.mp4"), pattern(1000, 1)).unwrap();
    fs::write(sony.join("b.arw"), pattern(1000, 2)).unwrap();
    let plan = batch_plan(&r);
    let (s, _) = run_mode(&plan, engine::Mode::Move);
    assert_eq!((s.copied, s.moved, s.failed), (2, 2, 0));
    assert!(!r.src.join("a.mp4").exists() && !sony.join("b.arw").exists());
}

// ---------- second drive (mirror) and shoot-date folders ----------

fn with_mirror(r: &mut Rig) -> PathBuf {
    let m = r._tmp.path().join("mirror-drive");
    r.cfg.mirror = m.to_string_lossy().into();
    m
}

fn set_mtime(path: &Path, when: std::time::SystemTime) {
    fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

#[test]
fn mirror_writes_and_verifies_both_copies_from_one_read() {
    let mut r = rig();
    let m = with_mirror(&mut r);
    let big = pattern(3 * 1024 * 1024 + 17, 9);
    fs::write(r.src.join("a.mp4"), &big).unwrap();
    fs::write(r.src.join("b.wav"), pattern(2_000, 4)).unwrap();
    let plan = plan_for(&r);
    assert!(plan.is_mirrored());
    let (s, ev) = run(&plan);
    assert_eq!((s.copied, s.failed, s.mirrored), (2, 0, 2), "{:?} {:?}", s.failures, s.mirror_failures);
    let day = plan.dest_dir.strip_prefix(&r.dest).unwrap().to_path_buf();
    assert_eq!(fs::read(plan.dest_dir.join("a.mp4")).unwrap(), big);
    assert_eq!(fs::read(m.join(&day).join("a.mp4")).unwrap(), big, "the mirror is byte-identical");
    assert_eq!(list(&m.join(&day)), ["a.mp4", "b.wav"], "no .part leftovers on the mirror");
    assert!(ev.iter().any(|e| matches!(e, Event::Done { outcome: Outcome::Copied { mirror: engine::Mirror::Ok(_), .. }, .. })));
    // progress still adds up to the plan size once, not twice
    let copied: u64 = ev.iter().filter_map(|e| if let Event::Progress { bytes, .. } = e { Some(*bytes) } else { None }).sum();
    assert_eq!(copied, plan.total_bytes());
}

#[test]
fn a_failed_mirror_never_fails_the_main_copy_and_blocks_move() {
    let mut r = rig();
    let m = with_mirror(&mut r);
    fs::write(r.src.join("a.mp4"), pattern(10_000, 1)).unwrap();
    let mut plan = plan_for(&r);
    // Make the mirror folder impossible to create: a plain file sits where its parent should be.
    fs::write(&m, b"not a directory").unwrap();
    plan.mode = engine::Mode::Move;
    let (s, ev) = run(&plan);
    assert_eq!((s.copied, s.failed, s.mirrored), (1, 0, 0));
    assert_eq!(s.mirror_failures.len(), 1);
    assert!(plan.dest_dir.join("a.mp4").exists(), "the main copy is fine");
    assert!(r.src.join("a.mp4").exists(), "move must not delete the original when the mirror failed");
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Done { outcome: Outcome::Copied { removed: engine::Removal::Failed(_), mirror: engine::Mirror::Failed(_), .. }, .. }))
    );
}

#[test]
fn move_with_a_mirror_deletes_the_original_only_after_both_verify() {
    let mut r = rig();
    let m = with_mirror(&mut r);
    fs::write(r.src.join("a.mp4"), pattern(10_000, 1)).unwrap();
    let mut plan = plan_for(&r);
    plan.mode = engine::Mode::Move;
    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.moved, s.mirrored), (1, 1, 1));
    assert!(!r.src.join("a.mp4").exists());
    let day = plan.dest_dir.strip_prefix(&r.dest).unwrap().to_path_buf();
    assert!(plan.dest_dir.join("a.mp4").exists() && m.join(day).join("a.mp4").exists());
}

#[test]
fn rerun_with_a_new_mirror_only_fills_the_mirror() {
    let mut r = rig();
    fs::write(r.src.join("a.mp4"), pattern(20_000, 2)).unwrap();
    let (s, _) = run(&plan_for(&r)); // first without a mirror
    assert_eq!(s.copied, 1);
    let m = with_mirror(&mut r);
    let plan = plan_for(&r);
    let (s, ev) = run(&plan);
    assert_eq!((s.copied, s.skipped, s.mirrored), (0, 1, 1), "main copy is skipped by checksum, the mirror is filled");
    let day = plan.dest_dir.strip_prefix(&r.dest).unwrap().to_path_buf();
    assert_eq!(fs::read(m.join(day).join("a.mp4")).unwrap(), pattern(20_000, 2));
    assert!(ev.iter().any(|e| matches!(e, Event::Done { outcome: Outcome::Skipped { mirror: engine::Mirror::Ok(_), .. }, .. })));
}

#[test]
fn preflight_blocks_a_missing_mirror_drive() {
    let mut r = rig();
    fs::write(r.src.join("a.mp4"), b"x").unwrap();
    r.cfg.mirror = "/Volumes/definitely-not-mounted-zz/backup".into();
    let plan = plan_for(&r);
    let err = engine::preflight(&plan).unwrap_err();
    assert!(err.contains("mirror") && err.contains("not mounted"), "{err}");
    assert!(!Path::new("/Volumes/definitely-not-mounted-zz").exists(), "nothing was created");
}

#[test]
fn shoot_date_mode_makes_a_folder_per_day_filmed() {
    let mut r = rig();
    r.cfg.organize = "shoot".into();
    let day1 = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86_400);
    let day2 = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86_400);
    fs::write(r.src.join("old.mp4"), pattern(500, 1)).unwrap();
    fs::write(r.src.join("new.mp4"), pattern(500, 2)).unwrap();
    fs::write(r.src.join("new2.mp4"), pattern(500, 3)).unwrap();
    set_mtime(&r.src.join("old.mp4"), day1);
    set_mtime(&r.src.join("new.mp4"), day2);
    set_mtime(&r.src.join("new2.mp4"), day2);
    let plan = plan_for(&r);
    let fmt = |t: std::time::SystemTime| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string();
    let (s, _) = run(&plan);
    assert_eq!((s.copied, s.failed), (3, 0));
    assert_eq!(plan.dest_dir, r.dest, "the shown destination is the base folder");
    assert_eq!(list(&r.dest.join(fmt(day1)).join("DJI")), ["old.mp4"]);
    assert_eq!(list(&r.dest.join(fmt(day2)).join("DJI")), ["new.mp4", "new2.mp4"]);
    assert!(!r.dest.join(chrono::Local::now().format("%Y-%m-%d").to_string()).exists(), "no folder for today");
}

#[test]
fn shoot_date_and_mirror_combine() {
    let mut r = rig();
    r.cfg.organize = "shoot".into();
    let m = with_mirror(&mut r);
    fs::write(r.src.join("old.mp4"), pattern(500, 1)).unwrap();
    let day = std::time::SystemTime::now() - std::time::Duration::from_secs(20 * 86_400);
    set_mtime(&r.src.join("old.mp4"), day);
    let (s, _) = run(&plan_for(&r));
    assert_eq!((s.copied, s.mirrored), (1, 1));
    let d = chrono::DateTime::<chrono::Local>::from(day).format("%Y-%m-%d").to_string();
    assert!(r.dest.join(&d).join("DJI/old.mp4").exists() && m.join(&d).join("DJI/old.mp4").exists());
}
