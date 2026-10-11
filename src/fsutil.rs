//! Small filesystem and formatting helpers.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Replace control characters so hostile filenames can't inject terminal escape sequences.
pub fn sanitize_display(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { '\u{FFFD}' } else { c }).collect()
}

pub fn human_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

pub fn human_rate(bps: f64) -> String {
    format!("{}/s", human_bytes(bps.max(0.0) as u64))
}

pub fn human_duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// (total, free) bytes of the filesystem holding `path` (nearest existing ancestor).
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return None;
        }
    }
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    // macOS: statvfs uses 32-bit block counts, which overflow on big network shares (a 20 TB SMB share
    // reported as 2.4 TB). statfs has 64-bit counts and is what `df` uses.
    #[cfg(target_os = "macos")]
    {
        let mut st: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized, writable statfs.
        if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let bs = st.f_bsize as u64;
        Some((st.f_blocks * bs, st.f_bavail * bs))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized, writable statvfs.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
    }
}

/// `/Volumes/<name>` for paths on an external volume (macOS), else None.
pub fn volume_root(path: &Path) -> Option<PathBuf> {
    let mut it = path.components();
    use std::path::Component::*;
    match (it.next(), it.next(), it.next()) {
        (Some(RootDir), Some(Normal(v)), Some(Normal(name))) if v == "Volumes" => Some(PathBuf::from("/Volumes").join(name)),
        _ => None,
    }
}

/// For destinations under `/Volumes/X`, refuse to proceed (and never create dirs) if X isn't mounted.
pub fn destination_volume_present(dest: &Path) -> bool {
    match volume_root(dest) {
        Some(root) => root.is_dir(),
        None => true,
    }
}

/// Ask the OS not to cache reads on this file so verification hits the disk, not RAM.
pub fn disable_cache(file: &File) {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: valid open fd; F_NOCACHE only changes caching behaviour.
        unsafe {
            libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = file;
}

fn is_unsupported(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::Unsupported
        || matches!(e.raw_os_error(), Some(c) if c == libc::ENOTSUP || c == libc::EOPNOTSUPP || c == libc::ENOTTY || c == libc::EINVAL)
}

/// Push a file's data to stable storage as hard as the filesystem allows.
/// Local disks: a full flush. Network shares (SMB/NFS) often reject the full-flush request with
/// "Operation not supported" (os error 45), so fall back to plain fsync, and if even that is
/// unsupported the server owns durability and we continue; the checksum re-read still runs.
pub fn flush_best_effort(f: &File) -> std::io::Result<()> {
    flush_with(
        f,
        |f| f.sync_all(),
        |f| {
            use std::os::fd::AsRawFd;
            // SAFETY: valid open fd.
            if unsafe { libc::fsync(f.as_raw_fd()) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
        },
    )
}

fn flush_with(f: &File, full: impl Fn(&File) -> std::io::Result<()>, plain: impl Fn(&File) -> std::io::Result<()>) -> std::io::Result<()> {
    match full(f) {
        Err(e) if is_unsupported(&e) => match plain(f) {
            Err(e2) if is_unsupported(&e2) => Ok(()),
            other => other,
        },
        other => other,
    }
}

pub const BUF: usize = 2 * 1024 * 1024;

/// Hash a file with blake3. `nocache` bypasses the page cache for a true on-disk read.
/// `on_bytes` is called with each chunk length (used for progress); return false to stop
/// (the call then fails with `ErrorKind::Interrupted`).
pub fn hash_file(path: &Path, nocache: bool, mut on_bytes: impl FnMut(u64) -> bool) -> std::io::Result<(blake3::Hash, u64)> {
    let mut f = File::open(path)?;
    if nocache {
        disable_cache(&f);
    }
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; BUF];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
        if !on_bytes(n as u64) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "stopped"));
        }
    }
    Ok((h.finalize(), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_controls() {
        assert_eq!(sanitize_display("a\x1b[31mb\n"), "a\u{FFFD}[31mb\u{FFFD}");
    }

    #[test]
    fn formats() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500_000), "1.5 MB");
        assert_eq!(human_duration(75), "1m15s");
        assert_eq!(human_duration(3700), "1h01m");
    }

    #[test]
    fn volume_roots() {
        assert_eq!(volume_root(Path::new("/Volumes/SD_Card/DCIM/x")), Some(PathBuf::from("/Volumes/SD_Card")));
        assert_eq!(volume_root(Path::new("/tmp/x")), None);
        assert!(destination_volume_present(Path::new("/tmp/anything")));
        assert!(!destination_volume_present(Path::new("/Volumes/definitely-not-mounted-xyz/a")));
    }

    #[test]
    fn disk_space_works_on_missing_child() {
        let d = tempfile::tempdir().unwrap();
        let (total, free) = disk_space(&d.path().join("a/b/c")).unwrap();
        assert!(total > 0 && free <= total);
    }

    #[test]
    fn flush_falls_back_on_unsupported_but_reports_real_errors() {
        let d = tempfile::tempdir().unwrap();
        let f = File::create(d.path().join("f")).unwrap();
        let enotsup = || Err(std::io::Error::from_raw_os_error(libc::ENOTSUP));
        let eio = || Err(std::io::Error::from_raw_os_error(libc::EIO));
        // full flush unsupported (SMB) -> plain fsync used
        assert!(flush_with(&f, |_| enotsup(), |_| Ok(())).is_ok());
        // both unsupported -> tolerated
        assert!(flush_with(&f, |_| enotsup(), |_| enotsup()).is_ok());
        // a genuine I/O error is never swallowed
        assert!(flush_with(&f, |_| eio(), |_| Ok(())).is_err());
        assert!(flush_with(&f, |_| enotsup(), |_| eio()).is_err());
        assert!(flush_best_effort(&f).is_ok());
    }

    #[test]
    fn disk_space_agrees_with_df() {
        let d = tempfile::tempdir().unwrap();
        let (total, free) = disk_space(d.path()).unwrap();
        let out = std::process::Command::new("df").arg("-k").arg(d.path()).output().unwrap();
        let line = String::from_utf8_lossy(&out.stdout).lines().last().unwrap().to_string();
        let cols: Vec<&str> = line.split_whitespace().collect();
        let df_total: u64 = cols[1].parse::<u64>().unwrap() * 1024;
        let df_avail: u64 = cols[3].parse::<u64>().unwrap() * 1024;
        let close = |a: u64, b: u64| (a as i128 - b as i128).unsigned_abs() < (b as u128 / 50 + (64 << 20)); // within 2%
        assert!(close(total, df_total), "total {total} vs df {df_total}");
        assert!(close(free, df_avail), "free {free} vs df {df_avail}");
    }

    #[test]
    fn hashing_matches_blake3() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, b"hello world").unwrap();
        let mut seen = 0;
        let (h, n) = hash_file(&p, true, |b| {
            seen += b;
            true
        })
        .unwrap();
        assert_eq!(h, blake3::hash(b"hello world"));
        assert_eq!((n, seen), (11, 11));
    }
}
