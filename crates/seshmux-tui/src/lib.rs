mod attach_flow;
mod delete_flow;
mod keymap;
mod list_flow;
mod new_flow;
mod theme;
mod ui;

use std::io::{Stdout, stdout};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
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
use ratatui::style::Color;
use ratatui::widgets::{List, ListItem, ListState};
use seshmux_app::App;

use crate::ui::modal::render_error_modal;
use crate::ui::text::{compact_hint, wrapped_paragraph};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiExit {
    Completed,
    BackAtRoot,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootMenuExit {
    Action(RootAction),
    Exit,
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
        let terminal = enter_with_ops(
            || enable_raw_mode().context("failed to enable raw mode"),
            || {
                let mut out = stdout();
                execute!(out, EnterAlternateScreen, Hide).context("failed to initialize terminal")
            },
            || {
                let backend = CrosstermBackend::new(stdout());
                Terminal::new(backend).context("failed to create terminal backend")
            },
            || {
                let mut out = stdout();
                execute!(out, Show, LeaveAlternateScreen)
                    .context("failed to restore terminal screen during rollback")
            },
            || disable_raw_mode().context("failed to disable raw mode during rollback"),
        )?;
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

    pub(crate) fn autoresize(&mut self) -> Result<()> {
        self.terminal
            .autoresize()
            .context("failed to autoresize terminal")?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(self.terminal.backend_mut(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn enter_with_ops<
    T,
    EnableRawMode,
    EnterAltScreen,
    CreateTerminal,
    LeaveAltScreen,
    DisableRawMode,
>(
    mut enable_raw_mode_op: EnableRawMode,
    mut enter_alt_screen_op: EnterAltScreen,
    mut create_terminal_op: CreateTerminal,
    mut leave_alt_screen_op: LeaveAltScreen,
    mut disable_raw_mode_op: DisableRawMode,
) -> Result<T>
where
    EnableRawMode: FnMut() -> Result<()>,
    EnterAltScreen: FnMut() -> Result<()>,
    CreateTerminal: FnMut() -> Result<T>,
    LeaveAltScreen: FnMut() -> Result<()>,
    DisableRawMode: FnMut() -> Result<()>,
{
    enable_raw_mode_op()?;

    if let Err(error) = enter_alt_screen_op() {
        return Err(failure_with_rollback(
            error,
            true,
            false,
            &mut leave_alt_screen_op,
            &mut disable_raw_mode_op,
        ));
    }

    match create_terminal_op() {
        Ok(terminal) => Ok(terminal),
        Err(error) => Err(failure_with_rollback(
            error,
            true,
            true,
            &mut leave_alt_screen_op,
            &mut disable_raw_mode_op,
        )),
    }
}

fn failure_with_rollback<LeaveAltScreen, DisableRawMode>(
    setup_error: anyhow::Error,
    raw_enabled: bool,
    alt_screen_entered: bool,
    leave_alt_screen_op: &mut LeaveAltScreen,
    disable_raw_mode_op: &mut DisableRawMode,
) -> anyhow::Error
where
    LeaveAltScreen: FnMut() -> Result<()>,
    DisableRawMode: FnMut() -> Result<()>,
{
    let cleanup_error = rollback_partial_terminal_setup(
        raw_enabled,
        alt_screen_entered,
        leave_alt_screen_op,
        disable_raw_mode_op,
    );

    match cleanup_error {
        Some(cleanup_error) => {
            anyhow!("{setup_error:#}\nterminal rollback cleanup failed: {cleanup_error:#}")
        }
        None => setup_error,
    }
}

fn rollback_partial_terminal_setup<LeaveAltScreen, DisableRawMode>(
    raw_enabled: bool,
    alt_screen_entered: bool,
    leave_alt_screen_op: &mut LeaveAltScreen,
    disable_raw_mode_op: &mut DisableRawMode,
) -> Option<anyhow::Error>
where
    LeaveAltScreen: FnMut() -> Result<()>,
    DisableRawMode: FnMut() -> Result<()>,
{
    let mut cleanup_failures = Vec::<String>::new();

    if alt_screen_entered && let Err(error) = leave_alt_screen_op() {
        cleanup_failures.push(format!(
            "failed to restore alternate screen during rollback: {error:#}"
        ));
    }

    if raw_enabled && let Err(error) = disable_raw_mode_op() {
        cleanup_failures.push(format!(
            "failed to disable raw mode during rollback: {error:#}"
        ));
    }

    if cleanup_failures.is_empty() {
        None
    } else {
        Some(anyhow!(cleanup_failures.join("\n")))
    }
}

pub(crate) fn is_ctrl_c(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
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
        if keymap::is_back(key) || keymap::is_quit(key) {
            return Some(RootMenuExit::Exit);
        }

        if keymap::is_up(key) {
            self.selected = self.selected.saturating_sub(1);
            return None;
        }

        if keymap::is_down(key) {
            if self.selected + 1 < ROOT_ACTIONS.len() {
                self.selected += 1;
            }
            return None;
        }

        if keymap::is_confirm(key) {
            return Some(RootMenuExit::Action(ROOT_ACTIONS[self.selected]));
        }

        None
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

        let title = wrapped_paragraph(format!(
            "seshmux\n{}\nSelect a workflow",
            cwd.to_string_lossy()
        ))
        .block(theme::chrome("Root"));
        frame.render_widget(title, header);

        let items: Vec<ListItem<'_>> = ROOT_ACTIONS
            .iter()
            .map(|action| ListItem::new(action.title()))
            .collect();
        let list = List::new(items)
            .block(theme::chrome("Actions"))
            .highlight_style(theme::table_highlight(Color::Cyan));

        let mut state = ListState::default();
        state.select(Some(self.selected));
        frame.render_stateful_widget(list, body, &mut state);

        let hints = wrapped_paragraph(compact_hint(
            area.width,
            "Enter: select    Up/Down or j/k: move    Esc/q: exit",
            "Enter: select    j/k: move    Esc/q: exit",
            "Enter: select | j/k: move | Esc/q: exit",
        ))
        .block(theme::key_block());
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
    let mut session = TerminalSession::enter()?;
    let mut active = ActiveScreen::Root(RootScreen::new());
    let mut global_error: Option<String> = None;

    loop {
        session.draw(|frame| {
            match &active {
                ActiveScreen::Root(screen) => screen.render(frame, cwd),
                ActiveScreen::New(screen) => screen.render(frame),
                ActiveScreen::List(screen) => screen.render(frame),
                ActiveScreen::Attach(screen) => screen.render(frame),
                ActiveScreen::Delete(screen) => screen.render(frame),
            }

            if let Some(message) = global_error.as_deref() {
                render_global_error(frame, message);
            }
        })?;

        let event = event::read().context("failed to read terminal event")?;
        let key = match event {
            Event::Resize(_, _) => {
                session.autoresize()?;
                continue;
            }
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press) => key,
            _ => continue,
        };

        if is_ctrl_c(key) {
            return Ok(UiExit::Canceled);
        }

        if global_error.is_some() {
            if keymap::is_confirm(key) || keymap::is_back(key) {
                global_error = None;
            }
            continue;
        }

        let transition = match &mut active {
            ActiveScreen::Root(screen) => match screen.on_key(key) {
                Some(RootMenuExit::Action(action)) => Some(Transition::Open(action)),
                Some(RootMenuExit::Exit) => Some(Transition::Return(UiExit::Completed)),
                None => None,
            },
            ActiveScreen::New(screen) => match screen.on_key(key, app) {
                Ok(value) => value.map(Transition::Return),
                Err(error) => {
                    global_error = Some(format!("{error:#}"));
                    None
                }
            },
            ActiveScreen::List(screen) => match screen.on_key(key, app) {
                Ok(value) => value.map(Transition::Return),
                Err(error) => {
                    global_error = Some(format!("{error:#}"));
                    None
                }
            },
            ActiveScreen::Attach(screen) => match screen.on_key(key, app) {
                Ok(value) => value.map(Transition::Return),
                Err(error) => {
                    global_error = Some(format!("{error:#}"));
                    None
                }
            },
            ActiveScreen::Delete(screen) => match screen.on_key(key, app) {
                Ok(value) => value.map(Transition::Return),
                Err(error) => {
                    global_error = Some(format!("{error:#}"));
                    None
                }
            },
        };

        if let Some(transition) = transition {
            match transition {
                Transition::Open(action) => {
                    match action {
                        RootAction::New => match NewScreen::new(app, cwd) {
                            Ok(screen) => active = ActiveScreen::New(Box::new(screen)),
                            Err(error) => global_error = Some(format!("{error:#}")),
                        },
                        RootAction::List => match ListScreen::new(app, cwd) {
                            Ok(screen) => active = ActiveScreen::List(Box::new(screen)),
                            Err(error) => global_error = Some(format!("{error:#}")),
                        },
                        RootAction::Attach => match AttachScreen::new(app, cwd) {
                            Ok(screen) => active = ActiveScreen::Attach(Box::new(screen)),
                            Err(error) => global_error = Some(format!("{error:#}")),
                        },
                        RootAction::Delete => match DeleteScreen::new(app, cwd) {
                            Ok(screen) => active = ActiveScreen::Delete(Box::new(screen)),
                            Err(error) => global_error = Some(format!("{error:#}")),
                        },
                    };
                }
                Transition::Return(UiExit::Canceled) => return Ok(UiExit::Canceled),
                Transition::Return(UiExit::Completed) => return Ok(UiExit::Completed),
                Transition::Return(UiExit::BackAtRoot) => {
                    active = ActiveScreen::Root(RootScreen::new());
                }
            }
        }
    }
}

fn render_global_error(frame: &mut ratatui::Frame<'_>, message: &str) {
    let text = format!("Operation failed.\n\n{message}");
    render_error_modal(frame, &text, 88, 72, "Enter/Esc to continue.");
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
    use anyhow::anyhow;
    use std::cell::RefCell;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;

    use super::{RootMenuExit, RootScreen, centered_rect, enter_with_ops};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

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

    #[test]
    fn root_screen_esc_and_q_exit() {
        let mut root = RootScreen::new();
        assert_eq!(root.on_key(key(KeyCode::Esc)), Some(RootMenuExit::Exit));
        assert_eq!(
            root.on_key(key(KeyCode::Char('q'))),
            Some(RootMenuExit::Exit)
        );
    }

    #[test]
    fn root_screen_supports_j_and_k_navigation() {
        let mut root = RootScreen::new();
        let _ = root.on_key(key(KeyCode::Char('j')));
        let _ = root.on_key(key(KeyCode::Char('j')));
        assert_eq!(root.selected, 2);

        let _ = root.on_key(key(KeyCode::Char('k')));
        assert_eq!(root.selected, 1);
    }

    #[test]
    fn enter_with_ops_rolls_back_raw_mode_when_alt_screen_step_fails() {
        let calls = RefCell::new(Vec::<&'static str>::new());

        let error = enter_with_ops(
            || {
                calls.borrow_mut().push("enable_raw_mode");
                Ok(())
            },
            || {
                calls.borrow_mut().push("enter_alt_screen");
                Err(anyhow!("enter alt failed"))
            },
            || {
                calls.borrow_mut().push("create_terminal");
                Ok(())
            },
            || {
                calls.borrow_mut().push("leave_alt_screen");
                Ok(())
            },
            || {
                calls.borrow_mut().push("disable_raw_mode");
                Ok(())
            },
        )
        .expect_err("enter should fail");

        assert_eq!(
            calls.into_inner(),
            vec!["enable_raw_mode", "enter_alt_screen", "disable_raw_mode"]
        );
        assert!(format!("{error:#}").contains("enter alt failed"));
    }

    #[test]
    fn enter_with_ops_rolls_back_alt_screen_then_raw_mode_when_terminal_creation_fails() {
        let calls = RefCell::new(Vec::<&'static str>::new());

        let error = enter_with_ops(
            || {
                calls.borrow_mut().push("enable_raw_mode");
                Ok(())
            },
            || {
                calls.borrow_mut().push("enter_alt_screen");
                Ok(())
            },
            || {
                calls.borrow_mut().push("create_terminal");
                Err::<(), _>(anyhow!("create terminal failed"))
            },
            || {
                calls.borrow_mut().push("leave_alt_screen");
                Ok(())
            },
            || {
                calls.borrow_mut().push("disable_raw_mode");
                Ok(())
            },
        )
        .expect_err("enter should fail");

        assert_eq!(
            calls.into_inner(),
            vec![
                "enable_raw_mode",
                "enter_alt_screen",
                "create_terminal",
                "leave_alt_screen",
                "disable_raw_mode",
            ]
        );
        assert!(format!("{error:#}").contains("create terminal failed"));
    }

    #[test]
    fn enter_with_ops_attempts_both_cleanup_steps_when_alt_cleanup_fails() {
        let calls = RefCell::new(Vec::<&'static str>::new());

        let error = enter_with_ops(
            || {
                calls.borrow_mut().push("enable_raw_mode");
                Ok(())
            },
            || {
                calls.borrow_mut().push("enter_alt_screen");
                Ok(())
            },
            || {
                calls.borrow_mut().push("create_terminal");
                Err::<(), _>(anyhow!("create terminal failed"))
            },
            || {
                calls.borrow_mut().push("leave_alt_screen");
                Err(anyhow!("leave alt failed"))
            },
            || {
                calls.borrow_mut().push("disable_raw_mode");
                Err(anyhow!("disable raw failed"))
            },
        )
        .expect_err("enter should fail");

        assert_eq!(
            calls.into_inner(),
            vec![
                "enable_raw_mode",
                "enter_alt_screen",
                "create_terminal",
                "leave_alt_screen",
                "disable_raw_mode",
            ]
        );

        let message = format!("{error:#}");
        assert!(message.contains("create terminal failed"));
        assert!(message.contains("leave alt failed"));
        assert!(message.contains("disable raw failed"));
    }
}
