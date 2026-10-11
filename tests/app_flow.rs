#![allow(clippy::field_reassign_with_default)]
use offload::app::*;
use offload::config::Config;
use offload::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Rig {
    _tmp: tempfile::TempDir,
    app: App,
    src: PathBuf,
    dest: PathBuf,
    paths: Paths,
}

fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card/DCIM");
    let src2 = tmp.path().join("card2");
    let dest = tmp.path().join("backup");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&src2).unwrap();
    let mut cfg = Config::default();
    cfg.destination = dest.to_string_lossy().into();
    cfg.profiles[0].sources = vec![src.to_string_lossy().into(), src2.to_string_lossy().into(), tmp.path().join("missing-card").to_string_lossy().into()];
    cfg.profiles[1].sources = vec![tmp.path().join("no-sony").to_string_lossy().into()];
    let paths = Paths { config: tmp.path().join("cfg/config.toml"), history: tmp.path().join("data/history.jsonl"), state: tmp.path().join("data/state.json") };
    let mut app = App::new(cfg, paths.clone());
    app.sys_actions = false;
    Rig { _tmp: tmp, app, src, dest, paths }
}

fn key(app: &mut App, c: KeyCode) {
    app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
}
fn chr(app: &mut App, c: char) {
    key(app, KeyCode::Char(c));
}
fn typ(app: &mut App, s: &str) {
    for c in s.chars() {
        chr(app, c);
    }
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn wait_for(app: &mut App, screen: Screen) {
    let t0 = Instant::now();
    while app.screen != screen {
        app.tick();
        assert!(t0.elapsed() < Duration::from_secs(20), "timed out waiting for {screen:?}, stuck on {:?}", app.screen);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fill(r: &Rig) {
    fs::write(r.src.join("DJI_0001.MP4"), vec![1u8; 3_000_000]).unwrap();
    fs::write(r.src.join("DJI_0002.mp4"), vec![2u8; 2_000_000]).unwrap();
    fs::write(r.src.join("DJI_0001.LRF"), vec![3u8; 1000]).unwrap();
    fs::write(r.src.join("audio.WAV"), vec![4u8; 500_000]).unwrap();
}

#[test]
fn home_shows_profiles_status_and_destination() {
    let mut r = rig();
    let s = render(&mut r.app, 100, 30);
    assert!(s.contains("DJI Osmo Action"), "{s}");
    assert!(s.contains("Sony Mirrorless"));
    assert!(s.contains("partly connected"), "2 of 3 sources present:\n{s}");
    assert!(s.contains("no card detected"));
    assert!(s.contains("Backup destination"));
    assert!(s.contains("not mounted") || s.contains("free of") || s.contains("backup"), "{s}");
    assert!(s.contains("never: LRF"));
    assert!(s.contains("╔═╗"), "wordmark");
}

#[test]
fn full_flow_home_review_run_summary_with_verified_files() {
    let mut r = rig();
    fill(&r);
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Scanning);
    wait_for(&mut r.app, Screen::Review);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("3 of 3 selected"), "{s}");
    assert!(s.contains("DJI_0001.MP4") && s.contains("audio.WAV"));
    assert!(!s.contains(".LRF"), "LRF must never be listed");
    assert!(s.contains("not found - skipped"), "missing source warning:\n{s}");
    assert!(s.contains("Ready to copy"));
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Running, "{:?}", r.app.review.as_ref().map(|r| r.error.clone()));
    let mid = render(&mut r.app, 120, 36);
    assert!(mid.contains("Throughput") && mid.contains("Files"));
    wait_for(&mut r.app, Screen::Summary);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("All files copied and verified"), "{s}");
    assert!(s.contains("3 files"));

    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let out = r.dest.join(&day).join("DJI");
    let mut names: Vec<_> = fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["DJI_0001.MP4", "DJI_0002.mp4", "audio.WAV"]);
    assert_eq!(fs::read(out.join("DJI_0001.MP4")).unwrap(), vec![1u8; 3_000_000]);
    // history written
    let hist = offload::history::load(&r.paths.history);
    assert_eq!(hist.len(), 1);
    assert_eq!((hist[0].copied, hist[0].failed), (3, 0));

    // stats screen shows it
    chr(&mut r.app, 's');
    assert_eq!(r.app.screen, Screen::Stats);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("sessions") && s.contains("DJI Osmo Action"), "{s}");
    key(&mut r.app, KeyCode::Esc);
    assert_eq!(r.app.screen, Screen::Summary);
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Home);

    // second run: everything is recognized as already copied
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Review);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("3 already on dest"), "{s}");
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Summary);
    let (_, sum) = r.app.summary.clone().unwrap();
    assert_eq!((sum.copied, sum.skipped), (0, 3));
}

#[test]
fn review_selection_and_date_filtering() {
    let mut r = rig();
    fill(&r);
    // Backdate one file so there are two shoot dates.
    let old = std::time::SystemTime::now() - Duration::from_secs(5 * 86400);
    fs::File::options().write(true).open(r.src.join("audio.WAV")).unwrap().set_modified(old).unwrap();
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Review);
    assert_eq!(r.app.review.as_ref().unwrap().dates.len(), 2);

    chr(&mut r.app, 'a'); // deselect all
    assert_eq!(r.app.review.as_ref().unwrap().selected_count(), 0);
    key(&mut r.app, KeyCode::Enter); // nothing selected -> stays, with an error
    assert_eq!(r.app.screen, Screen::Review);
    assert!(r.app.review.as_ref().unwrap().error.as_ref().unwrap().contains("Nothing selected"));

    chr(&mut r.app, 'a'); // select all
    key(&mut r.app, KeyCode::Tab);
    assert_eq!(r.app.review.as_ref().unwrap().focus, Focus::Dates);
    chr(&mut r.app, ' '); // first (oldest) date toggled off -> removes the WAV
    let rv = r.app.review.as_ref().unwrap();
    assert_eq!(rv.selected_count(), 2);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("[~]") || s.contains("[ ]"), "{s}");
    chr(&mut r.app, ' '); // toggle back on
    assert_eq!(r.app.review.as_ref().unwrap().selected_count(), 3);

    key(&mut r.app, KeyCode::Tab);
    chr(&mut r.app, ' '); // toggle first file off
    assert_eq!(r.app.review.as_ref().unwrap().selected_count(), 2);
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Summary);
    assert_eq!(r.app.summary.as_ref().unwrap().1.copied, 2);
}

#[test]
fn missing_backup_drive_blocks_start_and_creates_nothing() {
    let mut r = rig();
    fill(&r);
    r.app.cfg.destination = "/Volumes/definitely-not-mounted-zz/Backup".into();
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Review);
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Review);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("not mounted"), "{s}");
    assert!(!std::path::Path::new("/Volumes/definitely-not-mounted-zz").exists());
}

#[test]
fn cancel_flow_asks_for_confirmation() {
    let mut r = rig();
    for i in 0..5 {
        fs::write(r.src.join(format!("big{i}.mp4")), vec![i as u8; 12_000_000]).unwrap();
    }
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Review);
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Running);
    key(&mut r.app, KeyCode::Esc);
    assert!(r.app.run.as_ref().unwrap().confirm_cancel);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("Cancel and discard"), "{s}");
    chr(&mut r.app, 'n'); // keep going
    assert!(!r.app.run.as_ref().unwrap().confirm_cancel);
    assert!(!r.app.run.as_ref().unwrap().ctrl.is_cancelled());
    key(&mut r.app, KeyCode::Esc);
    chr(&mut r.app, 'y');
    wait_for(&mut r.app, Screen::Summary);
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let out = r.dest.join(day).join("DJI");
    for e in fs::read_dir(&out).unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!n.ends_with(".part"), "leftover {n}");
    }
}

#[test]
fn pause_and_resume() {
    let mut r = rig();
    for i in 0..4 {
        fs::write(r.src.join(format!("b{i}.mp4")), vec![i as u8; 10_000_000]).unwrap();
    }
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Review);
    key(&mut r.app, KeyCode::Enter);
    chr(&mut r.app, 'p');
    assert!(r.app.run.as_ref().unwrap().ctrl.is_paused());
    assert!(render(&mut r.app, 120, 36).contains("PAUSED"));
    chr(&mut r.app, 'p');
    assert!(!r.app.run.as_ref().unwrap().ctrl.is_paused());
    wait_for(&mut r.app, Screen::Summary);
    assert_eq!(r.app.summary.as_ref().unwrap().1.copied, 4);
}

#[test]
fn settings_editor_edits_saves_and_validates() {
    let mut r = rig();
    chr(&mut r.app, 'e');
    assert_eq!(r.app.screen, Screen::Config);
    let s = render(&mut r.app, 120, 40);
    assert!(s.contains("Backup folder") && s.contains("Copy types") && s.contains("Add a profile"), "{s}");

    // edit destination (first row)
    key(&mut r.app, KeyCode::Enter);
    assert!(r.app.editor.input.is_some());
    r.app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    typ(&mut r.app, "/tmp/new-backup");
    key(&mut r.app, KeyCode::Enter);
    assert!(r.app.editor.input.is_none());
    assert_eq!(r.app.cfg.destination, "/tmp/new-backup");
    assert_eq!(Config::load(&r.paths.config).unwrap().destination, "/tmp/new-backup", "saved to disk");

    // go to DJI profile's Subfolder: rows = 5 globals, header, name, subfolder
    r.app.editor.cursor = 7;
    key(&mut r.app, KeyCode::Enter);
    r.app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    typ(&mut r.app, "../../evil");
    key(&mut r.app, KeyCode::Enter);
    assert!(r.app.editor.input.is_some(), "invalid input keeps the editor open");
    assert_eq!(r.app.cfg.profiles[0].subfolder, "DJI", "invalid value must not be applied");
    assert!(render(&mut r.app, 120, 40).contains("Not saved"));
    key(&mut r.app, KeyCode::Esc);
    assert!(r.app.editor.input.is_none());

    // edit include list
    r.app.editor.cursor = 9;
    key(&mut r.app, KeyCode::Enter);
    r.app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    typ(&mut r.app, "MP4, .mov wav");
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.cfg.profiles[0].include, ["mp4", "mov", "wav"]);

    // add then delete a profile
    let n = r.app.cfg.profiles.len();
    r.app.editor.cursor = r.app.editor_rows().len() - 1;
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.cfg.profiles.len(), n + 1);
    chr(&mut r.app, 'd');
    chr(&mut r.app, 'y');
    assert_eq!(r.app.cfg.profiles.len(), n);

    // toggle notify
    r.app.editor.cursor = 2;
    key(&mut r.app, KeyCode::Enter);
    assert!(!Config::load(&r.paths.config).unwrap().notify);

    key(&mut r.app, KeyCode::Esc);
    assert_eq!(r.app.screen, Screen::Home);
}

#[test]
fn cannot_delete_last_profile() {
    let mut r = rig();
    r.app.cfg.profiles.truncate(1);
    chr(&mut r.app, 'e');
    r.app.editor.cursor = 5;
    chr(&mut r.app, 'd');
    assert!(r.app.editor.confirm_delete.is_none());
    assert_eq!(r.app.cfg.profiles.len(), 1);
}

#[test]
fn theme_cycles_persists_and_reloads() {
    let mut r = rig();
    let start = r.app.theme.name;
    chr(&mut r.app, 't');
    assert_ne!(r.app.theme.name, start);
    let chosen = r.app.theme.name;
    let app2 = App::new(r.app.cfg.clone(), r.paths.clone());
    assert_eq!(app2.theme.name, chosen);
}

#[test]
fn mouse_click_and_scroll() {
    let mut r = rig();
    render(&mut r.app, 100, 30);
    let (rect, _) = r.app.hits.iter().find(|(_, h)| *h == Hit::Profile(1)).cloned().unwrap();
    let click = |x, y| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE };
    r.app.on_mouse(click(rect.x + 3, rect.y + 2));
    assert_eq!(r.app.home_sel, 1);
    r.app.on_mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: 0, row: 0, modifiers: KeyModifiers::NONE });
    assert_eq!(r.app.home_sel, 0);
    // clicking the already-selected profile starts scanning
    render(&mut r.app, 100, 30);
    let (rect, _) = r.app.hits.iter().find(|(_, h)| *h == Hit::Profile(0)).cloned().unwrap();
    r.app.on_mouse(click(rect.x + 3, rect.y + 2));
    assert_eq!(r.app.screen, Screen::Scanning);
}

#[test]
fn every_screen_renders_at_awkward_sizes_without_panicking() {
    let mut r = rig();
    fill(&r);
    let sizes = [(1, 1), (10, 3), (30, 8), (60, 12), (80, 24), (200, 60)];
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    chr(&mut r.app, '?');
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    chr(&mut r.app, '?');
    chr(&mut r.app, 'e');
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    key(&mut r.app, KeyCode::Esc);
    chr(&mut r.app, 's');
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    key(&mut r.app, KeyCode::Esc);
    key(&mut r.app, KeyCode::Enter);
    for (w, h) in sizes {
        render(&mut r.app, w, h); // scanning
    }
    wait_for(&mut r.app, Screen::Review);
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    key(&mut r.app, KeyCode::Enter);
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    wait_for(&mut r.app, Screen::Summary);
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
}

#[test]
fn hostile_filenames_cannot_inject_escape_sequences() {
    let mut r = rig();
    fs::write(r.src.join("evil\x1b[2Jname.mp4"), b"x").unwrap();
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Review);
    let s = render(&mut r.app, 120, 36);
    assert!(!s.contains('\x1b'));
    assert!(s.contains("evil"));
}

#[test]
fn ctrl_c_quits() {
    let mut r = rig();
    r.app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(r.app.should_quit);
}

#[test]
fn number_keys_start_profiles_and_empty_card_shows_hint() {
    let mut r = rig();
    chr(&mut r.app, '2'); // Sony: source missing
    assert_eq!(r.app.screen, Screen::Scanning);
    wait_for(&mut r.app, Screen::Review);
    let s = render(&mut r.app, 120, 36);
    assert!(s.contains("No matching files found"), "{s}");
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Review, "nothing to start");
    key(&mut r.app, KeyCode::Esc);
    assert_eq!(r.app.screen, Screen::Home);
    chr(&mut r.app, '9'); // out of range: ignored
    assert_eq!(r.app.screen, Screen::Home);
}

/// Renders every screen with stable demo paths (used to produce the README screenshots).
/// Run: cargo test --test app_flow dump_frames -- --ignored --nocapture
#[test]
#[ignore]
fn dump_frames() {
    let demo = PathBuf::from("/tmp/offload-demo");
    let _ = fs::remove_dir_all(&demo);
    let dji = demo.join("SD_Card/DCIM/DJI_001");
    let sony_photos = demo.join("Untitled/DCIM/100MSDCF");
    let sony_clips = demo.join("Untitled/private/M4ROOT/CLIP");
    for d in [&dji, &demo.join("OsmoAction/DCIM"), &sony_photos, &sony_clips, &demo.join("Backup")] {
        fs::create_dir_all(d).unwrap();
    }
    for i in 0..9 {
        fs::write(dji.join(format!("DJI_2026100{}_{:04}_D.MP4", 5 + i / 5, i + 1)), vec![i as u8; 30_000_000 + i * 9_000_000]).unwrap();
    }
    fs::write(dji.join("DJI_20261005_0001_D.WAV"), vec![4u8; 1_500_000]).unwrap();
    fs::write(dji.join("DJI_20261005_0001_D.LRF"), vec![4u8; 1_500]).unwrap();
    for i in 0..4 {
        fs::write(sony_photos.join(format!("DSC0371{i}.ARW")), vec![9u8; 2_000_000]).unwrap();
    }
    fs::write(sony_clips.join("C2546.MP4"), vec![5u8; 20_000_000]).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(3 * 86400);
    fs::File::options().write(true).open(dji.join("DJI_20261005_0001_D.WAV")).unwrap().set_modified(old).unwrap();
    let mut cfg = Config::default();
    cfg.destination = demo.join("Backup").to_string_lossy().into();
    cfg.profiles[0].sources = vec![dji.to_string_lossy().into(), demo.join("OsmoAction/DCIM").to_string_lossy().into()];
    cfg.profiles[1].sources = vec![demo.join("Untitled/DCIM").to_string_lossy().into(), sony_clips.to_string_lossy().into()];
    let paths = Paths { config: demo.join("cfg/config.toml"), history: demo.join("data/history.jsonl"), state: demo.join("data/state.json") };
    let mut app = App::new(cfg, paths);
    app.sys_actions = false;
    let r = &mut app;
    println!("=====HOME\n{}", render(r, 100, 34));
    chr(r, 'b');
    wait_for(r, Screen::Review);
    println!("=====REVIEW\n{}", render(r, 122, 34));
    key(r, KeyCode::Enter);
    let t0 = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(40));
        r.tick();
        if r.screen != Screen::Running || t0.elapsed() > Duration::from_secs(10) {
            break;
        }
        if r.run.as_ref().map(|x| x.done_count() >= 3 && x.copy_rate > 0.0).unwrap_or(false) {
            break;
        }
    }
    println!("=====RUNNING\n{}", render(r, 122, 34));
    wait_for(r, Screen::Summary);
    println!("=====SUMMARY\n{}", render(r, 100, 22));
    chr(r, 's');
    println!("=====STATS\n{}", render(r, 100, 22));
    key(r, KeyCode::Esc);
    key(r, KeyCode::Enter);
    chr(r, 'e');
    println!("=====SETTINGS\n{}", render(r, 100, 24));
}

// ---------- batch: both cameras at once ----------

fn batch_rig() -> Rig {
    let mut r = rig();
    let sony = r._tmp.path().join("sony/DCIM");
    let clip = r._tmp.path().join("sony/CLIP");
    fs::create_dir_all(&sony).unwrap();
    fs::create_dir_all(&clip).unwrap();
    r.app.cfg.profiles[1].sources = vec![sony.to_string_lossy().into(), clip.to_string_lossy().into()];
    r.app.refresh_live();
    fs::write(r.src.join("DJI_0001.MP4"), vec![1u8; 2_000_000]).unwrap();
    fs::write(r.src.join("DJI_0002.WAV"), vec![2u8; 300_000]).unwrap();
    fs::write(sony.join("DSC0001.ARW"), vec![3u8; 1_500_000]).unwrap();
    fs::write(clip.join("C0001.MP4"), vec![4u8; 3_000_000]).unwrap();
    r
}

#[test]
fn home_shows_a_batch_card_with_live_readiness() {
    let mut r = batch_rig();
    let s = render(&mut r.app, 110, 34);
    assert!(s.contains("Batch · DJI + Sony"), "{s}");
    assert!(s.contains("all ready"), "{s}");
    assert!(s.contains("every file verified"));
    assert!(s.contains("b batch all"), "footer hint");
    // only one camera connected -> batch card says so
    let mut r2 = rig(); // Sony source is missing here
    let s = render(&mut r2.app, 110, 34);
    assert!(s.contains("1 of 2 ready"), "{s}");
}

#[test]
fn batch_flow_review_run_summary_and_history() {
    let mut r = batch_rig();
    chr(&mut r.app, 'b');
    assert_eq!(r.app.screen, Screen::Scanning);
    wait_for(&mut r.app, Screen::Review);
    let rv = r.app.review.as_ref().unwrap();
    assert!(rv.is_batch());
    assert_eq!(rv.selected_count(), 4);
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("Batch · DJI + Sony"), "{s}");
    assert!(s.contains("Cameras in this batch"));
    assert!(s.contains("DJI") && s.contains("Sony") && s.contains("DSC0001.ARW") && s.contains("DJI_0001.MP4"));
    // deselect one Sony file, then run
    let idx = r.app.review.as_ref().unwrap().scan.files.iter().position(|f| f.path.ends_with("C0001.MP4")).unwrap();
    r.app.review.as_mut().unwrap().selected[idx] = false;
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Running);
    let mid = render(&mut r.app, 130, 40);
    assert!(mid.contains("Batch · DJI + Sony"), "{mid}");
    wait_for(&mut r.app, Screen::Summary);
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("Batch · DJI + Sony") && s.contains("All files copied and verified"), "{s}");
    assert!(s.contains("2 copied") && s.contains("1 copied"), "per-camera rows:\n{s}");

    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dji = r.dest.join(&day).join("DJI");
    let son = r.dest.join(&day).join("Sony");
    assert_eq!(fs::read(dji.join("DJI_0001.MP4")).unwrap(), vec![1u8; 2_000_000]);
    assert_eq!(fs::read(son.join("DSC0001.ARW")).unwrap(), vec![3u8; 1_500_000]);
    assert!(!son.join("C0001.MP4").exists(), "deselected file not copied");

    let hist = offload::history::load(&r.paths.history);
    assert_eq!(hist.len(), 2, "one history record per camera");
    let mut names: Vec<_> = hist.iter().map(|h| h.profile.as_str()).collect();
    names.sort();
    assert_eq!(names, ["DJI Osmo Action", "Sony Mirrorless"]);
    assert_eq!(hist.iter().map(|h| h.copied).sum::<usize>(), 3);
}

#[test]
fn batch_with_one_camera_connected_runs_just_that_one_and_says_so() {
    let mut r = rig(); // DJI present, Sony missing
    fs::write(r.src.join("DJI_0001.MP4"), vec![1u8; 100_000]).unwrap();
    chr(&mut r.app, 'b');
    assert!(r.app.toast.as_ref().unwrap().text.contains("left out of the batch"), "{:?}", r.app.toast);
    wait_for(&mut r.app, Screen::Review);
    assert!(!r.app.review.as_ref().unwrap().is_batch(), "a batch of one behaves like a normal run");
    assert_eq!(r.app.review.as_ref().unwrap().profiles, vec![0]);
}

#[test]
fn batch_with_no_cards_stays_home() {
    let mut r = rig();
    r.app.cfg.profiles[0].sources = vec!["/definitely/not/here".into()];
    r.app.refresh_live();
    chr(&mut r.app, 'b');
    assert_eq!(r.app.screen, Screen::Home);
    assert!(r.app.toast.as_ref().unwrap().text.contains("No camera cards"));
}

#[test]
fn batch_card_reachable_by_arrows_enter_number_zero_and_mouse() {
    let mut r = batch_rig();
    key(&mut r.app, KeyCode::Down);
    key(&mut r.app, KeyCode::Down);
    assert_eq!(r.app.home_sel, 2, "profiles 0,1 then the batch card");
    key(&mut r.app, KeyCode::Down);
    assert_eq!(r.app.home_sel, 2, "can't go past the batch card");
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Scanning);
    wait_for(&mut r.app, Screen::Review);
    assert!(r.app.review.as_ref().unwrap().is_batch());

    let mut r = batch_rig();
    chr(&mut r.app, '0');
    wait_for(&mut r.app, Screen::Review);
    assert!(r.app.review.as_ref().unwrap().is_batch());

    let mut r = batch_rig();
    render(&mut r.app, 110, 40);
    let (rect, _) = r.app.hits.iter().find(|(_, h)| *h == Hit::Profile(2)).cloned().unwrap();
    let click = |x, y| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE };
    r.app.on_mouse(click(rect.x + 3, rect.y + 2));
    assert_eq!(r.app.home_sel, 2);
    render(&mut r.app, 110, 40);
    r.app.on_mouse(click(rect.x + 3, rect.y + 2));
    assert_eq!(r.app.screen, Screen::Scanning);
}

#[test]
fn single_profile_config_has_no_batch_card() {
    let mut r = rig();
    r.app.cfg.profiles.truncate(1);
    r.app.refresh_live();
    assert!(!r.app.has_batch());
    assert!(!render(&mut r.app, 110, 34).contains("batch"));
    chr(&mut r.app, 'b'); // ignored
    assert_eq!(r.app.screen, Screen::Home);
}

#[test]
fn batch_screens_render_at_awkward_sizes() {
    let mut r = batch_rig();
    let sizes = [(1, 1), (30, 8), (60, 14), (80, 24), (200, 60)];
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    chr(&mut r.app, 'b');
    wait_for(&mut r.app, Screen::Review);
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    key(&mut r.app, KeyCode::Enter);
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
    wait_for(&mut r.app, Screen::Summary);
    for (w, h) in sizes {
        render(&mut r.app, w, h);
    }
}

// ---------- copy vs move, and deleting originals afterwards ----------

fn mode_rig() -> Rig {
    let r = rig();
    fs::write(r.src.join("A.MP4"), vec![1u8; 700_000]).unwrap();
    fs::write(r.src.join("B.MP4"), vec![2u8; 300_000]).unwrap();
    fs::write(r.src.join("C.WAV"), vec![3u8; 50_000]).unwrap();
    r
}

fn to_review(r: &mut Rig) {
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Review);
}

fn day_dir(r: &Rig) -> PathBuf {
    r.dest.join(chrono::Local::now().format("%Y-%m-%d").to_string()).join("DJI")
}

#[test]
fn review_defaults_to_copy_and_m_toggles_mode() {
    let mut r = mode_rig();
    to_review(&mut r);
    assert_eq!(r.app.review.as_ref().unwrap().mode, offload::engine::Mode::Copy);
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("COPY") && s.contains("originals stay on the card"), "{s}");
    chr(&mut r.app, 'm');
    assert_eq!(r.app.review.as_ref().unwrap().mode, offload::engine::Mode::Move);
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("MOVE") && s.contains("originals deleted once verified"), "{s}");
    chr(&mut r.app, 'm');
    assert_eq!(r.app.review.as_ref().unwrap().mode, offload::engine::Mode::Copy);
}

#[test]
fn config_default_mode_move_is_honoured() {
    let mut r = mode_rig();
    r.app.cfg.mode = "move".into();
    to_review(&mut r);
    assert_eq!(r.app.review.as_ref().unwrap().mode, offload::engine::Mode::Move);
}

#[test]
fn move_needs_an_explicit_y_and_enter_does_not_confirm() {
    let mut r = mode_rig();
    to_review(&mut r);
    chr(&mut r.app, 'm');
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.screen, Screen::Review, "asks first");
    assert!(r.app.review.as_ref().unwrap().confirm_move);
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("MOVE deletes each original"), "{s}");
    key(&mut r.app, KeyCode::Enter); // Enter must NOT confirm a destructive action
    assert_eq!(r.app.screen, Screen::Review);
    assert!(!r.app.review.as_ref().unwrap().confirm_move, "any other key backs out");
    assert!(r.src.join("A.MP4").exists() && !day_dir(&r).exists(), "nothing happened");

    key(&mut r.app, KeyCode::Enter);
    chr(&mut r.app, 'n');
    assert_eq!(r.app.screen, Screen::Review);

    key(&mut r.app, KeyCode::Enter);
    chr(&mut r.app, 'y');
    assert_eq!(r.app.screen, Screen::Running);
    assert!(render(&mut r.app, 130, 40).contains("MOVING"));
    wait_for(&mut r.app, Screen::Summary);
    assert!(!r.src.join("A.MP4").exists() && !r.src.join("B.MP4").exists() && !r.src.join("C.WAV").exists(), "originals deleted after verification");
    assert_eq!(fs::read(day_dir(&r).join("A.MP4")).unwrap(), vec![1u8; 700_000]);
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("3 deleted from the card (moved)"), "{s}");
    assert_eq!(r.app.summary.as_ref().unwrap().1.moved, 3);
}

#[test]
fn copy_then_offer_to_delete_originals_with_explicit_confirmation() {
    let mut r = mode_rig();
    to_review(&mut r);
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Summary);
    assert!(r.src.join("A.MP4").exists(), "copy mode keeps originals");
    assert_eq!(r.app.originals.len(), 3);
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("press d to delete the 3 verified ones"), "{s}");
    assert!(s.contains("d delete originals"), "footer offers it:\n{s}");

    // d, then Enter: must NOT delete
    chr(&mut r.app, 'd');
    assert!(r.app.delete_confirm);
    assert!(render(&mut r.app, 140, 40).contains("PERMANENTLY delete 3 originals"));
    key(&mut r.app, KeyCode::Enter);
    assert!(!r.app.delete_confirm);
    assert!(r.src.join("A.MP4").exists() && r.src.join("B.MP4").exists() && r.src.join("C.WAV").exists());

    // d, then another key: keeps
    chr(&mut r.app, 'd');
    chr(&mut r.app, 'n');
    assert!(r.src.join("A.MP4").exists());

    // d, then y: deletes
    chr(&mut r.app, 'd');
    chr(&mut r.app, 'y');
    assert!(!r.src.join("A.MP4").exists() && !r.src.join("B.MP4").exists() && !r.src.join("C.WAV").exists());
    assert_eq!(fs::read(day_dir(&r).join("B.MP4")).unwrap(), vec![2u8; 300_000], "the backups are untouched");
    let s = render(&mut r.app, 130, 40);
    assert!(s.contains("3 deleted") && s.contains("from the card"), "{s}");
    assert!(!s.contains("press d to delete"), "the offer is gone");
    chr(&mut r.app, 'd'); // nothing left to delete
    assert!(!r.app.delete_confirm);
}

#[test]
fn files_that_failed_are_never_offered_for_deletion() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = mode_rig();
    to_review(&mut r);
    fs::set_permissions(r.src.join("B.MP4"), fs::Permissions::from_mode(0o000)).unwrap();
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Summary);
    fs::set_permissions(r.src.join("B.MP4"), fs::Permissions::from_mode(0o644)).unwrap();
    if r.app.summary.as_ref().unwrap().1.failed == 0 {
        return; // running as root: permissions are ignored
    }
    assert_eq!(r.app.originals.len(), 2);
    assert_eq!(r.app.ineligible, 1);
    assert!(render(&mut r.app, 130, 40).contains("1 file(s) were not verified"));
    chr(&mut r.app, 'd');
    chr(&mut r.app, 'y');
    assert!(r.src.join("B.MP4").exists(), "the file that failed to copy is untouched");
    assert!(!r.src.join("A.MP4").exists() && !r.src.join("C.WAV").exists());
}

#[test]
fn delete_refuses_originals_that_changed_after_the_copy() {
    let mut r = mode_rig();
    to_review(&mut r);
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Summary);
    fs::write(r.src.join("A.MP4"), b"new footage recorded after the offload").unwrap();
    chr(&mut r.app, 'd');
    chr(&mut r.app, 'y');
    assert!(r.src.join("A.MP4").exists(), "modified original is kept");
    assert!(!r.src.join("B.MP4").exists());
    let rep = r.app.deleted.as_ref().unwrap();
    assert_eq!((rep.deleted, rep.failures.len()), (2, 1));
    assert!(render(&mut r.app, 130, 40).contains("A.MP4"));
}

#[test]
fn no_delete_offer_after_a_move_or_when_nothing_was_verified() {
    let mut r = mode_rig();
    r.app.cfg.mode = "move".into();
    to_review(&mut r);
    key(&mut r.app, KeyCode::Enter);
    chr(&mut r.app, 'y');
    wait_for(&mut r.app, Screen::Summary);
    chr(&mut r.app, 'd');
    assert!(!r.app.delete_confirm, "moves already deleted; nothing more to offer");
    assert!(!render(&mut r.app, 130, 40).contains("d delete originals"));
}

#[test]
fn settings_can_set_the_default_mode() {
    let mut r = mode_rig();
    chr(&mut r.app, 'e');
    let rows = r.app.editor_rows();
    let i = rows.iter().position(|row| *row == Row::Global(GField::Mode)).unwrap();
    r.app.editor.cursor = i;
    assert!(render(&mut r.app, 120, 40).contains("copy (keep originals)"));
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.cfg.mode, "move");
    assert_eq!(Config::load(&r.paths.config).unwrap().mode, "move", "saved");
    assert!(render(&mut r.app, 120, 40).contains("move (delete originals once verified)"));
    key(&mut r.app, KeyCode::Enter);
    assert_eq!(r.app.cfg.mode, "copy");
}

#[test]
fn batch_move_deletes_from_both_cards_and_summary_says_so() {
    let mut r = batch_rig();
    chr(&mut r.app, 'b');
    wait_for(&mut r.app, Screen::Review);
    chr(&mut r.app, 'm');
    key(&mut r.app, KeyCode::Enter);
    chr(&mut r.app, 'y');
    wait_for(&mut r.app, Screen::Summary);
    assert_eq!(r.app.summary.as_ref().unwrap().1.moved, 4);
    assert!(!r.src.join("DJI_0001.MP4").exists());
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert!(r.dest.join(&day).join("Sony/DSC0001.ARW").exists());
}

/// Renders the copy/move screens as text. Run: cargo test --test app_flow dump_modes -- --ignored --nocapture
#[test]
#[ignore]
fn dump_modes() {
    let mut r = mode_rig();
    to_review(&mut r);
    chr(&mut r.app, 'm');
    println!("=====MOVE_REVIEW\n{}", render(&mut r.app, 120, 30));
    key(&mut r.app, KeyCode::Enter);
    println!("=====MOVE_CONFIRM\n{}", render(&mut r.app, 120, 30));
    chr(&mut r.app, 'n');
    chr(&mut r.app, 'm');
    key(&mut r.app, KeyCode::Enter);
    wait_for(&mut r.app, Screen::Summary);
    println!("=====COPY_SUMMARY\n{}", render(&mut r.app, 120, 26));
    chr(&mut r.app, 'd');
    println!("=====DELETE_CONFIRM\n{}", render(&mut r.app, 140, 26));
    chr(&mut r.app, 'y');
    println!("=====AFTER_DELETE\n{}", render(&mut r.app, 120, 26));
}
