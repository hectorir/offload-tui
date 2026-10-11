//! Non-interactive `offload run`: scriptable copies with plain-text output.

use crate::config::Config;
use crate::engine::{self, Control, Event, Outcome};
use crate::fsutil::{human_bytes, human_duration, human_rate, sanitize_display};
use crate::{history, scan};
use anyhow::{Result, bail};
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Returns the process exit code. One name copies one camera; several names (or `all`) copy a batch.
pub struct Options<'a> {
    /// Profile names (partial names work).
    pub names: &'a [String],
    /// Include every profile.
    pub all: bool,
    pub yes: bool,
    pub dry_run: bool,
}

pub fn run(cfg: &Config, opts: &Options, history_path: &Path, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<i32> {
    let Options { names, all, yes, dry_run } = *opts;
    // Resolve which cameras to include, in config order, without duplicates.
    let mut idxs: Vec<usize> = vec![];
    if all {
        idxs.extend(0..cfg.profiles.len());
    }
    for n in names {
        let Some(p) = cfg.find_profile(n) else {
            let avail: Vec<_> = cfg.profiles.iter().map(|p| p.name.as_str()).collect();
            bail!("no profile matching '{}'. Available: {}", n, avail.join(", "));
        };
        let i = cfg.profiles.iter().position(|q| std::ptr::eq(q, p)).unwrap_or(0);
        if !idxs.contains(&i) {
            idxs.push(i);
        }
    }
    if idxs.is_empty() {
        bail!("name at least one profile, or use --all");
    }
    idxs.sort_unstable();
    let batch = idxs.len() > 1;
    if batch {
        let names: Vec<_> = idxs.iter().map(|i| sanitize_display(&cfg.profiles[*i].name)).collect();
        writeln!(out, "Batch: {}", names.join(" + "))?;
    }

    let mut parts = vec![];
    for i in &idxs {
        let profile = &cfg.profiles[*i];
        if !batch {
            writeln!(out, "Profile: {}", sanitize_display(&profile.name))?;
        }
        let sc = scan::scan_profile(profile);
        if batch {
            writeln!(out, "{}", sanitize_display(&profile.name))?;
        }
        for s in &sc.sources {
            writeln!(
                out,
                "  {} {}{}",
                if s.present { "●" } else { "○" },
                sanitize_display(&s.path.display().to_string()),
                if s.present { format!("  ({} files)", s.files) } else { "  (not found, skipped)".into() }
            )?;
        }
        parts.push((*i, sc));
    }
    let scan = scan::merge_scans(parts);
    if scan.files.is_empty() {
        writeln!(out, "Nothing to copy.")?;
        return Ok(if scan.any_source_present() { 0 } else { 3 });
    }
    let batch_parts: Vec<engine::BatchPart> = idxs
        .iter()
        .map(|i| engine::BatchPart { profile_index: *i, profile: &cfg.profiles[*i], files: scan.files.iter().filter(|f| f.profile == *i).collect() })
        .filter(|p| !p.files.is_empty())
        .collect();
    let plan = engine::build_batch_plan(cfg, &batch_parts, chrono::Local::now())?;
    writeln!(out, "{} files, {} -> {}", plan.items.len(), human_bytes(plan.total_bytes()), sanitize_display(&plan.dest_dir.display().to_string()))?;
    if dry_run {
        for it in &plan.items {
            let tag = if batch { format!("[{}] ", sanitize_display(&cfg.profiles[it.group].name)) } else { String::new() };
            writeln!(out, "  would copy {tag}{}  ({})", sanitize_display(&it.name), human_bytes(it.size))?;
        }
        writeln!(out, "Dry run: nothing was written.")?;
        return Ok(0);
    }
    if let Err(e) = engine::preflight(&plan) {
        writeln!(out, "Error: {e}")?;
        return Ok(2);
    }
    if !yes {
        write!(out, "Copy and verify? [y/N] ")?;
        out.flush()?;
        let mut line = String::new();
        input.read_line(&mut line)?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            writeln!(out, "Aborted.")?;
            return Ok(1);
        }
    }

    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
    }
    let ctrl = Control::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let p2 = plan.clone();
    let c2 = ctrl.clone();
    let worker = std::thread::spawn(move || engine::run(&p2, &c2, tx));
    let total = plan.items.len();
    let mut done = 0;
    let mut outcomes: Vec<Option<Outcome>> = vec![None; total];
    let summary = loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            ctrl.cancel();
        }
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Event::Done { index, outcome }) => {
                done += 1;
                let name = sanitize_display(&plan.items[index].name);
                let name = if batch { format!("[{}] {name}", sanitize_display(&cfg.profiles[plan.items[index].group].name)) } else { name };
                outcomes[index] = Some(outcome.clone());
                let line = match outcome {
                    Outcome::Copied { renamed: true, .. } => format!("verified (name clashed, kept both)  {name}"),
                    Outcome::Copied { .. } => format!("verified   {name}"),
                    Outcome::Skipped { .. } => format!("skipped    {name} (identical copy already there)"),
                    Outcome::Failed(e) => format!("FAILED     {name}: {}", sanitize_display(&e)),
                    Outcome::Cancelled => format!("cancelled  {name}"),
                };
                writeln!(out, "[{done}/{total}] {line}")?;
            }
            Ok(Event::Finished(s)) => break s,
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => bail!("copy worker stopped unexpectedly"),
        }
    };
    let _ = worker.join();
    // One history record per camera (time split by data share so lifetime stats don't double-count).
    let mut per: Vec<(usize, usize, usize, usize, u64)> = idxs.iter().map(|i| (*i, 0, 0, 0, 0)).collect();
    for (it, st) in plan.items.iter().zip(&outcomes) {
        if let Some(e) = per.iter_mut().find(|e| e.0 == it.group) {
            match st {
                Some(Outcome::Copied { .. }) => {
                    e.1 += 1;
                    e.4 += it.size;
                }
                Some(Outcome::Skipped { .. }) => e.2 += 1,
                Some(Outcome::Failed(_)) => e.3 += 1,
                _ => {}
            }
        }
    }
    let all_bytes: u64 = per.iter().map(|e| e.4).sum();
    for (i, copied, skipped, failed, bytes) in &per {
        if plan.items.iter().all(|it| it.group != *i) {
            continue;
        }
        let share = if all_bytes > 0 { *bytes as f64 / all_bytes as f64 } else { 1.0 / per.len() as f64 };
        let dir = plan.items.iter().find(|it| it.group == *i).map(|it| it.dest_dir.display().to_string()).unwrap_or_default();
        let rec = history::Record {
            when: chrono::Local::now(),
            profile: cfg.profiles[*i].name.clone(),
            dest: dir,
            copied: *copied,
            skipped: *skipped,
            failed: *failed,
            bytes: *bytes,
            secs: (summary.elapsed.as_secs_f64() * share).round() as u64,
            cancelled: summary.cancelled,
        };
        let _ = history::append(history_path, &rec);
    }
    if batch {
        writeln!(out)?;
        for (i, copied, skipped, failed, bytes) in &per {
            writeln!(
                out,
                "  {:<20} {} copied ({}), {} skipped, {} failed",
                sanitize_display(&cfg.profiles[*i].name),
                copied,
                human_bytes(*bytes),
                skipped,
                failed
            )?;
        }
    }
    let rate = if summary.elapsed.as_secs_f64() > 0.5 {
        format!(" ({})", human_rate(summary.bytes_copied as f64 / summary.elapsed.as_secs_f64()))
    } else {
        String::new()
    };
    writeln!(
        out,
        "\n{} copied ({}), {} skipped, {} renamed, {} failed in {}{}",
        summary.copied,
        human_bytes(summary.bytes_copied),
        summary.skipped,
        summary.renamed,
        summary.failed,
        human_duration(summary.elapsed.as_secs()),
        rate
    )?;
    Ok(if summary.failed > 0 || summary.cancelled { 1 } else { 0 })
}
