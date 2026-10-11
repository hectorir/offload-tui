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
    fn eject_refuses_non_volumes() {
        assert!(eject(Path::new("/")).is_err());
        assert!(eject(Path::new("/Volumes/X/sub")).is_err());
        assert!(eject(Path::new("/Users")).is_err());
    }
}
