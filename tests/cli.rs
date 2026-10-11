use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_offload"))
}

struct Env {
    tmp: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        Env { tmp: tempfile::tempdir().unwrap() }
    }
    fn p(&self, s: &str) -> PathBuf {
        self.tmp.path().join(s)
    }
    fn cmd(&self) -> Command {
        let mut c = bin();
        c.env("XDG_CONFIG_HOME", self.p("xdg-config")).env("XDG_DATA_HOME", self.p("xdg-data")).env("HOME", self.p("home"));
        c
    }
    /// A config with one profile pointing into the temp dir.
    fn write_config(&self) -> PathBuf {
        let cfg = self.p("config.toml");
        fs::write(
            &cfg,
            format!(
                "destination = '{}'\n[[profile]]\nname = 'Test Cam'\nsubfolder = 'Cam'\nsources = ['{}', '{}']\ninclude = ['mp4', 'wav']\nexclude = ['lrf']\n",
                self.p("backup").display(),
                self.p("card/DCIM").display(),
                self.p("card-missing").display()
            ),
        )
        .unwrap();
        fs::create_dir_all(self.p("card/DCIM/100")).unwrap();
        fs::write(self.p("card/DCIM/100/A.MP4"), vec![7u8; 100_000]).unwrap();
        fs::write(self.p("card/DCIM/100/B.wav"), vec![8u8; 50_000]).unwrap();
        fs::write(self.p("card/DCIM/100/A.LRF"), b"proxy").unwrap();
        cfg
    }
}

fn out(o: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn version_and_help() {
    let o = bin().arg("--version").output().unwrap();
    assert!(out(&o).starts_with("offload"));
    let o = bin().arg("--help").output().unwrap();
    let s = out(&o);
    assert!(s.contains("run") && s.contains("pack") && s.contains("init"));
}

#[test]
fn init_writes_hectorir_config_and_refuses_overwrite() {
    let e = Env::new();
    let cfg = e.p("c/config.toml");
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "init"]).output().unwrap();
    assert!(o.status.success(), "{}", out(&o));
    let text = fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("/Volumes/SD_Card/DCIM/DJI_001") && text.contains("/Volumes/OsmoAction/DCIM"));
    assert!(text.contains("/Volumes/Untitled/DCIM") && text.contains("/Volumes/Untitled/private/M4ROOT/CLIP"));
    assert!(text.contains("Old External Drives/0 Photos & My Footage/Needs to Back up"));
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "init"]).output().unwrap();
    assert!(!o.status.success());
    assert!(out(&o).contains("already exists"));
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "init", "--force"]).output().unwrap();
    assert!(o.status.success());
    assert!(e.p("c/config.toml.bak").exists());
}

#[test]
fn config_path_honours_xdg() {
    let e = Env::new();
    let o = e.cmd().arg("config-path").output().unwrap();
    assert_eq!(out(&o).trim(), e.p("xdg-config/offload/config.toml").display().to_string());
}

#[test]
fn profiles_lists_builtin_preset_without_a_config_file() {
    let e = Env::new();
    let o = e.cmd().arg("profiles").output().unwrap();
    let s = out(&o);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("DJI Osmo Action") && s.contains("Sony Mirrorless"));
}

#[test]
fn dry_run_writes_nothing() {
    let e = Env::new();
    let cfg = e.write_config();
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "test", "--dry-run"]).output().unwrap();
    let s = out(&o);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("would copy A.MP4") && s.contains("would copy B.wav"), "{s}");
    assert!(!s.contains("A.LRF"));
    assert!(s.contains("not found, skipped"));
    assert!(!e.p("backup").exists(), "dry run must not create the destination");
    assert!(!e.p("xdg-data").exists(), "dry run must not write history");
}

#[test]
fn run_copies_verifies_and_is_idempotent() {
    let e = Env::new();
    let cfg = e.write_config();
    let args = ["--config", cfg.to_str().unwrap(), "run", "Test Cam", "--yes"];
    let o = e.cmd().args(args).output().unwrap();
    let s = out(&o);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("2 copied") && s.contains("0 failed"), "{s}");
    let day = chrono_today();
    let dest = e.p("backup").join(&day).join("Cam");
    assert_eq!(fs::read(dest.join("A.MP4")).unwrap(), vec![7u8; 100_000]);
    assert_eq!(fs::read(dest.join("B.wav")).unwrap(), vec![8u8; 50_000]);
    assert!(!dest.join("A.LRF").exists());
    assert!(e.p("xdg-data/offload/history.jsonl").exists());

    let o = e.cmd().args(args).output().unwrap();
    let s = out(&o);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("0 copied") && s.contains("2 skipped"), "{s}");
}

#[test]
fn run_without_yes_aborts_on_no_and_proceeds_on_yes() {
    let e = Env::new();
    let cfg = e.write_config();
    let run = |answer: &str| {
        let mut child = e
            .cmd()
            .args(["--config", cfg.to_str().unwrap(), "run", "test"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(answer.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    let o = run("n\n");
    assert_eq!(o.status.code(), Some(1));
    assert!(!e.p("backup").exists());
    let o = run("y\n");
    assert!(o.status.success(), "{}", out(&o));
    assert!(e.p("backup").exists());
}

#[test]
fn unknown_profile_and_bad_config_fail_cleanly() {
    let e = Env::new();
    let cfg = e.write_config();
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "nope", "--yes"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(out(&o).contains("no profile matching"));

    let bad = e.p("bad.toml");
    fs::write(&bad, "[[profile]]\nname='x'\nsubfolder='../escape'\nsources=['/a']\ninclude=['mp4']\n").unwrap();
    let o = e.cmd().args(["--config", bad.to_str().unwrap(), "profiles"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(out(&o).contains("single plain folder name"), "{}", out(&o));
}

#[test]
fn missing_destination_volume_exits_2_without_creating_anything() {
    let e = Env::new();
    let cfg = e.write_config();
    let text = fs::read_to_string(&cfg).unwrap().replace(&format!("{}", e.p("backup").display()), "/Volumes/definitely-not-mounted-zz/Backup");
    fs::write(&cfg, text).unwrap();
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "test", "--yes"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("not mounted"));
    assert!(!Path::new("/Volumes/definitely-not-mounted-zz").exists());
}

#[test]
fn pack_export_import_roundtrip() {
    let e = Env::new();
    let cfg = e.write_config();
    let pack = e.p("cam.toml");
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "pack", "export", "test", "-o", pack.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "{}", out(&o));
    assert!(!fs::read_to_string(&pack).unwrap().contains("backup"), "pack must not leak your drive paths");

    // import into a fresh config (starts from the built-in preset)
    let fresh = e.p("fresh/config.toml");
    let o = e.cmd().args(["--config", fresh.to_str().unwrap(), "pack", "import", pack.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "{}", out(&o));
    let o = e.cmd().args(["--config", fresh.to_str().unwrap(), "profiles"]).output().unwrap();
    let s = out(&o);
    assert!(s.contains("Test Cam") && s.contains("DJI Osmo Action"), "{s}");

    // importing again is refused
    let o = e.cmd().args(["--config", fresh.to_str().unwrap(), "pack", "import", pack.to_str().unwrap()]).output().unwrap();
    assert!(!o.status.success());
    assert!(out(&o).contains("already have"));

    // malicious pack rejected
    let evil = e.p("evil.toml");
    fs::write(&evil, "[profile]\nname='e'\nsubfolder='../../x'\nsources=['/a']\ninclude=['mp4']\n").unwrap();
    let o = e.cmd().args(["--config", fresh.to_str().unwrap(), "pack", "import", evil.to_str().unwrap()]).output().unwrap();
    assert!(!o.status.success());
}

fn chrono_today() -> String {
    let o = Command::new("date").arg("+%Y-%m-%d").output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn two_camera_config(e: &Env) -> PathBuf {
    let cfg = e.p("two.toml");
    fs::write(
        &cfg,
        format!(
            "destination = '{}'\n[[profile]]\nname = 'Action Cam'\nsubfolder = 'Action'\nsources = ['{}']\ninclude = ['mp4']\nexclude = ['lrf']\n\n[[profile]]\nname = 'Mirrorless'\nsubfolder = 'Mirror'\nsources = ['{}']\ninclude = ['arw', 'mp4']\n",
            e.p("backup").display(),
            e.p("action").display(),
            e.p("mirror").display()
        ),
    )
    .unwrap();
    fs::create_dir_all(e.p("action")).unwrap();
    fs::create_dir_all(e.p("mirror")).unwrap();
    fs::write(e.p("action/A.MP4"), vec![1u8; 60_000]).unwrap();
    fs::write(e.p("action/A.LRF"), b"x").unwrap();
    fs::write(e.p("mirror/B.ARW"), vec![2u8; 40_000]).unwrap();
    fs::write(e.p("mirror/A.MP4"), vec![3u8; 30_000]).unwrap(); // same name as the action cam's file
    cfg
}

#[test]
fn batch_dry_run_lists_both_cameras_and_writes_nothing() {
    let e = Env::new();
    let cfg = two_camera_config(&e);
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "action", "mirror", "--dry-run"]).output().unwrap();
    let s = out(&o);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("Batch: Action Cam + Mirrorless"), "{s}");
    assert!(s.contains("[Action Cam] A.MP4") && s.contains("[Mirrorless] B.ARW") && s.contains("[Mirrorless] A.MP4"), "{s}");
    assert!(!s.contains("LRF"));
    assert!(!e.p("backup").exists());
}

#[test]
fn run_all_copies_every_camera_into_its_own_folder() {
    let e = Env::new();
    let cfg = two_camera_config(&e);
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "--all", "--yes"]).output().unwrap();
    let s = out(&o);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("3 copied") && s.contains("0 failed"), "{s}");
    assert!(s.contains("Action Cam") && s.contains("Mirrorless"), "per-camera lines:\n{s}");
    let day = chrono_today();
    let root = e.p("backup").join(day);
    assert_eq!(fs::read(root.join("Action/A.MP4")).unwrap(), vec![1u8; 60_000]);
    assert_eq!(fs::read(root.join("Mirror/A.MP4")).unwrap(), vec![3u8; 30_000], "same file name, different camera, no clash");
    assert_eq!(fs::read(root.join("Mirror/B.ARW")).unwrap(), vec![2u8; 40_000]);
    assert!(!root.join("Action/A.LRF").exists());
    let hist = fs::read_to_string(e.p("xdg-data/offload/history.jsonl")).unwrap();
    assert_eq!(hist.lines().count(), 2, "one history record per camera");
}

#[test]
fn run_requires_a_profile_or_all() {
    let e = Env::new();
    let cfg = two_camera_config(&e);
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "--yes"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(out(&o).contains("at least one profile"), "{}", out(&o));
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "action", "ghost", "--yes"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(out(&o).contains("no profile matching 'ghost'"));
    assert!(!e.p("backup").exists(), "an unknown name aborts before anything is written");
}

#[test]
fn move_flag_deletes_originals_after_verified_copy() {
    let e = Env::new();
    let cfg = e.write_config();
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "test", "--yes", "--move"]).output().unwrap();
    let s = out(&o);
    assert!(o.status.success(), "{s}");
    assert!(s.contains("moved") && s.contains("2 originals deleted from the card, 0 kept"), "{s}");
    assert!(!e.p("card/DCIM/100/A.MP4").exists() && !e.p("card/DCIM/100/B.wav").exists());
    assert!(e.p("card/DCIM/100/A.LRF").exists(), "files that were never in the plan are untouched");
    let dest = e.p("backup").join(chrono_today()).join("Cam");
    assert_eq!(fs::read(dest.join("A.MP4")).unwrap(), vec![7u8; 100_000]);
}

#[test]
fn move_dry_run_and_declined_prompt_delete_nothing() {
    let e = Env::new();
    let cfg = e.write_config();
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "test", "--move", "--dry-run"]).output().unwrap();
    assert!(out(&o).contains("would move A.MP4"), "{}", out(&o));
    assert!(e.p("card/DCIM/100/A.MP4").exists());

    let mut child = e
        .cmd()
        .args(["--config", cfg.to_str().unwrap(), "run", "test", "--move"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(b"n\n").unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(out(&o).contains("DELETE each original"), "{}", out(&o));
    assert!(e.p("card/DCIM/100/A.MP4").exists() && !e.p("backup").exists());
}

#[test]
fn config_mode_move_is_the_default_and_copy_flag_overrides_it() {
    let e = Env::new();
    let cfg = e.write_config();
    let text = fs::read_to_string(&cfg).unwrap();
    fs::write(&cfg, format!("mode = 'move'\n{text}")).unwrap();
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "test", "--yes", "--copy"]).output().unwrap();
    assert!(o.status.success(), "{}", out(&o));
    assert!(e.p("card/DCIM/100/A.MP4").exists(), "--copy keeps originals even if the config says move");
    // second run honours the config's move default (files are already backed up -> skipped, originals removed)
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "test", "--yes"]).output().unwrap();
    assert!(o.status.success(), "{}", out(&o));
    assert!(out(&o).contains("original deleted"), "{}", out(&o));
    assert!(!e.p("card/DCIM/100/A.MP4").exists());
}

#[test]
fn move_and_copy_flags_conflict() {
    let e = Env::new();
    let cfg = e.write_config();
    let o = e.cmd().args(["--config", cfg.to_str().unwrap(), "run", "test", "--move", "--copy", "--yes"]).output().unwrap();
    assert!(!o.status.success());
    assert!(e.p("card/DCIM/100/A.MP4").exists());
}
