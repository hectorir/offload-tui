//! Application state and input handling (rendering lives in `ui`).

use crate::config::{self, Config, Profile};
use crate::engine::{self, Control, Event, Outcome, Plan, Summary};
use crate::fsutil;
use crate::history::{self, Record};
use crate::scan::{self, ScanResult};
use crate::sys;
use crate::theme::{self, Theme};
use chrono::{Local, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    Scanning,
    Review,
    Running,
    Summary,
    Config,
    Stats,
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub history: PathBuf,
    pub state: PathBuf,
}

impl Paths {
    pub fn standard() -> Paths {
        Paths { config: config::default_config_path(), history: history::default_path(), state: config::data_dir().join("state.json") }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Err,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub level: Level,
    pub at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct ProfileLive {
    pub present: Vec<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct DestLive {
    pub mounted: bool,
    pub space: Option<(u64, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Files,
    Dates,
}

pub struct Review {
    pub profile: usize,
    pub scan: ScanResult,
    pub selected: Vec<bool>,
    /// Same-size file already at the destination (confirmed by checksum during the copy).
    pub present: Vec<bool>,
    pub cursor: usize,
    pub scroll: usize,
    pub dates: Vec<NaiveDate>,
    pub date_cursor: usize,
    pub focus: Focus,
    pub dest_dir: PathBuf,
    pub dest: DestLive,
    pub error: Option<String>,
}

impl Review {
    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|s| **s).count()
    }
    pub fn selected_bytes(&self) -> u64 {
        self.scan.files.iter().zip(&self.selected).filter(|(_, s)| **s).map(|(f, _)| f.size).sum()
    }
    pub fn already_count(&self) -> usize {
        self.present.iter().zip(&self.selected).filter(|(p, s)| **p && **s).count()
    }
    pub fn need_bytes(&self) -> u64 {
        self.scan.files.iter().zip(self.selected.iter().zip(&self.present)).filter(|(_, (s, p))| **s && !**p).map(|(f, _)| f.size).sum()
    }
    pub fn enough_space(&self) -> bool {
        match self.dest.space {
            Some((_, free)) => free >= self.need_bytes().saturating_add(1 << 20),
            None => true,
        }
    }
    pub fn date_state(&self, d: NaiveDate) -> (usize, usize) {
        let mut sel = 0;
        let mut tot = 0;
        for (f, s) in self.scan.files.iter().zip(&self.selected) {
            if f.date == d {
                tot += 1;
                if *s {
                    sel += 1;
                }
            }
        }
        (sel, tot)
    }
    pub fn toggle_date(&mut self, d: NaiveDate) {
        let (sel, tot) = self.date_state(d);
        let target = sel != tot;
        for (f, s) in self.scan.files.iter().zip(self.selected.iter_mut()) {
            if f.date == d {
                *s = target;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemState {
    Queued,
    Copying(u64),
    Verifying(u64),
    Copied { renamed: bool },
    Skipped,
    Failed(String),
    Cancelled,
}

impl ItemState {
    pub fn is_final(&self) -> bool {
        matches!(self, ItemState::Copied { .. } | ItemState::Skipped | ItemState::Failed(_) | ItemState::Cancelled)
    }
}

pub struct Run {
    pub profile: usize,
    pub plan: Plan,
    pub state: Vec<ItemState>,
    pub copied: u64,
    pub verified: u64,
    pub total: u64,
    pub started: Instant,
    pub last_sample: Instant,
    pub last_copied: u64,
    pub last_verified: u64,
    pub spark: VecDeque<u64>,
    pub copy_rate: f64,
    pub verify_rate: f64,
    pub ctrl: Arc<Control>,
    pub rx: Receiver<Event>,
    pub confirm_cancel: bool,
    pub scroll: usize,
    pub follow: bool,
    pub paused_at: Option<Instant>,
    pub paused_total: Duration,
}

impl Run {
    pub fn done_count(&self) -> usize {
        self.state.iter().filter(|s| s.is_final()).count()
    }
    pub fn elapsed(&self) -> Duration {
        let paused = self.paused_total + self.paused_at.map(|p| p.elapsed()).unwrap_or_default();
        self.started.elapsed().saturating_sub(paused)
    }
    pub fn eta(&self) -> Option<Duration> {
        let (remaining, rate) =
            if self.copied < self.total { (self.total - self.copied, self.copy_rate) } else { (self.total.saturating_sub(self.verified), self.verify_rate) };
        if rate < 1.0 || remaining == 0 {
            return None;
        }
        Some(Duration::from_secs_f64(remaining as f64 / rate))
    }
    pub fn active_index(&self) -> usize {
        self.state
            .iter()
            .position(|s| matches!(s, ItemState::Copying(_) | ItemState::Verifying(_)))
            .or_else(|| self.state.iter().position(|s| !s.is_final()))
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GField {
    Destination,
    DateFormat,
    Notify,
    Theme,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PField {
    Name,
    Subfolder,
    Sources,
    Include,
    Exclude,
    Icon,
    Destination,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Global(GField),
    Header(usize),
    Field(usize, PField),
    AddProfile,
}

#[derive(Default)]
pub struct Editor {
    pub cursor: usize,
    pub scroll: usize,
    pub input: Option<String>,
    pub confirm_delete: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Profile(usize),
    File(usize),
    Date(usize),
    Row(usize),
}

pub struct StatsView {
    pub records: Vec<Record>,
    pub stats: history::Stats,
    pub scroll: usize,
}

pub struct App {
    pub cfg: Config,
    pub paths: Paths,
    pub config_exists: bool,
    pub theme: Theme,
    pub screen: Screen,
    pub should_quit: bool,
    pub show_help: bool,
    pub home_sel: usize,
    pub home_scroll: usize,
    pub live: Vec<ProfileLive>,
    pub dest_live: DestLive,
    pub last_live: Instant,
    pub scan_rx: Option<Receiver<(usize, ScanResult)>>,
    pub scan_profile: usize,
    pub review: Option<Review>,
    pub run: Option<Run>,
    pub summary: Option<(usize, Summary)>,
    pub eject_confirm: Option<Vec<PathBuf>>,
    pub editor: Editor,
    pub stats: Option<StatsView>,
    pub toast: Option<Toast>,
    pub tick_count: usize,
    pub hits: Vec<(Rect, Hit)>,
    /// Desktop notifications, eject and open actions. Disabled in tests.
    pub sys_actions: bool,
    pub list_height: usize,
}

const SAMPLE_EVERY: Duration = Duration::from_millis(250);
const SPARK_LEN: usize = 90;

impl App {
    pub fn new(cfg: Config, paths: Paths) -> App {
        let config_exists = paths.config.exists();
        let theme = load_theme_pref(&paths.state).unwrap_or_else(|| cfg.theme.clone());
        let mut app = App {
            theme: Theme::by_name(&theme),
            cfg,
            config_exists,
            paths,
            screen: Screen::Home,
            should_quit: false,
            show_help: false,
            home_sel: 0,
            home_scroll: 0,
            live: vec![],
            dest_live: DestLive::default(),
            last_live: Instant::now() - Duration::from_secs(10),
            scan_rx: None,
            scan_profile: 0,
            review: None,
            run: None,
            summary: None,
            eject_confirm: None,
            editor: Editor::default(),
            stats: None,
            toast: None,
            tick_count: 0,
            hits: vec![],
            sys_actions: true,
            list_height: 10,
        };
        app.refresh_live();
        app
    }

    // ---------- helpers ----------

    pub fn toast(&mut self, text: impl Into<String>, level: Level) {
        self.toast = Some(Toast { text: text.into(), level, at: Instant::now() });
    }

    pub fn refresh_live(&mut self) {
        self.live = self.cfg.profiles.iter().map(|p| ProfileLive { present: p.source_paths().iter().map(|s| s.is_dir()).collect() }).collect();
        self.dest_live = dest_live(&self.cfg.destination_path());
        self.last_live = Instant::now();
        if self.home_sel >= self.cfg.profiles.len() {
            self.home_sel = self.cfg.profiles.len().saturating_sub(1);
        }
    }

    pub fn set_theme(&mut self, name: &str) {
        self.theme = Theme::by_name(name);
        save_theme_pref(&self.paths.state, self.theme.name);
    }

    fn profile(&self, i: usize) -> Option<&Profile> {
        self.cfg.profiles.get(i)
    }

    // ---------- tick ----------

    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        if let Some(t) = &self.toast
            && t.at.elapsed() > Duration::from_secs(5)
        {
            self.toast = None;
        }
        match self.screen {
            Screen::Home if self.last_live.elapsed() > Duration::from_secs(1) => self.refresh_live(),
            Screen::Scanning => self.poll_scan(),
            Screen::Review => {
                // Keep the destination status fresh (drive plugged in while reviewing).
                if self.last_live.elapsed() > Duration::from_secs(2) {
                    self.refresh_review_dest();
                    self.last_live = Instant::now();
                }
            }
            Screen::Running => self.poll_run(),
            _ => {}
        }
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };
        match rx.try_recv() {
            Ok((idx, scan)) => {
                self.scan_rx = None;
                self.on_scan_done(idx, scan);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.scan_rx = None;
                self.screen = Screen::Home;
                self.toast("Scan failed unexpectedly", Level::Err);
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn on_scan_done(&mut self, idx: usize, scan: ScanResult) {
        let Some(profile) = self.profile(idx).cloned() else { return };
        let dest_dir = engine::build_plan(&self.cfg, &profile, &[], Local::now()).map(|p| p.dest_dir).unwrap_or_default();
        let present = scan
            .files
            .iter()
            .map(|f| std::fs::symlink_metadata(dest_dir.join(f.path.file_name().unwrap_or_default())).map(|m| m.len() == f.size).unwrap_or(false))
            .collect();
        let dates: Vec<NaiveDate> = scan.by_date().keys().copied().collect();
        let n = scan.files.len();
        self.review = Some(Review {
            profile: idx,
            selected: vec![true; n],
            present,
            cursor: 0,
            scroll: 0,
            dates,
            date_cursor: 0,
            focus: Focus::Files,
            dest: dest_live(&dest_dir),
            dest_dir,
            error: None,
            scan,
        });
        self.screen = Screen::Review;
    }

    fn refresh_review_dest(&mut self) {
        if let Some(rv) = &mut self.review {
            rv.dest = dest_live(&rv.dest_dir);
        }
    }

    fn poll_run(&mut self) {
        let mut finished = None;
        if let Some(run) = &mut self.run {
            while let Ok(ev) = run.rx.try_recv() {
                match ev {
                    Event::Started { index } => {
                        if let Some(s) = run.state.get_mut(index)
                            && *s == ItemState::Queued
                        {
                            *s = ItemState::Copying(0);
                        }
                    }
                    Event::Progress { index, bytes } => {
                        run.copied += bytes;
                        if let Some(ItemState::Copying(b)) = run.state.get_mut(index) {
                            *b += bytes;
                        }
                    }
                    Event::Verifying { index } => {
                        if let Some(s) = run.state.get_mut(index)
                            && !s.is_final()
                        {
                            *s = ItemState::Verifying(0);
                        }
                    }
                    Event::VerifyProgress { index, bytes } => {
                        run.verified += bytes;
                        if let Some(ItemState::Verifying(b)) = run.state.get_mut(index) {
                            *b += bytes;
                        }
                    }
                    Event::Done { index, outcome } => {
                        if let Some(s) = run.state.get_mut(index) {
                            *s = match outcome {
                                Outcome::Copied { renamed, .. } => ItemState::Copied { renamed },
                                Outcome::Skipped { .. } => ItemState::Skipped,
                                Outcome::Failed(e) => ItemState::Failed(e),
                                Outcome::Cancelled => ItemState::Cancelled,
                            };
                        }
                    }
                    Event::Finished(s) => finished = Some(s),
                }
            }
            let dt = run.last_sample.elapsed();
            if dt >= SAMPLE_EVERY && run.paused_at.is_none() {
                let secs = dt.as_secs_f64();
                let cr = (run.copied - run.last_copied) as f64 / secs;
                let vr = (run.verified - run.last_verified) as f64 / secs;
                // Light smoothing so the numbers don't jitter.
                run.copy_rate = if run.copy_rate == 0.0 { cr } else { run.copy_rate * 0.6 + cr * 0.4 };
                run.verify_rate = if run.verify_rate == 0.0 { vr } else { run.verify_rate * 0.6 + vr * 0.4 };
                run.spark.push_back(cr.max(vr) as u64);
                while run.spark.len() > SPARK_LEN {
                    run.spark.pop_front();
                }
                run.last_sample = Instant::now();
                run.last_copied = run.copied;
                run.last_verified = run.verified;
            }
            if run.follow {
                let h = self.list_height.max(1);
                let a = run.active_index();
                run.scroll = a.saturating_sub(h / 2);
            }
        }
        if let Some(s) = finished {
            self.finish_run(s);
        }
    }

    fn finish_run(&mut self, s: Summary) {
        let Some(run) = self.run.take() else { return };
        let name = self.profile(run.profile).map(|p| p.name.clone()).unwrap_or_default();
        let _ = history::append(&self.paths.history, &Record::from_summary(&name, &s, Local::now()));
        if self.cfg.notify && self.sys_actions {
            let body = if s.cancelled {
                format!("{name}: cancelled after {} files", s.copied)
            } else if s.failed > 0 {
                format!("{name}: {} copied, {} FAILED", s.copied, s.failed)
            } else {
                format!("{name}: {} files copied & verified ({})", s.copied + s.skipped, fsutil::human_bytes(s.bytes_copied))
            };
            sys::notify("offload", &body);
        }
        self.summary = Some((run.profile, s));
        self.eject_confirm = None;
        self.screen = Screen::Summary;
    }

    // ---------- starting things ----------

    pub fn begin_scan(&mut self, idx: usize) {
        let Some(profile) = self.profile(idx).cloned() else { return };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let r = scan::scan_profile(&profile);
            let _ = tx.send((idx, r));
        });
        self.scan_profile = idx;
        self.scan_rx = Some(rx);
        self.screen = Screen::Scanning;
    }

    pub fn start_run(&mut self) {
        let Some(rv) = &mut self.review else { return };
        let Some(profile) = self.cfg.profiles.get(rv.profile).cloned() else { return };
        let files: Vec<_> = rv.scan.files.iter().zip(&rv.selected).filter(|(_, s)| **s).map(|(f, _)| f).collect();
        let plan = match engine::build_plan(&self.cfg, &profile, &files, Local::now()) {
            Ok(p) => p,
            Err(e) => {
                rv.error = Some(e.to_string());
                return;
            }
        };
        if let Err(e) = engine::preflight(&plan) {
            rv.error = Some(e);
            return;
        }
        rv.error = None;
        let (tx, rx) = mpsc::channel();
        let ctrl = Control::new();
        let (p2, c2) = (plan.clone(), ctrl.clone());
        std::thread::spawn(move || {
            engine::run(&p2, &c2, tx);
        });
        let total = plan.total_bytes();
        let n = plan.items.len();
        let now = Instant::now();
        self.run = Some(Run {
            profile: rv.profile,
            plan,
            state: vec![ItemState::Queued; n],
            copied: 0,
            verified: 0,
            total,
            started: now,
            last_sample: now,
            last_copied: 0,
            last_verified: 0,
            spark: VecDeque::new(),
            copy_rate: 0.0,
            verify_rate: 0.0,
            ctrl,
            rx,
            confirm_cancel: false,
            scroll: 0,
            follow: true,
            paused_at: None,
            paused_total: Duration::ZERO,
        });
        self.screen = Screen::Running;
    }

    pub fn open_stats(&mut self) {
        let records = history::load(&self.paths.history);
        let stats = history::stats(&records);
        self.stats = Some(StatsView { records, stats, scroll: 0 });
        self.screen = Screen::Stats;
    }

    // ---------- keys ----------

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            if let Some(run) = &self.run {
                if run.confirm_cancel || run.ctrl.is_cancelled() {
                    run.ctrl.cancel();
                    self.should_quit = true; // second Ctrl+C: leave; partial files are cleaned by the engine
                } else {
                    run.ctrl.cancel();
                }
                return;
            }
            self.should_quit = true;
            return;
        }
        if self.editor.input.is_some() && self.screen == Screen::Config {
            self.config_input_key(key);
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        if key.code == KeyCode::Char('?') {
            self.show_help = true;
            return;
        }
        match self.screen {
            Screen::Home => self.home_key(key),
            Screen::Scanning => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.scan_rx = None;
                    self.screen = Screen::Home;
                }
            }
            Screen::Review => self.review_key(key),
            Screen::Running => self.run_key(key),
            Screen::Summary => self.summary_key(key),
            Screen::Config => self.config_key(key),
            Screen::Stats => self.stats_key(key),
        }
    }

    fn home_key(&mut self, key: KeyEvent) {
        let n = self.cfg.profiles.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.home_sel = self.home_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if self.home_sel + 1 < n {
                    self.home_sel += 1
                }
            }
            KeyCode::Enter => self.begin_scan(self.home_sel),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let i = c as usize - '1' as usize;
                if i < n {
                    self.home_sel = i;
                    self.begin_scan(i);
                }
            }
            KeyCode::Char('e') => {
                self.editor = Editor::default();
                self.screen = Screen::Config;
            }
            KeyCode::Char('s') | KeyCode::Char('h') => self.open_stats(),
            KeyCode::Char('t') => {
                let next = self.theme.next();
                self.set_theme(next.name);
                self.toast(format!("Theme: {}", next.name), Level::Info);
            }
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    fn review_key(&mut self, key: KeyEvent) {
        let Some(rv) = &mut self.review else { return };
        let page = self.list_height.max(1);
        let (len, cursor) = match rv.focus {
            Focus::Files => (rv.scan.files.len(), &mut rv.cursor),
            Focus::Dates => (rv.dates.len(), &mut rv.date_cursor),
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => *cursor = cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if *cursor + 1 < len {
                    *cursor += 1
                }
            }
            KeyCode::PageUp => *cursor = cursor.saturating_sub(page),
            KeyCode::PageDown => *cursor = (*cursor + page).min(len.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => *cursor = 0,
            KeyCode::End | KeyCode::Char('G') => *cursor = len.saturating_sub(1),
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('d') => {
                rv.focus = if rv.focus == Focus::Files && !rv.dates.is_empty() { Focus::Dates } else { Focus::Files };
            }
            KeyCode::Char(' ') | KeyCode::Char('x') => match rv.focus {
                Focus::Files => {
                    if let Some(s) = rv.selected.get_mut(rv.cursor) {
                        *s = !*s;
                    }
                }
                Focus::Dates => {
                    if let Some(d) = rv.dates.get(rv.date_cursor).copied() {
                        rv.toggle_date(d);
                    }
                }
            },
            KeyCode::Char('a') => {
                let all = rv.selected.iter().all(|s| *s);
                rv.selected.iter_mut().for_each(|s| *s = !all);
            }
            KeyCode::Enter => {
                if rv.selected_count() == 0 {
                    rv.error = Some("Nothing selected. Press space to select files, or 'a' for all.".into());
                } else {
                    self.start_run();
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.review = None;
                self.screen = Screen::Home;
            }
            _ => {}
        }
        self.review_follow_cursor();
    }

    pub fn review_follow_cursor(&mut self) {
        let h = self.list_height.max(1);
        if let Some(rv) = &mut self.review {
            if rv.cursor < rv.scroll {
                rv.scroll = rv.cursor;
            } else if rv.cursor >= rv.scroll + h {
                rv.scroll = rv.cursor + 1 - h;
            }
        }
    }

    fn run_key(&mut self, key: KeyEvent) {
        let Some(run) = &mut self.run else { return };
        if run.confirm_cancel {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    run.ctrl.set_paused(false);
                    run.ctrl.cancel();
                    run.confirm_cancel = false;
                }
                _ => run.confirm_cancel = false,
            }
            return;
        }
        match key.code {
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                let paused = !run.ctrl.is_paused();
                run.ctrl.set_paused(paused);
                if paused {
                    run.paused_at = Some(Instant::now());
                } else if let Some(p) = run.paused_at.take() {
                    run.paused_total += p.elapsed();
                    run.last_sample = Instant::now();
                }
            }
            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') => run.confirm_cancel = true,
            KeyCode::Up | KeyCode::Char('k') => {
                run.follow = false;
                run.scroll = run.scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                run.follow = false;
                run.scroll = (run.scroll + 1).min(run.state.len().saturating_sub(1));
            }
            KeyCode::Char('f') => run.follow = true,
            _ => {}
        }
    }

    fn summary_key(&mut self, key: KeyEvent) {
        if let Some(vols) = self.eject_confirm.clone() {
            self.eject_confirm = None;
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) {
                let mut errs = vec![];
                let mut ok = 0;
                for v in &vols {
                    if !self.sys_actions {
                        ok += 1;
                        continue;
                    }
                    match sys::eject(v) {
                        Ok(()) => ok += 1,
                        Err(e) => errs.push(format!("{}: {e}", v.display())),
                    }
                }
                if errs.is_empty() {
                    self.toast(format!("Ejected {ok} volume(s). Safe to unplug."), Level::Ok);
                } else {
                    self.toast(format!("Eject failed: {}", errs.join("; ")), Level::Err);
                }
            }
            return;
        }
        match key.code {
            KeyCode::Char('o') => {
                if let Some((_, s)) = &self.summary
                    && self.sys_actions
                {
                    sys::open_folder(&s.dest_dir);
                }
            }
            KeyCode::Char('x') | KeyCode::Char('e') => {
                let vols = self.summary.as_ref().and_then(|(i, _)| self.cfg.profiles.get(*i)).map(|p| sys::volumes_of(&p.source_paths())).unwrap_or_default();
                if vols.is_empty() {
                    self.toast("No ejectable source volumes are mounted.", Level::Warn);
                } else {
                    self.eject_confirm = Some(vols);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('h') => self.open_stats(),
            KeyCode::Enter | KeyCode::Esc => {
                self.summary = None;
                self.review = None;
                self.refresh_live();
                self.screen = Screen::Home;
            }
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }
    }

    fn stats_key(&mut self, key: KeyEvent) {
        let Some(sv) = &mut self.stats else { return };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => sv.scroll = sv.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => sv.scroll = (sv.scroll + 1).min(sv.records.len().saturating_sub(1)),
            KeyCode::PageUp => sv.scroll = sv.scroll.saturating_sub(10),
            KeyCode::PageDown => sv.scroll = (sv.scroll + 10).min(sv.records.len().saturating_sub(1)),
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                self.screen = if self.summary.is_some() { Screen::Summary } else { Screen::Home };
            }
            _ => {}
        }
    }

    // ---------- config editor ----------

    pub fn editor_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Global(GField::Destination), Row::Global(GField::DateFormat), Row::Global(GField::Notify), Row::Global(GField::Theme)];
        for i in 0..self.cfg.profiles.len() {
            rows.push(Row::Header(i));
            for f in [PField::Name, PField::Subfolder, PField::Sources, PField::Include, PField::Exclude, PField::Icon, PField::Destination] {
                rows.push(Row::Field(i, f));
            }
        }
        rows.push(Row::AddProfile);
        rows
    }

    pub fn row_text(&self, row: Row) -> (String, String) {
        match row {
            Row::Global(GField::Destination) => ("Backup folder".into(), self.cfg.destination.clone()),
            Row::Global(GField::DateFormat) => ("Date folder format".into(), self.cfg.date_format.clone()),
            Row::Global(GField::Notify) => ("Notify when done".into(), if self.cfg.notify { "on" } else { "off" }.into()),
            Row::Global(GField::Theme) => ("Theme".into(), self.theme.name.into()),
            Row::Header(i) => (self.cfg.profiles[i].name.clone(), String::new()),
            Row::Field(i, f) => {
                let p = &self.cfg.profiles[i];
                match f {
                    PField::Name => ("Name".into(), p.name.clone()),
                    PField::Subfolder => ("Subfolder".into(), p.subfolder.clone()),
                    PField::Sources => ("Sources (;)".into(), p.sources.join("; ")),
                    PField::Include => ("Copy types".into(), p.include.join(", ")),
                    PField::Exclude => ("Never copy".into(), p.exclude.join(", ")),
                    PField::Icon => ("Icon".into(), p.icon.clone()),
                    PField::Destination => ("Folder override".into(), p.destination.clone().unwrap_or_default()),
                }
            }
            Row::AddProfile => ("+ Add a profile".into(), String::new()),
        }
    }

    fn config_key(&mut self, key: KeyEvent) {
        let rows = self.editor_rows();
        let n = rows.len();
        if let Some(i) = self.editor.confirm_delete {
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                let mut next = self.cfg.clone();
                next.profiles.remove(i);
                self.commit(next, "Profile deleted");
                self.editor.cursor = self.editor.cursor.min(self.editor_rows().len() - 1);
            }
            self.editor.confirm_delete = None;
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.editor.cursor = self.editor.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.editor.cursor = (self.editor.cursor + 1).min(n - 1),
            KeyCode::PageUp => self.editor.cursor = self.editor.cursor.saturating_sub(8),
            KeyCode::PageDown => self.editor.cursor = (self.editor.cursor + 8).min(n - 1),
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Row::Header(i) | Row::Field(i, _) = rows[self.editor.cursor] {
                    if self.cfg.profiles.len() > 1 {
                        self.editor.confirm_delete = Some(i);
                    } else {
                        self.toast("Can't delete the last profile", Level::Warn);
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('e') => self.activate_row(rows[self.editor.cursor]),
            KeyCode::Esc | KeyCode::Char('q') => {
                self.refresh_live();
                self.screen = Screen::Home;
            }
            _ => {}
        }
    }

    fn activate_row(&mut self, row: Row) {
        match row {
            Row::Global(GField::Notify) => {
                let mut next = self.cfg.clone();
                next.notify = !next.notify;
                self.commit(next, "Saved");
            }
            Row::Global(GField::Theme) => {
                let t = self.theme.next();
                self.set_theme(t.name);
            }
            Row::Field(i, PField::Icon) => {
                let order = ["camera", "action", "mirrorless"];
                let cur = order.iter().position(|o| *o == self.cfg.profiles[i].icon).unwrap_or(0);
                let mut next = self.cfg.clone();
                next.profiles[i].icon = order[(cur + 1) % order.len()].into();
                self.commit(next, "Saved");
            }
            Row::AddProfile => {
                let mut next = self.cfg.clone();
                let mut name = "New camera".to_string();
                let mut k = 2;
                while next.profiles.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                    name = format!("New camera {k}");
                    k += 1;
                }
                next.profiles.push(Profile {
                    name,
                    subfolder: "Camera".into(),
                    sources: vec!["/Volumes/CARD/DCIM".into()],
                    include: vec!["mp4".into(), "jpg".into()],
                    exclude: vec![],
                    icon: "camera".into(),
                    destination: None,
                });
                if self.commit(next, "Profile added - edit its fields below") {
                    self.editor.cursor = self.editor_rows().len() - 2 - 7; // its header row
                }
            }
            Row::Header(_) => {}
            other => {
                let (_, v) = self.row_text(other);
                self.editor.input = Some(v);
            }
        }
    }

    fn config_input_key(&mut self, key: KeyEvent) {
        let rows = self.editor_rows();
        let row = rows[self.editor.cursor.min(rows.len() - 1)];
        let Some(buf) = &mut self.editor.input else { return };
        match key.code {
            KeyCode::Esc => self.editor.input = None,
            KeyCode::Enter => {
                let text = buf.trim().to_string();
                if self.apply_text(row, &text) {
                    self.editor.input = None;
                }
            }
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => buf.clear(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => buf.push(c),
            _ => {}
        }
    }

    /// Apply edited text to a copy of the config; keep it only if it validates, then save.
    fn apply_text(&mut self, row: Row, text: &str) -> bool {
        let mut next = self.cfg.clone();
        let list = |s: &str, seps: &[char]| -> Vec<String> { s.split(|c| seps.contains(&c)).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect() };
        match row {
            Row::Global(GField::Destination) => next.destination = text.into(),
            Row::Global(GField::DateFormat) => next.date_format = text.into(),
            Row::Field(i, f) => {
                let p = &mut next.profiles[i];
                match f {
                    PField::Name => p.name = text.into(),
                    PField::Subfolder => p.subfolder = text.into(),
                    PField::Sources => p.sources = list(text, &[';']),
                    PField::Include => p.include = list(text, &[',', ' ']).iter().map(|e| config::norm_ext(e)).collect(),
                    PField::Exclude => p.exclude = list(text, &[',', ' ']).iter().map(|e| config::norm_ext(e)).collect(),
                    PField::Destination => p.destination = if text.is_empty() { None } else { Some(text.into()) },
                    PField::Icon => {}
                }
            }
            _ => {}
        }
        self.commit(next, "Saved")
    }

    fn commit(&mut self, next: Config, ok_msg: &str) -> bool {
        match next.save(&self.paths.config) {
            Ok(()) => {
                self.cfg = next;
                self.config_exists = true;
                self.refresh_live();
                self.toast(ok_msg, Level::Ok);
                true
            }
            Err(e) => {
                self.toast(format!("Not saved: {e:#}"), Level::Err);
                false
            }
        }
    }

    // ---------- mouse ----------

    pub fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.show_help {
                    self.show_help = false;
                    return;
                }
                let hit = self
                    .hits
                    .iter()
                    .rev()
                    .find(|(r, _)| m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height)
                    .map(|(_, h)| *h);
                if let Some(h) = hit {
                    self.click(h);
                }
            }
            _ => {}
        }
    }

    fn click(&mut self, h: Hit) {
        match (h, self.screen) {
            (Hit::Profile(i), Screen::Home) => {
                if self.home_sel == i {
                    self.begin_scan(i);
                } else {
                    self.home_sel = i;
                }
            }
            (Hit::File(i), Screen::Review) => {
                if let Some(rv) = &mut self.review {
                    rv.focus = Focus::Files;
                    rv.cursor = i;
                    if let Some(s) = rv.selected.get_mut(i) {
                        *s = !*s;
                    }
                }
            }
            (Hit::Date(i), Screen::Review) => {
                if let Some(rv) = &mut self.review {
                    rv.focus = Focus::Dates;
                    rv.date_cursor = i;
                    if let Some(d) = rv.dates.get(i).copied() {
                        rv.toggle_date(d);
                    }
                }
            }
            (Hit::Row(i), Screen::Config) if self.editor.input.is_none() => {
                let rows = self.editor_rows();
                if i < rows.len() {
                    if self.editor.cursor == i {
                        self.activate_row(rows[i]);
                    } else {
                        self.editor.cursor = i;
                    }
                }
            }
            _ => {}
        }
    }

    fn scroll_by(&mut self, d: i32) {
        let step = |v: usize, max: usize| -> usize { (v as i64 + d as i64).clamp(0, max as i64) as usize };
        match self.screen {
            Screen::Home => self.home_sel = step(self.home_sel, self.cfg.profiles.len().saturating_sub(1)),
            Screen::Review => {
                if let Some(rv) = &mut self.review {
                    match rv.focus {
                        Focus::Files => rv.cursor = step(rv.cursor, rv.scan.files.len().saturating_sub(1)),
                        Focus::Dates => rv.date_cursor = step(rv.date_cursor, rv.dates.len().saturating_sub(1)),
                    }
                }
                self.review_follow_cursor();
            }
            Screen::Running => {
                if let Some(run) = &mut self.run {
                    run.follow = false;
                    run.scroll = step(run.scroll, run.state.len().saturating_sub(1));
                }
            }
            Screen::Stats => {
                if let Some(sv) = &mut self.stats {
                    sv.scroll = step(sv.scroll, sv.records.len().saturating_sub(1));
                }
            }
            Screen::Config if self.editor.input.is_none() => {
                let n = self.editor_rows().len();
                self.editor.cursor = step(self.editor.cursor, n - 1);
            }
            _ => {}
        }
    }
}

impl Config {
    /// Destination base used for live drive-status display.
    pub fn destination_path(&self) -> PathBuf {
        config::expand_tilde(&self.destination)
    }
}

pub fn dest_live(path: &std::path::Path) -> DestLive {
    let mounted = fsutil::destination_volume_present(path);
    DestLive { mounted, space: if mounted { fsutil::disk_space(path) } else { None } }
}

fn load_theme_pref(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let t = v.get("theme")?.as_str()?.to_string();
    theme::NAMES.contains(&t.as_str()).then_some(t)
}

fn save_theme_pref(path: &std::path::Path, name: &str) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, serde_json::json!({ "theme": name }).to_string());
}
