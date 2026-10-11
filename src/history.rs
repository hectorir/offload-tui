//! Append-only JSON-lines history of offload sessions, plus lifetime stats.

use crate::config::data_dir;
use crate::engine::Summary;
use anyhow::Result;
use chrono::{DateTime, Datelike, Local};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub when: DateTime<Local>,
    pub profile: String,
    pub dest: String,
    pub copied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub bytes: u64,
    pub secs: u64,
    #[serde(default)]
    pub cancelled: bool,
}

impl Record {
    pub fn from_summary(profile: &str, s: &Summary, when: DateTime<Local>) -> Record {
        Record {
            when,
            profile: profile.to_string(),
            dest: s.dest_dir.display().to_string(),
            copied: s.copied,
            skipped: s.skipped,
            failed: s.failed,
            bytes: s.bytes_copied,
            secs: s.elapsed.as_secs(),
            cancelled: s.cancelled,
        }
    }
}

pub fn default_path() -> PathBuf {
    data_dir().join("history.jsonl")
}

pub fn append(path: &Path, rec: &Record) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(rec)?)?;
    Ok(())
}

/// Load all records, newest first. Corrupt lines are skipped, never fatal.
pub fn load(path: &Path) -> Vec<Record> {
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    let mut v: Vec<Record> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    v.reverse();
    v
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub sessions: usize,
    pub files: usize,
    pub bytes: u64,
    pub secs: u64,
    pub by_month: BTreeMap<(i32, u32), u64>,
    pub by_profile: BTreeMap<String, u64>,
}

pub fn stats(records: &[Record]) -> Stats {
    let mut s = Stats::default();
    for r in records {
        s.sessions += 1;
        s.files += r.copied;
        s.bytes += r.bytes;
        s.secs += r.secs;
        *s.by_month.entry((r.when.year(), r.when.month())).or_default() += r.bytes;
        *s.by_profile.entry(r.profile.clone()).or_default() += r.bytes;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(profile: &str, bytes: u64) -> Record {
        Record { when: Local::now(), profile: profile.into(), dest: "/d".into(), copied: 2, skipped: 1, failed: 0, bytes, secs: 10, cancelled: false }
    }

    #[test]
    fn append_load_newest_first_and_skip_corrupt() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x/h.jsonl");
        append(&p, &rec("A", 1)).unwrap();
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"not json\n").unwrap();
        append(&p, &rec("B", 2)).unwrap();
        let v = load(&p);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].profile, "B");
        assert!(load(&d.path().join("missing")).is_empty());
    }

    #[test]
    fn stats_aggregate() {
        let s = stats(&[rec("A", 100), rec("A", 50), rec("B", 25)]);
        assert_eq!((s.sessions, s.files, s.bytes, s.secs), (3, 6, 175, 30));
        assert_eq!(s.by_profile["A"], 150);
        assert_eq!(s.by_month.values().sum::<u64>(), 175);
    }
}
