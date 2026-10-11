//! Scanning camera sources for files a profile wants.

use crate::config::Profile;
use chrono::{DateTime, Local, NaiveDate};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    pub date: NaiveDate,
    /// Lowercased extension.
    pub ext: String,
    /// Index into the profile's sources.
    pub source: usize,
}

#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub path: PathBuf,
    pub present: bool,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub files: Vec<FileEntry>,
    pub sources: Vec<SourceStatus>,
    /// Count of files skipped because they were symlinks or macOS `._` metadata.
    pub ignored: usize,
}

impl ScanResult {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
    pub fn any_source_present(&self) -> bool {
        self.sources.iter().any(|s| s.present)
    }
    pub fn by_ext(&self) -> BTreeMap<String, (usize, u64)> {
        let mut m = BTreeMap::new();
        for f in &self.files {
            let e = m.entry(f.ext.clone()).or_insert((0, 0));
            e.0 += 1;
            e.1 += f.size;
        }
        m
    }
    pub fn by_date(&self) -> BTreeMap<NaiveDate, (usize, u64)> {
        let mut m = BTreeMap::new();
        for f in &self.files {
            let e = m.entry(f.date).or_insert((0, 0));
            e.0 += 1;
            e.1 += f.size;
        }
        m
    }
}

pub fn scan_profile(p: &Profile) -> ScanResult {
    let mut res = ScanResult::default();
    for (i, src) in p.source_paths().into_iter().enumerate() {
        let mut st = SourceStatus { path: src.clone(), present: src.is_dir(), files: 0, bytes: 0 };
        if st.present {
            scan_dir(p, &src, i, &mut res, &mut st);
        }
        res.sources.push(st);
    }
    res.files.sort_by(|a, b| (a.modified, &a.path).cmp(&(b.modified, &b.path)));
    res
}

fn scan_dir(p: &Profile, dir: &Path, source: usize, res: &mut ScanResult, st: &mut SourceStatus) {
    // follow_links(false): symlinks are reported as symlinks and never traversed.
    for ent in WalkDir::new(dir).follow_links(false).into_iter().filter_map(Result::ok) {
        let ft = ent.file_type();
        if ft.is_symlink() {
            res.ignored += 1;
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let name = ent.file_name().to_string_lossy();
        if name.starts_with("._") || name == ".DS_Store" {
            res.ignored += 1;
            continue;
        }
        let Some(ext) = ent.path().extension().map(|e| e.to_string_lossy().to_lowercase()) else { continue };
        if !p.wants_ext(&ext) {
            continue;
        }
        let Ok(meta) = ent.metadata() else { continue };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let date = DateTime::<Local>::from(modified).date_naive();
        st.files += 1;
        st.bytes += meta.len();
        res.files.push(FileEntry { path: ent.path().to_path_buf(), size: meta.len(), modified, date, ext, source });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::fs;

    fn profile(src: &Path) -> Profile {
        let mut p = Config::default().profiles[0].clone();
        p.sources = vec![src.to_string_lossy().into()];
        p
    }

    #[test]
    fn filters_by_extension_case_insensitively_and_skips_lrf() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("sub/deeper")).unwrap();
        for f in ["a.MP4", "b.mp4", "c.WAV", "d.LRF", "e.lrf", "f.jpg", "sub/g.mp4", "sub/deeper/h.Wav", "noext"] {
            fs::write(d.path().join(f), b"x").unwrap();
        }
        let r = scan_profile(&profile(d.path()));
        let mut names: Vec<_> = r.files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, ["a.MP4", "b.mp4", "c.WAV", "g.mp4", "h.Wav"]);
    }

    #[test]
    fn ignores_appledouble_symlinks_and_reports_missing_sources() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("real.mp4"), b"1234").unwrap();
        fs::write(d.path().join("._real.mp4"), b"junk").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.mp4"), b"nope").unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("linkdir")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.mp4"), d.path().join("link.mp4")).unwrap();
        let mut p = profile(d.path());
        p.sources.push("/definitely/not/here".into());
        let r = scan_profile(&p);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.ignored, 3);
        assert!(r.sources[0].present && !r.sources[1].present);
        assert!(r.any_source_present());
        assert_eq!(r.total_bytes(), 4);
    }

    #[test]
    fn breakdowns() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.mp4"), b"12").unwrap();
        fs::write(d.path().join("b.mp4"), b"123").unwrap();
        fs::write(d.path().join("c.wav"), b"1").unwrap();
        let r = scan_profile(&profile(d.path()));
        let e = r.by_ext();
        assert_eq!(e["mp4"], (2, 5));
        assert_eq!(e["wav"], (1, 1));
        assert_eq!(r.by_date().values().map(|v| v.0).sum::<usize>(), 3);
    }
}
