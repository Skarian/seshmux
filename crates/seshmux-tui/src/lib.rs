mod attach_flow;
mod delete_flow;
mod list_flow;
mod new_flow;

use std::io::{Stdout, stdout};
use std::path::Path;

use anyhow::{Context, Result};
use attach_flow::AttachScreen;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use delete_flow::DeleteScreen;
use list_flow::ListScreen;
use new_flow::NewScreen;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
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
            Self::List => "List worktrees",
            Self::Attach => "Attach session",
            Self::Delete => "Delete worktree",
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

fn read_key_event() -> Result<KeyEvent> {
    loop {
        let event = event::read().context("failed to read terminal event")?;
        let Event::Key(key) = event else {
            continue;
        };

        if !matches!(key.kind, KeyEventKind::Press) {
            continue;
        }

        return Ok(key);
    }
}

#[derive(Debug)]
struct RootScreen {
    selected: usize,
}

impl RootScreen {
    fn new() -> Self {
        Self { selected: 0 }
    }

    fn on_key(&mut self, key: KeyEvent) -> Option<RootMenuExit> {
        match key.code {
            KeyCode::Esc => None,
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                if self.selected + 1 < ROOT_ACTIONS.len() {
                    self.selected += 1;
                }
                None
            }
            KeyCode::Enter => Some(RootMenuExit::Action(ROOT_ACTIONS[self.selected])),
            _ => None,
        }
    }

    fn render(&self, frame: &mut ratatui::Frame<'_>, cwd: &Path) {
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
        state.select(Some(self.selected));
        frame.render_stateful_widget(list, body, &mut state);

        let hints = Paragraph::new("Enter: select    Up/Down: move    Ctrl+C: cancel")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(hints, footer);
    }
}

enum ActiveScreen {
    Root(RootScreen),
    New(Box<NewScreen>),
    List(Box<ListScreen>),
    Attach(Box<AttachScreen>),
    Delete(Box<DeleteScreen>),
}

enum Transition {
    Open(RootAction),
    Return(UiExit),
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

    let mut session = TerminalSession::enter()?;
    let mut active = ActiveScreen::Root(RootScreen::new());

    loop {
        session.draw(|frame| match &active {
            ActiveScreen::Root(screen) => screen.render(frame, cwd),
            ActiveScreen::New(screen) => screen.render(frame),
            ActiveScreen::List(screen) => screen.render(frame),
            ActiveScreen::Attach(screen) => screen.render(frame),
            ActiveScreen::Delete(screen) => screen.render(frame),
        })?;

        let key = read_key_event()?;

        if is_ctrl_c(key) {
            return Ok(UiExit::Canceled);
        }

        let transition = match &mut active {
            ActiveScreen::Root(screen) => screen
                .on_key(key)
                .map(|RootMenuExit::Action(action)| Transition::Open(action)),
            ActiveScreen::New(screen) => screen.on_key(key, app)?.map(Transition::Return),
            ActiveScreen::List(screen) => screen.on_key(key, app)?.map(Transition::Return),
            ActiveScreen::Attach(screen) => screen.on_key(key, app)?.map(Transition::Return),
            ActiveScreen::Delete(screen) => screen.on_key(key, app)?.map(Transition::Return),
        };

        if let Some(transition) = transition {
            match transition {
                Transition::Open(action) => {
                    active = match action {
                        RootAction::New => ActiveScreen::New(Box::new(NewScreen::new(app, cwd)?)),
                        RootAction::List => {
                            ActiveScreen::List(Box::new(ListScreen::new(app, cwd)?))
                        }
                        RootAction::Attach => {
                            ActiveScreen::Attach(Box::new(AttachScreen::new(app, cwd)?))
                        }
                        RootAction::Delete => {
                            ActiveScreen::Delete(Box::new(DeleteScreen::new(app, cwd)?))
                        }
                    };
                }
                Transition::Return(exit) => match exit {
                    UiExit::BackAtRoot | UiExit::Completed => {
                        active = ActiveScreen::Root(RootScreen::new());
                    }
                    UiExit::Canceled => return Ok(UiExit::Canceled),
                },
            }
        }
    }
}

pub(crate) fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let pct_x = percent_x.min(100);
    let pct_y = percent_y.min(100);

    let [_, vertical, _] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .areas(area);
    let [_, horizontal, _] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .areas(vertical);
    horizontal
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::centered_rect;

    #[test]
    fn centered_rect_returns_middle_segment() {
        let area = Rect::new(0, 0, 100, 50);
        let centered = centered_rect(80, 60, area);

        assert_eq!(centered.width, 80);
        assert_eq!(centered.height, 30);
        assert_eq!(centered.x, 10);
        assert_eq!(centered.y, 10);
    }

    #[test]
    fn centered_rect_clamps_percentages_over_100() {
        let area = Rect::new(3, 4, 40, 20);
        let centered = centered_rect(120, 150, area);

        assert_eq!(centered, area);
    }
}
