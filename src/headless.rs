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

/// Returns the process exit code.
pub fn run(cfg: &Config, profile_name: &str, yes: bool, dry_run: bool, history_path: &Path, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<i32> {
    let Some(profile) = cfg.find_profile(profile_name) else {
        let names: Vec<_> = cfg.profiles.iter().map(|p| p.name.as_str()).collect();
        bail!("no profile matching '{}'. Available: {}", profile_name, names.join(", "));
    };
    writeln!(out, "Profile: {}", sanitize_display(&profile.name))?;
    let scan = scan::scan_profile(profile);
    for s in &scan.sources {
        writeln!(
            out,
            "  {} {}{}",
            if s.present { "●" } else { "○" },
            sanitize_display(&s.path.display().to_string()),
            if s.present { format!("  ({} files)", s.files) } else { "  (not found, skipped)".into() }
        )?;
    }
    if scan.files.is_empty() {
        writeln!(out, "Nothing to copy.")?;
        return Ok(if scan.any_source_present() { 0 } else { 3 });
    }
    let refs: Vec<_> = scan.files.iter().collect();
    let plan = engine::build_plan(cfg, profile, &refs, chrono::Local::now())?;
    writeln!(out, "{} files, {} -> {}", plan.items.len(), human_bytes(plan.total_bytes()), sanitize_display(&plan.dest_dir.display().to_string()))?;
    if dry_run {
        for it in &plan.items {
            writeln!(out, "  would copy {}  ({})", sanitize_display(&it.name), human_bytes(it.size))?;
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
    let summary = loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            ctrl.cancel();
        }
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Event::Done { index, outcome }) => {
                done += 1;
                let name = sanitize_display(&plan.items[index].name);
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
    let _ = history::append(history_path, &history::Record::from_summary(&profile.name, &summary, chrono::Local::now()));
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
