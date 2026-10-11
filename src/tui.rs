//! Terminal setup and the event loop.

use crate::app::App;
use crate::engine::Event as EngineEvent;
use crate::ui;
use anyhow::Result;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use std::io::stdout;
use std::time::{Duration, Instant};

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

pub fn run(mut app: App) -> Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    // Never leave the user's terminal broken, even on panic.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    let mut term = Terminal::new(CrosstermBackend::new(stdout()))?;
    let result = (|| -> Result<()> {
        loop {
            term.draw(|f| ui::draw(f, &mut app))?;
            if event::poll(Duration::from_millis(50))? {
                // Drain everything pending so scrolling stays responsive.
                loop {
                    match event::read()? {
                        Event::Key(k) if k.kind == KeyEventKind::Press => app.on_key(k),
                        Event::Mouse(m) => app.on_mouse(m),
                        _ => {}
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
            app.tick();
            if app.should_quit {
                break;
            }
        }
        Ok(())
    })();
    // If quitting mid-copy, let the engine clean up its partial files.
    if let Some(run) = app.run.take() {
        run.ctrl.set_paused(false);
        run.ctrl.cancel();
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            match run.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(EngineEvent::Finished(_)) => break,
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
        }
    }
    restore();
    result
}
