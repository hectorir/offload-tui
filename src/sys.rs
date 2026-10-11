//! OS integrations. Every external command is spawned with an argument vector (never through a shell),
//! so file names and messages can't be interpreted as commands.

use crate::fsutil::volume_root;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Desktop notification (macOS: osascript with argv; Linux: notify-send). Best effort.
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("osascript")
            .args(["-e", "on run argv", "-e", "display notification (item 1 of argv) with title (item 2 of argv) sound name \"Glass\"", "-e", "end run", "--"])
            .arg(body)
            .arg(title)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = Command::new("notify-send").arg("--").arg(title).arg(body).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

/// Keeps the computer awake while alive. macOS runs `caffeinate -w <our pid>`, so the hold also ends by itself
/// if offload is killed; Linux uses `systemd-inhibit` where available. Best effort: if it can't start, nothing happens.
pub struct AwakeGuard(Option<std::process::Child>);

impl AwakeGuard {
    pub fn new() -> AwakeGuard {
        let pid = std::process::id().to_string();
        let child = if cfg!(target_os = "macos") {
            Command::new("caffeinate").args(["-i", "-m", "-s", "-w", &pid]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        } else {
            Command::new("systemd-inhibit")
                .args(["--what=sleep:idle", "--who=offload", "--why=copying camera footage", "--mode=block", "tail", "--pid", &pid, "-f", "/dev/null"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        };
        AwakeGuard(child.ok())
    }

    /// Whether a hold is actually in place.
    pub fn active(&self) -> bool {
        self.0.is_some()
    }
}

impl Default for AwakeGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AwakeGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// What to do with the computer once a run has finished cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhenDone {
    Nothing,
    Sleep,
    Shutdown,
}

impl WhenDone {
    pub fn from_config(s: &str) -> WhenDone {
        match s.to_ascii_lowercase().as_str() {
            "sleep" => WhenDone::Sleep,
            "shutdown" => WhenDone::Shutdown,
            _ => WhenDone::Nothing,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            WhenDone::Nothing => "nothing",
            WhenDone::Sleep => "sleep",
            WhenDone::Shutdown => "shutdown",
        }
    }
    pub fn next(self) -> WhenDone {
        match self {
            WhenDone::Nothing => WhenDone::Sleep,
            WhenDone::Sleep => WhenDone::Shutdown,
            WhenDone::Shutdown => WhenDone::Nothing,
        }
    }
    /// Verb for messages: "sleep" / "shut down".
    pub fn verb(self) -> &'static str {
        match self {
            WhenDone::Nothing => "do nothing",
            WhenDone::Sleep => "go to sleep",
            WhenDone::Shutdown => "shut down",
        }
    }
}

/// How long the warning shows before the computer is actually put to sleep or shut down.
pub const POWER_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

/// Put the computer to sleep or shut it down. Arguments are fixed; nothing user-controlled reaches a shell.
pub fn power(action: WhenDone) {
    let (cmd, args): (&str, &[&str]) = match (action, cfg!(target_os = "macos")) {
        (WhenDone::Nothing, _) => return,
        (WhenDone::Sleep, true) => ("pmset", &["sleepnow"]),
        (WhenDone::Shutdown, true) => ("osascript", &["-e", "tell application \"System Events\" to shut down"]),
        (WhenDone::Sleep, false) => ("systemctl", &["suspend"]),
        (WhenDone::Shutdown, false) => ("systemctl", &["poweroff"]),
    };
    let _ = Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Reveal a folder in Finder / the file manager.
pub fn open_folder(p: &Path) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = Command::new(opener).arg("--").arg(p).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Unique mounted volumes behind these source paths (only `/Volumes/<name>`).
pub fn volumes_of(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = paths.iter().filter_map(|p| volume_root(p)).filter(|r| r.is_dir()).collect();
    v.sort();
    v.dedup();
    v
}

/// Safely eject a volume. Returns a human-readable result.
pub fn eject(volume: &Path) -> Result<(), String> {
    if volume_root(volume).as_deref() != Some(volume) {
        return Err("refusing to eject something that is not an external volume".into());
    }
    let out = Command::new("diskutil").arg("eject").arg(volume).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string().chars().take(200).collect()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_dedupe_and_filter() {
        let v = volumes_of(&[PathBuf::from("/tmp/x"), PathBuf::from("/Volumes/definitely-missing-zz/a")]);
        assert!(v.is_empty());
    }

    #[test]
    fn when_done_parses_and_cycles() {
        assert_eq!(WhenDone::from_config("SLEEP"), WhenDone::Sleep);
        assert_eq!(WhenDone::from_config("shutdown"), WhenDone::Shutdown);
        assert_eq!(WhenDone::from_config("whatever"), WhenDone::Nothing);
        assert_eq!(WhenDone::Nothing.next().next().next(), WhenDone::Nothing);
    }

    #[test]
    fn awake_guard_starts_and_stops_cleanly() {
        let g = AwakeGuard::new();
        drop(g); // must not hang or panic whether or not the helper exists
    }

    #[test]
    fn eject_refuses_non_volumes() {
        assert!(eject(Path::new("/")).is_err());
        assert!(eject(Path::new("/Volumes/X/sub")).is_err());
        assert!(eject(Path::new("/Users")).is_err());
    }
}
