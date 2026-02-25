use std::io::{Write, stdout};
use std::path::Path;

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use seshmux_app::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiExit {
    Completed,
    BackAtRoot,
    Canceled,
}

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, Hide).context("failed to initialize terminal")?;
        Ok(Self)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let mut out = stdout();
        let _ = execute!(out, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

pub fn run_root(_app: &App<'_>, cwd: &Path) -> Result<UiExit> {
    if let Ok(exit) = std::env::var("SESHMUX_TUI_TEST_EXIT") {
        return Ok(match exit.as_str() {
            "completed" => UiExit::Completed,
            "back" => UiExit::BackAtRoot,
            "canceled" => UiExit::Canceled,
            _ => UiExit::Completed,
        });
    }

    let _session = TerminalSession::enter()?;
    draw_placeholder(cwd)?;

    loop {
        let event = event::read().context("failed to read terminal event")?;
        if let Event::Key(key) = event {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(UiExit::Canceled);
            }

            match key.code {
                KeyCode::Esc => return Ok(UiExit::BackAtRoot),
                KeyCode::Enter => return Ok(UiExit::Completed),
                _ => {}
            }
        }
    }
}

fn draw_placeholder(cwd: &Path) -> Result<()> {
    let mut out = stdout();
    execute!(out, Clear(ClearType::All), MoveTo(0, 0)).context("failed to draw tui placeholder")?;

    writeln!(out, "seshmux")?;
    writeln!(out)?;
    writeln!(out, "Root TUI stub initialized for repo:")?;
    writeln!(out, "{}", cwd.display())?;
    writeln!(out)?;
    writeln!(out, "Enter: continue")?;
    writeln!(out, "Esc: back")?;
    writeln!(out, "Ctrl+C: cancel")?;
    out.flush().context("failed to flush terminal output")?;

    Ok(())
}
