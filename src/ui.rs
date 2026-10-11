//! Rendering. Pure functions of `App` state (plus recording click targets).

use crate::app::*;
use crate::engine::{Mode, Removal};
use crate::fsutil::{human_bytes, human_duration, human_rate, sanitize_display, volume_root};
use crate::theme::Theme;
use crate::widgets::{self, WORDMARK, gradient_bar, solid_bar, spinner};
use chrono::{DateTime, Datelike, Local};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Sparkline, Wrap};
use std::path::Path;

pub fn draw(f: &mut Frame, app: &mut App) {
    app.hits.clear();
    let full = f.area();
    let width = full.width.min(132);
    let area = Rect { x: full.x + (full.width - width) / 2, y: full.y, width, height: full.height };
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    let body = body.inner(Margin { horizontal: 1, vertical: 0 });
    match app.screen {
        Screen::Home => draw_home(f, app, body),
        Screen::Scanning => draw_scanning(f, app, body),
        Screen::Review => draw_review(f, app, body),
        Screen::Running => draw_running(f, app, body),
        Screen::Summary => draw_summary(f, app, body),
        Screen::Config => draw_config(f, app, body),
        Screen::Stats => draw_stats(f, app, body),
    }
    draw_footer(f, app, footer);
    if app.show_help {
        draw_help(f, app, full);
    }
}

// ---------- shared bits ----------

fn block<'a>(t: &Theme, title: impl Into<Line<'a>>) -> Block<'a> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(t.border)).title(title.into())
}

fn title<'a>(t: &Theme, s: &str) -> Line<'a> {
    Line::from(vec![Span::raw(" "), Span::styled(s.to_string(), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)), Span::raw(" ")])
}

fn trunc(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else if n <= 1 {
        "…".into()
    } else {
        format!("{}…", c[..n - 1].iter().collect::<String>())
    }
}

/// Shorten a path from the middle, keeping the end visible.
fn trunc_path(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        return s.to_string();
    }
    if n <= 4 {
        return "…".into();
    }
    let keep = n - 1;
    let head = keep / 3;
    let tail = keep - head;
    format!("{}…{}", c[..head].iter().collect::<String>(), c[c.len() - tail..].iter().collect::<String>())
}

fn dim(t: &Theme) -> Style {
    Style::default().fg(t.dim)
}
fn fg(t: &Theme) -> Style {
    Style::default().fg(t.fg)
}
fn bold(t: &Theme) -> Style {
    Style::default().fg(t.fg).add_modifier(Modifier::BOLD)
}

fn hints<'a>(t: &Theme, items: &[(&str, &str)]) -> Line<'a> {
    let mut spans = vec![];
    for (i, (k, d)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", dim(t)));
        }
        spans.push(Span::styled(k.to_string(), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {d}"), dim(t)));
    }
    Line::from(spans)
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let line = match app.screen {
        Screen::Home => {
            let mut h = vec![("↑↓", "choose"), ("enter", "start")];
            if app.has_batch() {
                h.push(("b", "batch all"));
            }
            h.extend([("e", "settings"), ("s", "stats"), ("t", "theme"), ("?", "help"), ("q", "quit")]);
            hints(t, &h)
        }
        Screen::Scanning => hints(t, &[("esc", "cancel")]),
        Screen::Review => match &app.review {
            Some(rv) if rv.confirm_move => Line::from(Span::styled(
                "Move = delete originals after verifying.  y = start   any other key = back",
                Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
            )),
            Some(rv) => hints(
                t,
                &[
                    ("space", "toggle"),
                    ("a", "all/none"),
                    ("tab", "dates"),
                    ("m", if rv.mode == Mode::Move { "mode: MOVE" } else { "mode: copy" }),
                    ("enter", "go"),
                    ("esc", "back"),
                    ("?", "help"),
                ],
            ),
            None => Line::from(""),
        },
        Screen::Running => match &app.run {
            Some(r) if r.confirm_cancel => Line::from(Span::styled(
                "Cancel and discard unfinished files?  y = yes   any other key = keep going",
                Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
            )),
            Some(r) if r.ctrl.is_paused() => hints(t, &[("p", "resume"), ("esc", "cancel")]),
            _ => hints(t, &[("p", "pause"), ("↑↓", "scroll"), ("f", "follow"), ("esc", "cancel")]),
        },
        Screen::Summary => {
            if let Some(v) = &app.eject_confirm {
                let names: Vec<_> = v.iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect();
                Line::from(Span::styled(
                    format!("Eject {}?  y = yes   any other key = no", names.join(", ")),
                    Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
                ))
            } else if app.delete_confirm {
                let bytes: u64 = app.originals.iter().map(|o| o.size).sum();
                Line::from(Span::styled(
                    format!(
                        "PERMANENTLY delete {} originals ({}) from the card? Cannot be undone.  y = delete   any other key = keep",
                        app.originals.len(),
                        human_bytes(bytes)
                    ),
                    Style::default().fg(t.err).add_modifier(Modifier::BOLD),
                ))
            } else {
                let mut h = vec![("o", "open folder")];
                if app.run_mode == Mode::Copy && !app.originals.is_empty() && app.deleted.is_none() {
                    h.push(("d", "delete originals"));
                }
                h.extend([("x", "eject card"), ("s", "stats"), ("enter", "home"), ("q", "quit")]);
                hints(t, &h)
            }
        }
        Screen::Config => {
            if app.editor.input.is_some() {
                hints(t, &[("enter", "save"), ("esc", "cancel"), ("ctrl+u", "clear")])
            } else if app.editor.confirm_delete.is_some() {
                Line::from(Span::styled("Delete this profile?  y = yes   any other key = no", Style::default().fg(t.warn).add_modifier(Modifier::BOLD)))
            } else {
                hints(t, &[("↑↓", "move"), ("enter", "edit"), ("d", "delete profile"), ("esc", "back")])
            }
        }
        Screen::Stats => hints(t, &[("↑↓", "scroll"), ("esc", "back")]),
    };
    let (left, right) = if let Some(toast) = &app.toast {
        let c = match toast.level {
            Level::Info => t.accent,
            Level::Ok => t.ok,
            Level::Warn => t.warn,
            Level::Err => t.err,
        };
        let txt = trunc(&sanitize_display(&toast.text), area.width as usize / 2);
        (area.width.saturating_sub(txt.chars().count() as u16 + 2), Some(Span::styled(format!(" {txt} "), Style::default().fg(c).add_modifier(Modifier::BOLD))))
    } else {
        (area.width, None)
    };
    let l = Rect { width: left, ..area };
    f.render_widget(Paragraph::new(line), l);
    if let Some(span) = right {
        let r = Rect { x: area.x + left, width: area.width - left, ..area };
        f.render_widget(Paragraph::new(Line::from(span)).alignment(Alignment::Right), r);
    }
}

fn storage_line<'a>(t: &Theme, label: &str, d: &DestLive, width: usize) -> Line<'a> {
    match (d.mounted, d.space) {
        (false, _) => {
            Line::from(vec![Span::styled("○ ", Style::default().fg(t.err)), Span::styled(format!("{label}: not mounted"), Style::default().fg(t.err))])
        }
        (true, Some((total, free))) => {
            let used = total.saturating_sub(free);
            let ratio = if total > 0 { used as f64 / total as f64 } else { 0.0 };
            let bar_w = width.saturating_sub(label.chars().count() + 38).clamp(6, 40);
            let mut spans = vec![Span::styled("● ", Style::default().fg(t.ok)), Span::styled(format!("{label}  "), bold(t))];
            spans.push(solid_bar(bar_w, ratio, if ratio > 0.9 { t.warn } else { t.accent }));
            let fill = bar_w.saturating_sub(spans.last().map(|s| s.content.chars().count()).unwrap_or(0));
            spans.push(Span::styled("░".repeat(fill), Style::default().fg(t.border)));
            spans.push(Span::styled(format!("  {} free of {}", human_bytes(free), human_bytes(total)), fg(t)));
            Line::from(spans)
        }
        (true, None) => Line::from(vec![Span::styled("● ", Style::default().fg(t.ok)), Span::styled(label.to_string(), bold(t))]),
    }
}

fn source_label(p: &Path) -> String {
    volume_root(p)
        .and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()))
}

// ---------- home ----------

fn draw_home(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let [head, mid, dest] = Layout::vertical([Constraint::Length(5), Constraint::Min(6), Constraint::Length(4)]).areas(area);

    // header
    let mut lines: Vec<Line> = WORDMARK
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let c = crate::theme::lerp(t.grad.0, t.grad.1, i as f32 / 2.0);
            Line::from(Span::styled(*l, Style::default().fg(c).add_modifier(Modifier::BOLD)))
        })
        .collect();
    lines.push(Line::from(Span::styled("verified camera offload", dim(&t))));
    f.render_widget(Paragraph::new(lines), head);
    let right = Line::from(vec![Span::styled("theme ", dim(&t)), Span::styled(t.name, Style::default().fg(t.accent2))]);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), Rect { height: 1, ..head });
    if !app.config_exists {
        let note = Line::from(Span::styled("using built-in 'hectorir' profiles · press e to customize", dim(&t)));
        f.render_widget(Paragraph::new(note).alignment(Alignment::Right), Rect { y: head.y + 1, height: 1, ..head });
    }

    // profile cards (+ the batch card when there are two or more cameras)
    const CARD_H: u16 = 7;
    let n_prof = app.cfg.profiles.len();
    let total_items = app.home_items();
    let visible = (mid.height / CARD_H).max(1) as usize;
    if app.home_sel < app.home_scroll {
        app.home_scroll = app.home_sel;
    } else if app.home_sel >= app.home_scroll + visible {
        app.home_scroll = app.home_sel + 1 - visible;
    }
    for (slot, idx) in (app.home_scroll..total_items).take(visible).enumerate() {
        let r = Rect { x: mid.x, y: mid.y + slot as u16 * CARD_H, width: mid.width, height: CARD_H.min(mid.height.saturating_sub(slot as u16 * CARD_H)) };
        if r.height < 3 {
            break;
        }
        let selected = idx == app.home_sel;
        if idx == n_prof {
            draw_batch_card(f, app, r, selected);
            app.hits.push((r, Hit::Profile(idx)));
            continue;
        }
        let p = &app.cfg.profiles[idx];
        let live = app.live.get(idx).cloned().unwrap_or_default();
        let srcs = p.source_paths();
        let n_present = live.present.iter().filter(|x| **x).count();
        let (status, status_c) = if n_present == 0 {
            ("○ no card detected", t.dim)
        } else if n_present == srcs.len() {
            ("● ready", t.ok)
        } else {
            ("◐ partly connected", t.warn)
        };
        let border = if selected { t.accent } else { t.border };
        let blk = Block::bordered()
            .border_type(if selected { BorderType::Thick } else { BorderType::Rounded })
            .border_style(Style::default().fg(border))
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{}", idx + 1), Style::default().fg(t.accent2).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
            ]));
        let inner = blk.inner(r);
        f.render_widget(blk, r);
        let art = widgets::logo(&p.icon);
        let lw = widgets::logo_width(&art) as u16;
        f.render_widget(Paragraph::new(widgets::logo_lines(&art, &t, selected)), Rect { x: inner.x + 1, width: lw.min(inner.width), ..inner });
        let tx = inner.x + lw + 3;
        let tw = inner.width.saturating_sub(lw + 4);
        let mut src_spans = vec![];
        for (i, s) in srcs.iter().enumerate() {
            let on = live.present.get(i).copied().unwrap_or(false);
            src_spans.push(Span::styled(if on { "● " } else { "○ " }, Style::default().fg(if on { t.ok } else { t.dim })));
            src_spans.push(Span::styled(format!("{}   ", trunc(&sanitize_display(&source_label(s)), 22)), if on { fg(&t) } else { dim(&t) }));
        }
        let incl = p.include.iter().map(|e| e.to_uppercase()).collect::<Vec<_>>().join(" ");
        let excl = if p.exclude.is_empty() {
            String::new()
        } else {
            format!("   never: {}", p.exclude.iter().map(|e| e.to_uppercase()).collect::<Vec<_>>().join(" "))
        };
        let dest_name = if p.subfolder.is_empty() { "<date>/".to_string() } else { format!("<date>/{}/", p.subfolder) };
        let rows = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(trunc(&sanitize_display(&p.name), tw as usize / 2), bold(&t).add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::styled(status, Style::default().fg(status_c)),
            ]),
            Line::from(src_spans),
            Line::from(vec![Span::styled("copies ", dim(&t)), Span::styled(incl, fg(&t)), Span::styled(excl, dim(&t))]),
            Line::from(vec![Span::styled("into   ", dim(&t)), Span::styled(sanitize_display(&dest_name), fg(&t))]),
        ];
        f.render_widget(Paragraph::new(rows), Rect { x: tx, y: inner.y, width: tw, height: inner.height });
        app.hits.push((r, Hit::Profile(idx)));
    }
    if total_items > visible {
        let s = format!("{}/{}", app.home_sel + 1, total_items);
        f.render_widget(
            Paragraph::new(Span::styled(s, dim(&t))).alignment(Alignment::Right),
            Rect { y: mid.y + mid.height.saturating_sub(1), height: 1, ..mid },
        );
    }

    // destination
    let blk = block(&t, title(&t, "Backup destination"));
    let inner = blk.inner(dest);
    f.render_widget(blk, dest);
    let base = app.cfg.destination_path();
    let lines = vec![
        storage_line(&t, &trunc(&source_label(&base), 24), &app.dest_live, inner.width as usize),
        Line::from(Span::styled(trunc_path(&sanitize_display(&base.display().to_string()), inner.width as usize), dim(&t))),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

/// Short label for a camera in tags and titles: its subfolder name, else its name.
fn short_name(p: &crate::config::Profile) -> String {
    let s = if p.subfolder.is_empty() { &p.name } else { &p.subfolder };
    sanitize_display(s)
}

fn profile_color(t: &Theme, slot: usize) -> ratatui::style::Color {
    [t.accent, t.accent2, t.ok, t.warn][slot % 4]
}

fn draw_batch_card(f: &mut Frame, app: &App, r: Rect, selected: bool) {
    let t = app.theme;
    let n = app.cfg.profiles.len();
    let connected = app.connected_profiles();
    let (status, status_c) = if connected.len() == n {
        ("● all ready".to_string(), t.ok)
    } else if connected.is_empty() {
        ("○ no cards detected".to_string(), t.dim)
    } else {
        (format!("◐ {} of {} ready", connected.len(), n), t.warn)
    };
    let border = if selected { t.accent2 } else { t.border };
    let blk = Block::bordered().border_type(if selected { BorderType::Thick } else { BorderType::Rounded }).border_style(Style::default().fg(border)).title(
        Line::from(vec![Span::raw(" "), Span::styled("b", Style::default().fg(t.accent2).add_modifier(Modifier::BOLD)), Span::styled(" · batch ", dim(&t))]),
    );
    let inner = blk.inner(r);
    f.render_widget(blk, r);
    let art = widgets::logo_batch();
    let lw = widgets::logo_width(&art) as u16;
    f.render_widget(Paragraph::new(widgets::logo_lines(&art, &t, selected)), Rect { x: inner.x + 1, width: lw.min(inner.width), ..inner });
    let tx = inner.x + lw + 3;
    let tw = inner.width.saturating_sub(lw + 4);
    let names: Vec<String> = app.cfg.profiles.iter().map(short_name).collect();
    let mut cams = vec![];
    for (i, p) in app.cfg.profiles.iter().enumerate() {
        let on = connected.contains(&i);
        cams.push(Span::styled(if on { "● " } else { "○ " }, Style::default().fg(if on { t.ok } else { t.dim })));
        cams.push(Span::styled(format!("{}   ", trunc(&sanitize_display(&p.name), 24)), if on { fg(&t) } else { dim(&t) }));
    }
    let into: Vec<String> = app.cfg.profiles.iter().map(|p| if p.subfolder.is_empty() { "<date>/".to_string() } else { format!("{}/", p.subfolder) }).collect();
    let rows = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(trunc(&format!("Batch · {}", names.join(" + ")), tw as usize / 2), bold(&t)),
            Span::raw("   "),
            Span::styled(status, Style::default().fg(status_c)),
        ]),
        Line::from(cams),
        Line::from(Span::styled("one review · one copy · every file verified", dim(&t))),
        Line::from(vec![
            Span::styled("into   ", dim(&t)),
            Span::styled(trunc(&sanitize_display(&format!("<date>/ {}", into.join(" + "))), tw as usize), fg(&t)),
        ]),
    ];
    f.render_widget(Paragraph::new(rows), Rect { x: tx, y: inner.y, width: tw, height: inner.height });
}

// ---------- scanning ----------

fn draw_scanning(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let name = app.scan_profiles.iter().filter_map(|i| app.cfg.profiles.get(*i)).map(|p| p.name.clone()).collect::<Vec<_>>().join(" + ");
    let mid = Rect { y: area.y + area.height / 3, height: 3, ..area };
    let lines = vec![
        Line::from(vec![
            Span::styled(spinner(app.tick_count / 2), Style::default().fg(t.accent)),
            Span::styled(format!("  Scanning {} …", sanitize_display(&name)), bold(&t)),
        ]),
        Line::from(Span::styled("looking for files on the card", dim(&t))),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), mid);
}

// ---------- review ----------

fn check(sel: usize, tot: usize) -> &'static str {
    if tot > 0 && sel == tot {
        "[x]"
    } else if sel == 0 {
        "[ ]"
    } else {
        "[~]"
    }
}

fn draw_review(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let Some(rv) = &app.review else { return };
    let [left, right] = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(area);
    let [files_a, peek_a] = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(left);

    // files
    let names: Vec<String> = rv.profiles.iter().filter_map(|i| app.cfg.profiles.get(*i)).map(short_name).collect();
    let pname = if rv.is_batch() {
        format!("Batch · {}", names.join(" + "))
    } else {
        rv.profiles.first().and_then(|i| app.cfg.profiles.get(*i)).map(|p| sanitize_display(&p.name)).unwrap_or_default()
    };
    let blk = block(
        &t,
        Line::from(vec![
            Span::raw(" "),
            Span::styled(pname, Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" · {} of {} selected ", rv.selected_count(), rv.scan.files.len()), dim(&t)),
        ]),
    );
    let inner = blk.inner(files_a);
    f.render_widget(blk, files_a);
    let h = inner.height as usize;
    let mut hits = vec![];
    let mut lines = vec![];
    if rv.scan.files.is_empty() {
        lines.push(Line::from(Span::styled("No matching files found on the connected cards.", Style::default().fg(t.warn))));
        lines.push(Line::from(Span::styled("Check the card is mounted and the source paths in settings (e).", dim(&t))));
    }
    for (row, i) in (rv.scroll..rv.scan.files.len()).take(h).enumerate() {
        let fe = &rv.scan.files[i];
        let on = rv.selected[i];
        let name = sanitize_display(&fe.path.file_name().unwrap_or_default().to_string_lossy());
        let is_cur = i == rv.cursor && rv.focus == Focus::Files;
        let namew = (inner.width as usize).saturating_sub(if rv.is_batch() { 40 } else { 34 });
        let mut style = if on { fg(&t) } else { dim(&t) };
        if is_cur {
            style = style.bg(t.sel_bg).add_modifier(Modifier::BOLD);
        }
        let mark = if rv.present[i] { Span::styled(" on dest", Style::default().fg(t.ok)) } else { Span::raw("") };
        let date = DateTime::<Local>::from(fe.modified).format("%b %d %H:%M").to_string();
        let mut spans = vec![Span::styled(if on { "[x] " } else { "[ ] " }, if on { Style::default().fg(t.ok) } else { dim(&t) })];
        if rv.is_batch() {
            let slot = rv.profiles.iter().position(|p| *p == fe.profile).unwrap_or(0);
            let tag = app.cfg.profiles.get(fe.profile).map(short_name).unwrap_or_default();
            spans.push(Span::styled(format!("{:<5} ", trunc(&tag, 5)), Style::default().fg(profile_color(&t, slot)).add_modifier(Modifier::BOLD)));
        }
        spans.extend([
            Span::styled(format!("{:<w$}", trunc(&name, namew), w = namew), style),
            Span::styled(format!(" {:>9}", human_bytes(fe.size)), style),
            Span::styled(format!("  {date}"), if is_cur { style } else { dim(&t) }),
        ]);
        spans.push(mark);
        lines.push(Line::from(spans));
        hits.push((Rect { x: inner.x, y: inner.y + row as u16, width: inner.width, height: 1 }, Hit::File(i)));
    }
    f.render_widget(Paragraph::new(lines), inner);

    // peek
    let blk = block(&t, title(&t, "Details"));
    let pi = blk.inner(peek_a);
    f.render_widget(blk, peek_a);
    let mut pl = vec![];
    if let Some(fe) = rv.scan.files.get(rv.cursor) {
        let full = sanitize_display(&fe.path.display().to_string());
        pl.push(Line::from(Span::styled(trunc_path(&full, pi.width as usize), fg(&t))));
        let m = DateTime::<Local>::from(fe.modified);
        pl.push(Line::from(vec![
            Span::styled(format!("{} · {} · ", fe.ext.to_uppercase(), human_bytes(fe.size)), fg(&t)),
            Span::styled(m.format("%a %b %d %Y, %H:%M:%S").to_string(), fg(&t)),
        ]));
        pl.push(Line::from(Span::styled(
            format!(
                "from {}{}",
                sanitize_display(&source_label(&rv.scan.sources[fe.source].path)),
                if rv.present[rv.cursor] { " · already at destination (size match)" } else { "" }
            ),
            dim(&t),
        )));
    }
    f.render_widget(Paragraph::new(pl), pi);

    // right column
    let n_src = rv.scan.sources.len() as u16;
    let n_types = rv.scan.by_ext().len() as u16;
    let cams_h = if rv.is_batch() { rv.profiles.len() as u16 + 2 } else { 0 };
    let [sum_a, cams_a, src_a, typ_a, date_a] = Layout::vertical([
        Constraint::Length(9),
        Constraint::Length(cams_h),
        Constraint::Length(n_src + 2),
        Constraint::Length(n_types.max(1) + 2),
        Constraint::Min(4),
    ])
    .areas(right);

    let blk = block(&t, title(&t, "Ready to copy"));
    let si = blk.inner(sum_a);
    f.render_widget(blk, sum_a);
    let need = rv.need_bytes();
    let mut sl = vec![
        Line::from(vec![
            Span::styled(format!("{} files", rv.selected_count()), bold(&t)),
            Span::styled(format!("  ·  {}", human_bytes(rv.selected_bytes())), fg(&t)),
        ]),
        Line::from(vec![
            Span::styled("to write  ", dim(&t)),
            Span::styled(human_bytes(need), if rv.enough_space() { fg(&t) } else { Style::default().fg(t.err) }),
            Span::styled(format!("   {} already on dest", rv.already_count()), dim(&t)),
        ]),
        Line::from(vec![
            Span::styled("into      ", dim(&t)),
            Span::styled(sanitize_display(&trunc_path(&rv.dest_dir.display().to_string(), si.width.saturating_sub(10) as usize)), fg(&t)),
        ]),
        storage_line(&t, "backup", &rv.dest, si.width as usize),
    ];
    sl.push(Line::from(match rv.mode {
        Mode::Copy => vec![
            Span::styled("mode      ", dim(&t)),
            Span::styled("COPY", bold(&t)),
            Span::styled(" · originals stay on the card", dim(&t)),
            Span::styled("  (m)", dim(&t)),
        ],
        Mode::Move => vec![
            Span::styled("mode      ", dim(&t)),
            Span::styled("MOVE", Style::default().fg(t.warn).add_modifier(Modifier::BOLD)),
            Span::styled(" · originals deleted once verified", Style::default().fg(t.warn)),
            Span::styled("  (m)", dim(&t)),
        ],
    }));
    if rv.confirm_move {
        sl.push(Line::from(Span::styled(
            "⚠ MOVE deletes each original from the card right after its copy is verified.  y = start moving · any other key = back",
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        )));
    } else if let Some(e) = &rv.error {
        sl.push(Line::from(Span::styled(format!("⚠ {e}"), Style::default().fg(t.err).add_modifier(Modifier::BOLD))));
    } else if !rv.dest.mounted {
        sl.push(Line::from(Span::styled("⚠ backup drive not mounted", Style::default().fg(t.err))));
    } else if !rv.enough_space() {
        sl.push(Line::from(Span::styled("⚠ not enough free space", Style::default().fg(t.err))));
    } else {
        let go = if rv.mode == Mode::Move { "press enter to move (copy + verify + delete original)" } else { "press enter to copy + verify" };
        sl.push(Line::from(Span::styled(go, Style::default().fg(t.ok))));
    }
    f.render_widget(Paragraph::new(sl).wrap(Wrap { trim: true }), si);

    if rv.is_batch() {
        let blk = block(&t, title(&t, "Cameras in this batch"));
        let ci = blk.inner(cams_a);
        f.render_widget(blk, cams_a);
        let cl: Vec<Line> = rv
            .profiles
            .iter()
            .enumerate()
            .filter_map(|(slot, pi)| {
                let p = app.cfg.profiles.get(*pi)?;
                let (sel, tot, bytes) = rv.profile_counts(*pi);
                Some(Line::from(vec![
                    Span::styled("● ", Style::default().fg(profile_color(&t, slot))),
                    Span::styled(format!("{:<8}", trunc(&short_name(p), 8)), bold(&t)),
                    Span::styled(format!("{sel:>3}/{tot:<3} files  {:>9}", human_bytes(bytes)), if sel > 0 { fg(&t) } else { dim(&t) }),
                ]))
            })
            .collect();
        f.render_widget(Paragraph::new(cl), ci);
    }

    let blk = block(&t, title(&t, "Sources"));
    let ii = blk.inner(src_a);
    f.render_widget(blk, src_a);
    let sl: Vec<Line> = rv
        .scan
        .sources
        .iter()
        .map(|s| {
            let (dot, c) = if s.present { ("● ", t.ok) } else { ("○ ", t.warn) };
            let info = if s.present { format!("  {} files", s.files) } else { "  not found - skipped".into() };
            Line::from(vec![
                Span::styled(dot, Style::default().fg(c)),
                Span::styled(sanitize_display(&trunc_path(&s.path.display().to_string(), (ii.width as usize).saturating_sub(info.len() + 2))), fg(&t)),
                Span::styled(info, dim(&t)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(sl), ii);

    let blk = block(&t, title(&t, "By type"));
    let ti = blk.inner(typ_a);
    f.render_widget(blk, typ_a);
    let by = rv.scan.by_ext();
    let maxb = by.values().map(|v| v.1).max().unwrap_or(1).max(1);
    let tl: Vec<Line> = by
        .iter()
        .enumerate()
        .map(|(i, (e, (n, b)))| {
            let c = if i % 2 == 0 { t.accent } else { t.accent2 };
            let bar_w = (ti.width as usize).saturating_sub(26).clamp(4, 30);
            Line::from(vec![
                Span::styled(format!("{:<5}", e.to_uppercase()), bold(&t)),
                Span::styled(format!("{n:>4}  {:>9}  ", human_bytes(*b)), fg(&t)),
                solid_bar(bar_w, *b as f64 / maxb as f64, c),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(tl), ti);

    let focus_dates = rv.focus == Focus::Dates;
    let blk = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focus_dates { t.accent } else { t.border }))
        .title(title(&t, "Shoot dates  (tab)"));
    let di = blk.inner(date_a);
    f.render_widget(blk, date_a);
    let dh = di.height as usize;
    let dscroll = rv.date_cursor.saturating_sub(dh.saturating_sub(1));
    let by_date = rv.scan.by_date();
    let mut dl = vec![];
    for (row, i) in (dscroll..rv.dates.len()).take(dh).enumerate() {
        let d = rv.dates[i];
        let (sel, tot) = rv.date_state(d);
        let (n, b) = by_date.get(&d).copied().unwrap_or((0, 0));
        let mut st = if sel > 0 { fg(&t) } else { dim(&t) };
        if focus_dates && i == rv.date_cursor {
            st = st.bg(t.sel_bg).add_modifier(Modifier::BOLD);
        }
        let _ = (d.year(), n);
        dl.push(Line::from(vec![
            Span::styled(format!("{} ", check(sel, tot)), if sel == tot { Style::default().fg(t.ok) } else { dim(&t) }),
            Span::styled(format!("{:<16}", d.format("%a %d %b %Y")), st),
            Span::styled(format!("{tot:>4} files  {:>9}", human_bytes(b)), st),
        ]));
        hits.push((Rect { x: di.x, y: di.y + row as u16, width: di.width, height: 1 }, Hit::Date(i)));
    }
    f.render_widget(Paragraph::new(dl), di);

    app.list_height = h.max(1);
    app.hits = hits;
}

// ---------- running ----------

fn state_cells<'a>(t: &Theme, st: &ItemState, size: u64, frame: usize, w: usize) -> (Span<'static>, Vec<Span<'a>>) {
    match st {
        ItemState::Queued => (Span::styled("·", dim(t)), vec![Span::styled("queued", dim(t))]),
        ItemState::Copying(b) => {
            let r = if size > 0 { *b as f64 / size as f64 } else { 1.0 };
            let mut v = gradient_bar(w, r, t);
            v.push(Span::styled(format!(" copying {:>3.0}%", r * 100.0), Style::default().fg(t.accent)));
            (Span::styled(spinner(frame), Style::default().fg(t.accent)), v)
        }
        ItemState::Verifying(b) => {
            let r = if size > 0 { *b as f64 / size as f64 } else { 1.0 };
            let mut v = gradient_bar(w, r, t);
            v.push(Span::styled(format!(" verifying {:>3.0}%", r * 100.0), Style::default().fg(t.accent2)));
            (Span::styled(spinner(frame), Style::default().fg(t.accent2)), v)
        }
        ItemState::Copied { renamed, removal } => {
            let (text, c) = match (removal, renamed) {
                (Removal::Removed, _) => ("verified · moved (original deleted)".to_string(), t.ok),
                (Removal::Failed(e), _) => (format!("verified, but original NOT deleted: {}", trunc(&sanitize_display(e), 50)), t.warn),
                (_, true) => ("verified · kept both (renamed)".to_string(), t.warn),
                _ => ("verified".to_string(), t.ok),
            };
            (
                Span::styled("✓", Style::default().fg(if matches!(removal, Removal::Failed(_)) { t.warn } else { t.ok })),
                vec![Span::styled(text, Style::default().fg(c))],
            )
        }
        ItemState::Skipped { removal } => {
            let (text, c) = match removal {
                Removal::Removed => ("already on destination · original deleted".to_string(), t.ok),
                Removal::Failed(e) => (format!("already there, but original NOT deleted: {}", trunc(&sanitize_display(e), 40)), t.warn),
                Removal::NotRequested => ("already on destination (checksum match)".to_string(), t.dim),
            };
            (Span::styled("↷", dim(t)), vec![Span::styled(text, Style::default().fg(c))])
        }
        ItemState::Failed(e) => {
            (Span::styled("✗", Style::default().fg(t.err)), vec![Span::styled(trunc(&sanitize_display(e), 60), Style::default().fg(t.err))])
        }
        ItemState::Cancelled => (Span::styled("−", Style::default().fg(t.warn)), vec![Span::styled("cancelled", Style::default().fg(t.warn))]),
    }
}

fn draw_running(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let frame = app.tick_count / 2;
    let Some(run) = &app.run else { return };
    let batch = run.profiles.len() > 1;
    let pname = if batch {
        format!("Batch · {}", run.profiles.iter().filter_map(|i| app.cfg.profiles.get(*i)).map(short_name).collect::<Vec<_>>().join(" + "))
    } else {
        run.profiles.first().and_then(|i| app.cfg.profiles.get(*i)).map(|p| sanitize_display(&p.name)).unwrap_or_default()
    };
    let paused = run.ctrl.is_paused();
    let copy_done = run.copied >= run.total;
    let (phase, phase_c) = if run.ctrl.is_cancelled() {
        ("CANCELLING", t.warn)
    } else if paused {
        ("PAUSED", t.warn)
    } else if copy_done {
        ("VERIFYING", t.accent2)
    } else {
        (if run.mode == Mode::Move { "MOVING" } else { "COPYING" }, t.accent)
    };
    let top_h = 6 + if batch { run.profiles.len().min(4) as u16 } else { 0 };
    let [top, mid, list_a] = Layout::vertical([Constraint::Length(top_h), Constraint::Length(5), Constraint::Min(4)]).areas(area);

    // progress block
    let blk = block(
        &t,
        Line::from(vec![
            Span::raw(" "),
            Span::styled(if paused || run.ctrl.is_cancelled() { "■ ".to_string() } else { format!("{} ", spinner(frame)) }, Style::default().fg(phase_c)),
            Span::styled(phase, Style::default().fg(phase_c).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {pname} "), bold(&t)),
        ]),
    );
    let inner = blk.inner(top);
    f.render_widget(blk, top);
    let bar_w = (inner.width as usize).saturating_sub(34).clamp(10, 70);
    let pct = |a: u64| if run.total > 0 { a as f64 / run.total as f64 } else { 1.0 };
    let mk = |label: &str, done: u64| {
        let r = pct(done);
        let mut v = vec![Span::styled(format!("{label:<7}"), dim(&t))];
        v.extend(gradient_bar(bar_w, r, &t));
        v.push(Span::styled(format!(" {:>3.0}%  ", r * 100.0), bold(&t)));
        v.push(Span::styled(format!("{} / {}", human_bytes(done.min(run.total)), human_bytes(run.total)), fg(&t)));
        Line::from(v)
    };
    let eta = run.eta().map(|d| human_duration(d.as_secs())).unwrap_or_else(|| "-".into());
    let mut lines = vec![
        mk("copy", run.copied),
        mk("verify", run.verified),
        Line::from(vec![
            Span::styled("files  ", dim(&t)),
            Span::styled(format!("{}/{}", run.done_count(), run.state.len()), bold(&t)),
            Span::styled("   speed ", dim(&t)),
            Span::styled(human_rate(if copy_done { run.verify_rate } else { run.copy_rate }), bold(&t)),
            Span::styled("   eta ", dim(&t)),
            Span::styled(eta, bold(&t)),
            Span::styled("   elapsed ", dim(&t)),
            Span::styled(human_duration(run.elapsed().as_secs()), fg(&t)),
        ]),
    ];
    if batch {
        // One line per camera: blended copy+verify progress, so you can see which card is being worked on.
        for (slot, pi) in run.profiles.iter().enumerate().take(4) {
            let Some(p) = app.cfg.profiles.get(*pi) else { continue };
            let (mut total, mut done, mut files, mut files_done) = (0u64, 0f64, 0usize, 0usize);
            for (item, st) in run.plan.items.iter().zip(&run.state) {
                if item.group != *pi {
                    continue;
                }
                files += 1;
                total += item.size;
                done += match st {
                    ItemState::Copied { .. } | ItemState::Skipped { .. } | ItemState::Failed(_) => item.size as f64,
                    ItemState::Copying(b) => *b as f64 / 2.0,
                    ItemState::Verifying(b) => item.size as f64 / 2.0 + *b as f64 / 2.0,
                    _ => 0.0,
                };
                if st.is_final() {
                    files_done += 1;
                }
            }
            let ratio = if total > 0 { done / total as f64 } else { 1.0 };
            let mut v =
                vec![Span::styled(format!("{:<7}", trunc(&short_name(p), 6)), Style::default().fg(profile_color(&t, slot)).add_modifier(Modifier::BOLD))];
            v.extend(gradient_bar(bar_w.min(40), ratio, &t));
            v.push(Span::styled(format!(" {:>3.0}%  {files_done}/{files} files · {}", ratio * 100.0, human_bytes(total)), fg(&t)));
            lines.push(Line::from(v));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);

    // throughput + storage
    let [spark_a, store_a] = Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)]).areas(mid);
    let blk = block(&t, title(&t, "Throughput"));
    let si = blk.inner(spark_a);
    f.render_widget(blk, spark_a);
    let data: Vec<u64> = run.spark.iter().copied().collect();
    let skip = data.len().saturating_sub(si.width as usize);
    f.render_widget(Sparkline::default().data(&data[skip..]).style(Style::default().fg(t.accent)), si);
    let blk = block(&t, title(&t, "Backup drive"));
    let bi = blk.inner(store_a);
    f.render_widget(blk, store_a);
    let live = crate::app::dest_live(&run.plan.dest_dir);
    let mut sl = vec![];
    match (live.mounted, live.space) {
        (true, Some((total, free))) => {
            let ratio = 1.0 - free as f64 / total.max(1) as f64;
            sl.push(Line::from(gradient_bar(bi.width as usize, ratio, &t)));
            sl.push(Line::from(Span::styled(format!("{} free", human_bytes(free)), bold(&t))));
            sl.push(Line::from(Span::styled(format!("of {}", human_bytes(total)), dim(&t))));
        }
        _ => sl.push(Line::from(Span::styled("not mounted!", Style::default().fg(t.err)))),
    }
    f.render_widget(Paragraph::new(sl), bi);

    // file list
    let blk = block(
        &t,
        Line::from(vec![
            Span::raw(" "),
            Span::styled("Files", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  → {} ", sanitize_display(&trunc_path(&run.plan.dest_dir.display().to_string(), 60))), dim(&t)),
        ]),
    );
    let li = blk.inner(list_a);
    f.render_widget(blk, list_a);
    let h = li.height as usize;
    let namew = (li.width as usize / 3).clamp(14, 30);
    let barw = 12usize;
    let mut lines = vec![];
    for i in run.scroll..run.state.len().min(run.scroll + h) {
        let item = &run.plan.items[i];
        let (icon, cells) = state_cells(&t, &run.state[i], item.size, frame, barw);
        let active = matches!(run.state[i], ItemState::Copying(_) | ItemState::Verifying(_));
        let mut spans = vec![icon, Span::raw(" ")];
        if batch {
            let slot = run.profiles.iter().position(|p| *p == item.group).unwrap_or(0);
            let tag = app.cfg.profiles.get(item.group).map(short_name).unwrap_or_default();
            spans.push(Span::styled(format!("{:<5} ", trunc(&tag, 5)), Style::default().fg(profile_color(&t, slot))));
        }
        spans.extend([
            Span::styled(format!("{:<w$}", trunc(&sanitize_display(&item.name), namew), w = namew), if active { bold(&t) } else { fg(&t) }),
            Span::styled(format!(" {:>9}  ", human_bytes(item.size)), dim(&t)),
        ]);
        spans.extend(cells);
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), li);
    app.list_height = h.max(1);
}

// ---------- summary ----------

fn draw_summary(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let Some((pids, s)) = &app.summary else { return };
    let batch = pids.len() > 1;
    let pname = if batch {
        format!("Batch · {}", pids.iter().filter_map(|i| app.cfg.profiles.get(*i)).map(short_name).collect::<Vec<_>>().join(" + "))
    } else {
        pids.first().and_then(|i| app.cfg.profiles.get(*i)).map(|p| sanitize_display(&p.name)).unwrap_or_default()
    };
    let (icon, head, c) = if s.cancelled {
        ("■", "Cancelled", t.warn)
    } else if s.failed > 0 {
        ("⚠", "Finished with errors", t.err)
    } else {
        ("✓", "All files copied and verified", t.ok)
    };
    let w = area.width.min(90);
    let h = (16 + s.failures.len().min(8) as u16 + s.move_failures.len().min(6) as u16 + if batch { pids.len() as u16 + 1 } else { 0 }).min(area.height);
    let a = Rect { x: area.x + (area.width - w) / 2, y: area.y + area.height.saturating_sub(h) / 3, width: w, height: h };
    let blk = Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(c)).title(title(&t, &pname));
    let inner = blk.inner(a);
    f.render_widget(blk, a);
    let mut lines = vec![
        Line::from(Span::styled(format!("{icon}  {head}"), Style::default().fg(c).add_modifier(Modifier::BOLD))).alignment(Alignment::Center),
        Line::from(""),
        Line::from(vec![Span::styled("copied     ", dim(&t)), Span::styled(format!("{} files · {}", s.copied, human_bytes(s.bytes_copied)), bold(&t))]),
        Line::from(vec![Span::styled("skipped    ", dim(&t)), Span::styled(format!("{} already on destination", s.skipped), fg(&t))]),
        Line::from(vec![Span::styled("renamed    ", dim(&t)), Span::styled(format!("{} (name clashed with a different file; both kept)", s.renamed), fg(&t))]),
        Line::from(vec![
            Span::styled("failed     ", dim(&t)),
            Span::styled(s.failed.to_string(), if s.failed > 0 { Style::default().fg(t.err).add_modifier(Modifier::BOLD) } else { fg(&t) }),
        ]),
        Line::from(vec![
            Span::styled("time       ", dim(&t)),
            Span::styled(human_duration(s.elapsed.as_secs()), fg(&t)),
            Span::styled(
                if s.elapsed.as_secs_f64() > 0.5 { format!("  ({})", human_rate(s.bytes_copied as f64 / s.elapsed.as_secs_f64())) } else { String::new() },
                dim(&t),
            ),
        ]),
        Line::from(vec![
            Span::styled("saved to   ", dim(&t)),
            Span::styled(sanitize_display(&trunc_path(&s.dest_dir.display().to_string(), inner.width.saturating_sub(12) as usize)), fg(&t)),
        ]),
        Line::from(""),
    ];
    if batch {
        for (slot, pp) in app.per_profile.iter().enumerate() {
            let Some(p) = app.cfg.profiles.get(pp.profile) else { continue };
            let (mark, mc) = if pp.failed > 0 {
                ("✗", t.err)
            } else if pp.copied + pp.skipped < pp.total {
                ("■", t.warn)
            } else {
                ("✓", t.ok)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(mc)),
                Span::styled(format!("{:<8}", trunc(&short_name(p), 8)), Style::default().fg(profile_color(&t, slot)).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} copied · {}", pp.copied, human_bytes(pp.bytes)), bold(&t)),
                Span::styled(if pp.skipped > 0 { format!("  {} skipped", pp.skipped) } else { String::new() }, dim(&t)),
                Span::styled(if pp.failed > 0 { format!("  {} FAILED", pp.failed) } else { String::new() }, Style::default().fg(t.err)),
            ]));
        }
        lines.push(Line::from(""));
    }
    if s.failed == 0 && !s.cancelled {
        lines.push(Line::from(Span::styled("Every copied file was re-read from disk and matched its source checksum.", dim(&t))));
    }
    match (app.run_mode, &app.deleted) {
        (Mode::Move, _) => {
            lines.push(Line::from(vec![Span::styled("originals  ", dim(&t)), Span::styled(format!("{} deleted from the card (moved)", s.moved), bold(&t))]));
            for (n, e) in s.move_failures.iter().take(6) {
                lines.push(Line::from(vec![
                    Span::styled("⚠ ", Style::default().fg(t.warn)),
                    Span::styled(format!("{}: ", trunc(&sanitize_display(n), 28)), bold(&t)),
                    Span::styled(format!("copy is safe, original kept: {}", trunc(&sanitize_display(e), 40)), Style::default().fg(t.warn)),
                ]));
            }
        }
        (Mode::Copy, Some(rep)) => {
            lines.push(Line::from(vec![
                Span::styled("originals  ", dim(&t)),
                Span::styled(format!("{} deleted ({}) from the card", rep.deleted, human_bytes(rep.bytes)), bold(&t)),
            ]));
            for (n, e) in rep.failures.iter().take(6) {
                lines.push(Line::from(vec![
                    Span::styled("⚠ ", Style::default().fg(t.warn)),
                    Span::styled(format!("{}: ", trunc(&sanitize_display(n), 28)), bold(&t)),
                    Span::styled(format!("kept: {}", trunc(&sanitize_display(e), 44)), Style::default().fg(t.warn)),
                ]));
            }
        }
        (Mode::Copy, None) if !app.originals.is_empty() => {
            let bytes: u64 = app.originals.iter().map(|o| o.size).sum();
            lines.push(Line::from(vec![
                Span::styled("originals  ", dim(&t)),
                Span::styled(
                    format!("still on the card · press d to delete the {} verified ones ({})", app.originals.len(), human_bytes(bytes)),
                    Style::default().fg(t.accent),
                ),
            ]));
            if app.ineligible > 0 {
                lines.push(Line::from(Span::styled(
                    format!("           {} file(s) were not verified and will never be offered for deletion", app.ineligible),
                    dim(&t),
                )));
            }
        }
        _ => lines.push(Line::from(Span::styled("Nothing was deleted from the card.", dim(&t)))),
    }
    for (n, e) in s.failures.iter().take(8) {
        lines.push(Line::from(vec![
            Span::styled("✗ ", Style::default().fg(t.err)),
            Span::styled(format!("{}: ", trunc(&sanitize_display(n), 28)), bold(&t)),
            Span::styled(trunc(&sanitize_display(e), 50), Style::default().fg(t.err)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// ---------- config ----------

fn draw_config(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let rows = app.editor_rows();
    let [note, list_a] = Layout::vertical([Constraint::Length(2), Constraint::Min(4)]).areas(area);
    let msg = if app.config_exists {
        format!("editing {}", app.paths.config.display())
    } else {
        format!("using built-in profiles · your first edit creates {}", app.paths.config.display())
    };
    f.render_widget(Paragraph::new(Line::from(Span::styled(sanitize_display(&msg), dim(&t)))), note);
    let blk = block(&t, title(&t, "Settings · changes save instantly"));
    let inner = blk.inner(list_a);
    f.render_widget(blk, list_a);
    let h = inner.height as usize;
    if app.editor.cursor < app.editor.scroll {
        app.editor.scroll = app.editor.cursor;
    } else if app.editor.cursor >= app.editor.scroll + h {
        app.editor.scroll = app.editor.cursor + 1 - h;
    }
    let mut lines = vec![];
    let mut hits = vec![];
    for (row, i) in (app.editor.scroll..rows.len()).take(h).enumerate() {
        let r = rows[i];
        let (label, value) = app.row_text(r);
        let cur = i == app.editor.cursor;
        let bg = |s: Style| if cur { s.bg(t.sel_bg) } else { s };
        let editing = cur && app.editor.input.is_some();
        let line = match r {
            Row::Header(_) => Line::from(vec![
                Span::styled(if cur { "▸ " } else { "  " }, Style::default().fg(t.accent)),
                Span::styled(sanitize_display(&label), bg(Style::default().fg(t.accent2).add_modifier(Modifier::BOLD))),
            ]),
            Row::AddProfile => {
                Line::from(vec![Span::styled(if cur { "▸ " } else { "  " }, Style::default().fg(t.accent)), Span::styled(label, bg(Style::default().fg(t.ok)))])
            }
            _ => {
                let indent = if matches!(r, Row::Field(..)) { "  " } else { "" };
                let shown = if editing { format!("{}▏", app.editor.input.clone().unwrap_or_default()) } else { sanitize_display(&value) };
                let vstyle = if editing { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { fg(&t) };
                Line::from(vec![
                    Span::styled(if cur { "▸ " } else { "  " }, Style::default().fg(t.accent)),
                    Span::styled(indent.to_string(), dim(&t)),
                    Span::styled(format!("{label:<20}"), bg(dim(&t))),
                    Span::styled(trunc(&shown, (inner.width as usize).saturating_sub(28)), bg(vstyle)),
                ])
            }
        };
        lines.push(line);
        hits.push((Rect { x: inner.x, y: inner.y + row as u16, width: inner.width, height: 1 }, Hit::Row(i)));
    }
    f.render_widget(Paragraph::new(lines), inner);
    app.list_height = h.max(1);
    app.hits = hits;
}

// ---------- stats ----------

fn draw_stats(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let Some(sv) = &app.stats else { return };
    let s = &sv.stats;
    let [tiles, charts, recent_a] = Layout::vertical([Constraint::Length(3), Constraint::Length(10), Constraint::Min(4)]).areas(area);
    let tile_a = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(tiles);
    for (i, (label, val)) in [
        ("sessions", s.sessions.to_string()),
        ("files copied", s.files.to_string()),
        ("data offloaded", human_bytes(s.bytes)),
        ("time copying", human_duration(s.secs)),
    ]
    .iter()
    .enumerate()
    {
        let blk = block(&t, title(&t, label));
        let inner = blk.inner(tile_a[i]);
        f.render_widget(blk, tile_a[i]);
        f.render_widget(Paragraph::new(Span::styled(val.clone(), Style::default().fg(t.accent2).add_modifier(Modifier::BOLD))), inner);
    }
    let [months_a, prof_a] = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).areas(charts);
    let blk = block(&t, title(&t, "Data by month"));
    let mi = blk.inner(months_a);
    f.render_widget(blk, months_a);
    let months: Vec<_> = s.by_month.iter().rev().take(mi.height as usize).collect::<Vec<_>>().into_iter().rev().collect();
    let maxb = months.iter().map(|(_, b)| **b).max().unwrap_or(1).max(1);
    let ml: Vec<Line> = months
        .iter()
        .map(|((y, m), b)| {
            let bar_w = (mi.width as usize).saturating_sub(20).clamp(4, 50);
            Line::from(vec![
                Span::styled(format!("{y}-{m:02}  "), dim(&t)),
                solid_bar(bar_w, **b as f64 / maxb as f64, t.accent),
                Span::styled(format!(" {}", human_bytes(**b)), fg(&t)),
            ])
        })
        .collect();
    if ml.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("No sessions yet. Copy something!", dim(&t))), mi);
    } else {
        f.render_widget(Paragraph::new(ml), mi);
    }
    let blk = block(&t, title(&t, "By camera"));
    let pi = blk.inner(prof_a);
    f.render_widget(blk, prof_a);
    let maxp = s.by_profile.values().copied().max().unwrap_or(1).max(1);
    let pl: Vec<Line> = s
        .by_profile
        .iter()
        .map(|(n, b)| {
            let bar_w = (pi.width as usize).saturating_sub(30).clamp(3, 20);
            Line::from(vec![
                Span::styled(format!("{:<16}", trunc(&sanitize_display(n), 15)), fg(&t)),
                solid_bar(bar_w, *b as f64 / maxp as f64, t.accent2),
                Span::styled(format!(" {}", human_bytes(*b)), dim(&t)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(pl), pi);

    let blk = block(&t, title(&t, "Recent sessions"));
    let ri = blk.inner(recent_a);
    f.render_widget(blk, recent_a);
    let rl: Vec<Line> = sv
        .records
        .iter()
        .skip(sv.scroll)
        .take(ri.height as usize)
        .map(|r| {
            let (mark, c) = if r.cancelled {
                ("■", t.warn)
            } else if r.failed > 0 {
                ("✗", t.err)
            } else {
                ("✓", t.ok)
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(c)),
                Span::styled(format!("{}  ", r.when.format("%Y-%m-%d %H:%M")), dim(&t)),
                Span::styled(format!("{:<18}", trunc(&sanitize_display(&r.profile), 17)), fg(&t)),
                Span::styled(format!("{:>5} files {:>10}  {}", r.copied, human_bytes(r.bytes), human_duration(r.secs)), fg(&t)),
                Span::styled(if r.skipped > 0 { format!("  {} skipped", r.skipped) } else { String::new() }, dim(&t)),
                Span::styled(if r.failed > 0 { format!("  {} FAILED", r.failed) } else { String::new() }, Style::default().fg(t.err)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(rl), ri);
}

// ---------- help ----------

fn draw_help(f: &mut Frame, app: &App, full: Rect) {
    let t = app.theme;
    let w = 64.min(full.width);
    let h = 27.min(full.height);
    let a = Rect { x: full.x + (full.width - w) / 2, y: full.y + (full.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, a);
    let blk = Block::bordered().border_type(BorderType::Double).border_style(Style::default().fg(t.accent)).title(title(&t, "Keys"));
    let inner = blk.inner(a);
    f.render_widget(blk, a);
    let sec = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(t.accent2).add_modifier(Modifier::BOLD)));
    let kv = |k: &str, d: &str| {
        Line::from(vec![Span::styled(format!("  {k:<14}"), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)), Span::styled(d.to_string(), fg(&t))])
    };
    let lines = vec![
        sec("Home"),
        kv("↑ ↓ / j k", "choose a camera profile (or press 1-9)"),
        kv("enter", "scan the card"),
        kv("b  (or 0)", "batch: every connected camera at once"),
        kv("e  s  t", "settings · stats · cycle theme"),
        sec("Review"),
        kv("space", "include / exclude a file or a whole day"),
        kv("tab", "switch between files and shoot dates"),
        kv("a", "select all / none"),
        kv("m", "mode: copy (keep) or move (delete originals)"),
        kv("enter", "start copying (checks free space first)"),
        sec("While copying"),
        kv("p", "pause / resume"),
        kv("esc", "cancel (partial files are removed)"),
        sec("After"),
        kv("o  x  d", "open folder · eject · delete verified originals"),
        sec("Everywhere"),
        kv("mouse", "click rows, scroll wheel"),
        kv("?", "this help"),
        kv("ctrl+c", "quit"),
        Line::from(""),
        Line::from(Span::styled("Files are copied to a .part file, re-read from disk,", dim(&t))),
        Line::from(Span::styled("checksum-verified, and only then renamed. Nothing is", dim(&t))),
        Line::from(Span::styled("ever overwritten or deleted from your card.", dim(&t))),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
