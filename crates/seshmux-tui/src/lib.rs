mod new_flow;

use std::io::{Stdout, stdout};
use std::path::Path;

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use seshmux_app::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiExit {
    Completed,
    BackAtRoot,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootMenuExit {
    Action(RootAction),
    Exit(UiExit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootAction {
    New,
    List,
    Attach,
    Delete,
}

impl RootAction {
    fn title(self) -> &'static str {
        match self {
            Self::New => "New worktree",
            Self::List => "List worktrees (coming in next milestone)",
            Self::Attach => "Attach session (coming in next milestone)",
            Self::Delete => "Delete worktree (coming in next milestone)",
        }
    }
}

const ROOT_ACTIONS: [RootAction; 4] = [
    RootAction::New,
    RootAction::List,
    RootAction::Attach,
    RootAction::Delete,
];

pub(crate) struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    pub(crate) fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, Hide).context("failed to initialize terminal")?;
        let backend = CrosstermBackend::new(out);
        let terminal = Terminal::new(backend).context("failed to create terminal backend")?;
        Ok(Self { terminal })
    }

    pub(crate) fn draw<F>(&mut self, draw_fn: F) -> Result<()>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal
            .draw(draw_fn)
            .context("failed to render terminal")?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(self.terminal.backend_mut(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

pub(crate) fn is_ctrl_c(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

pub fn run_root(app: &App<'_>, cwd: &Path) -> Result<UiExit> {
    if let Ok(exit) = std::env::var("SESHMUX_TUI_TEST_EXIT") {
        return Ok(match exit.as_str() {
            "completed" => UiExit::Completed,
            "back" => UiExit::BackAtRoot,
            "canceled" => UiExit::Canceled,
            _ => UiExit::Completed,
        });
    }

    loop {
        match run_root_menu(cwd)? {
            RootMenuExit::Action(RootAction::New) => match new_flow::run_new_flow(app, cwd)? {
                UiExit::BackAtRoot | UiExit::Completed => continue,
                UiExit::Canceled => return Ok(UiExit::Canceled),
            },
            RootMenuExit::Action(RootAction::List)
            | RootMenuExit::Action(RootAction::Attach)
            | RootMenuExit::Action(RootAction::Delete) => continue,
            RootMenuExit::Exit(exit) => return Ok(exit),
        }
    }
}

fn run_root_menu(cwd: &Path) -> Result<RootMenuExit> {
    let mut session = TerminalSession::enter()?;
    let mut selected = 0usize;
    let mut notice: Option<String> = None;

    loop {
        session.draw(|frame| draw_root_menu(frame, cwd, selected, notice.as_deref()))?;

        let event = event::read().context("failed to read terminal event")?;
        let Event::Key(key) = event else {
            continue;
        };

        if is_ctrl_c(key) {
            return Ok(RootMenuExit::Exit(UiExit::Canceled));
        }

        if notice.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    notice = None;
                }
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Esc => return Ok(RootMenuExit::Exit(UiExit::BackAtRoot)),
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if selected + 1 < ROOT_ACTIONS.len() {
                    selected += 1;
                }
            }
            KeyCode::Enter => {
                let action = ROOT_ACTIONS[selected];
                if matches!(action, RootAction::New) {
                    return Ok(RootMenuExit::Action(action));
                }
                notice = Some("This flow ships in the next milestone.".to_string());
            }
            _ => {}
        }
    }
}

fn draw_root_menu(
    frame: &mut ratatui::Frame<'_>,
    cwd: &Path,
    selected: usize,
    notice: Option<&str>,
) {
    let area = frame.area();
    let [header, body, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .areas(area);

    let title = Paragraph::new(format!(
        "seshmux\n{}\nSelect a workflow",
        cwd.to_string_lossy()
    ))
    .block(Block::default().borders(Borders::ALL).title("Root"));
    frame.render_widget(title, header);

    let items: Vec<ListItem<'_>> = ROOT_ACTIONS
        .iter()
        .map(|action| ListItem::new(action.title()))
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Actions"))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(list, body, &mut state);

    let hints = Paragraph::new("Enter: select    Esc: exit    Ctrl+C: cancel")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(hints, footer);

    if let Some(message) = notice {
        let popup = centered_rect(70, 20, area);
        frame.render_widget(Clear, popup);
        let paragraph = Paragraph::new(format!("{message}\n\nEnter/Esc to return"))
            .block(Block::default().borders(Borders::ALL).title("Notice"));
        frame.render_widget(paragraph, popup);
    }
}

pub(crate) fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let [vertical] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .areas(area);
    let [horizontal] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .areas(vertical);
    horizontal
}
