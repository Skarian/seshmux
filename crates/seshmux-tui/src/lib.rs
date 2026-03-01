mod attach_flow;
mod delete_flow;
mod keymap;
mod list_flow;
mod new_flow;
mod theme;
mod ui;

use std::io::{Stdout, stdout};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use attach_flow::AttachScreen;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
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
use ratatui::text::{Line, Text};
use ratatui::widgets::{List, ListItem, ListState};
use seshmux_app::App;

use crate::ui::modal::render_error_modal;
use crate::ui::text::{
    compact_hint, focus_line, key_hint_height, key_hint_paragraph, wrapped_paragraph,
};

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
            Self::Attach => "Attach to tmux session",
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
                execute!(out, EnterAlternateScreen, Hide)
                    .context("failed to enter alternate screen")
            },
            || {
                let mut out = stdout();
                execute!(out, EnableMouseCapture).context("failed to enable mouse capture")
            },
            || {
                let backend = CrosstermBackend::new(stdout());
                Terminal::new(backend).context("failed to create terminal backend")
            },
            || {
                let mut out = stdout();
                execute!(out, DisableMouseCapture)
                    .context("failed to disable mouse capture during rollback")
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
        let _ = execute!(self.terminal.backend_mut(), DisableMouseCapture);
        let _ = execute!(self.terminal.backend_mut(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn enter_with_ops<
    T,
    EnableRawMode,
    EnterAltScreen,
    EnableMouseCaptureOp,
    CreateTerminal,
    DisableMouseCaptureOp,
    LeaveAltScreen,
    DisableRawMode,
>(
    mut enable_raw_mode_op: EnableRawMode,
    mut enter_alt_screen_op: EnterAltScreen,
    mut enable_mouse_capture_op: EnableMouseCaptureOp,
    mut create_terminal_op: CreateTerminal,
    mut disable_mouse_capture_op: DisableMouseCaptureOp,
    mut leave_alt_screen_op: LeaveAltScreen,
    mut disable_raw_mode_op: DisableRawMode,
) -> Result<T>
where
    EnableRawMode: FnMut() -> Result<()>,
    EnterAltScreen: FnMut() -> Result<()>,
    EnableMouseCaptureOp: FnMut() -> Result<()>,
    CreateTerminal: FnMut() -> Result<T>,
    DisableMouseCaptureOp: FnMut() -> Result<()>,
    LeaveAltScreen: FnMut() -> Result<()>,
    DisableRawMode: FnMut() -> Result<()>,
{
    enable_raw_mode_op()?;

    if let Err(error) = enter_alt_screen_op() {
        return Err(failure_with_rollback(
            error,
            true,
            false,
            false,
            &mut disable_mouse_capture_op,
            &mut leave_alt_screen_op,
            &mut disable_raw_mode_op,
        ));
    }

    if let Err(error) = enable_mouse_capture_op() {
        return Err(failure_with_rollback(
            error,
            true,
            true,
            false,
            &mut disable_mouse_capture_op,
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
            true,
            &mut disable_mouse_capture_op,
            &mut leave_alt_screen_op,
            &mut disable_raw_mode_op,
        )),
    }
}

fn failure_with_rollback<DisableMouseCaptureOp, LeaveAltScreen, DisableRawMode>(
    setup_error: anyhow::Error,
    raw_enabled: bool,
    alt_screen_entered: bool,
    mouse_capture_enabled: bool,
    disable_mouse_capture_op: &mut DisableMouseCaptureOp,
    leave_alt_screen_op: &mut LeaveAltScreen,
    disable_raw_mode_op: &mut DisableRawMode,
) -> anyhow::Error
where
    DisableMouseCaptureOp: FnMut() -> Result<()>,
    LeaveAltScreen: FnMut() -> Result<()>,
    DisableRawMode: FnMut() -> Result<()>,
{
    let cleanup_error = rollback_partial_terminal_setup(
        raw_enabled,
        alt_screen_entered,
        mouse_capture_enabled,
        disable_mouse_capture_op,
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

fn rollback_partial_terminal_setup<DisableMouseCaptureOp, LeaveAltScreen, DisableRawMode>(
    raw_enabled: bool,
    alt_screen_entered: bool,
    mouse_capture_enabled: bool,
    disable_mouse_capture_op: &mut DisableMouseCaptureOp,
    leave_alt_screen_op: &mut LeaveAltScreen,
    disable_raw_mode_op: &mut DisableRawMode,
) -> Option<anyhow::Error>
where
    DisableMouseCaptureOp: FnMut() -> Result<()>,
    LeaveAltScreen: FnMut() -> Result<()>,
    DisableRawMode: FnMut() -> Result<()>,
{
    let mut cleanup_failures = Vec::<String>::new();

    if mouse_capture_enabled && let Err(error) = disable_mouse_capture_op() {
        cleanup_failures.push(format!(
            "failed to disable mouse capture during rollback: {error:#}"
        ));
    }

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

#[cfg(test)]
fn leave_with_ops<DisableMouseCaptureOp, LeaveAltScreen, DisableRawMode>(
    mut disable_mouse_capture_op: DisableMouseCaptureOp,
    mut leave_alt_screen_op: LeaveAltScreen,
    mut disable_raw_mode_op: DisableRawMode,
) where
    DisableMouseCaptureOp: FnMut() -> Result<()>,
    LeaveAltScreen: FnMut() -> Result<()>,
    DisableRawMode: FnMut() -> Result<()>,
{
    let _ = disable_mouse_capture_op();
    let _ = leave_alt_screen_op();
    let _ = disable_raw_mode_op();
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
        let key_text = compact_hint(
            area.width,
            "Enter: select    Up/Down or j/k: move    Esc/q: exit",
            "Enter: select    j/k: move    Esc/q: exit",
            "Enter: select | j/k: move | Esc/q: exit",
        );
        let footer_height = key_hint_height(area.width, key_text);
        let [header, body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(8),
                Constraint::Length(footer_height),
            ])
            .areas(area);

        let header_text = Text::from(vec![
            Line::from("seshmux"),
            Line::from(cwd.to_string_lossy().to_string()),
            focus_line("Choose what you want to do"),
        ]);
        let title = wrapped_paragraph(header_text).block(theme::chrome("Home"));
        frame.render_widget(title, header);

        let items: Vec<ListItem<'_>> = ROOT_ACTIONS
            .iter()
            .map(|action| ListItem::new(action.title()))
            .collect();
        let list = List::new(items)
            .block(theme::chrome(focus_line("Actions")))
            .highlight_style(theme::table_highlight(Color::Cyan));

        let mut state = ListState::default();
        state.select(Some(self.selected));
        frame.render_stateful_widget(list, body, &mut state);

        let hints = key_hint_paragraph(key_text).block(theme::key_block());
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewFlowDrainReason {
    Timeout,
    AfterInput,
}

trait RootLoopTickTarget {
    fn on_tick(&mut self) -> Result<()>;
    fn should_drain_loader_after_input(&self) -> bool;
}

impl RootLoopTickTarget for NewScreen {
    fn on_tick(&mut self) -> Result<()> {
        NewScreen::on_tick(self)
    }

    fn should_drain_loader_after_input(&self) -> bool {
        NewScreen::should_drain_loader_after_input(self)
    }
}

fn root_loop_drain_helper<T: RootLoopTickTarget>(
    target: &mut T,
    reason: NewFlowDrainReason,
) -> Result<bool> {
    if !matches!(reason, NewFlowDrainReason::Timeout) && !target.should_drain_loader_after_input() {
        return Ok(false);
    }

    target.on_tick()?;
    Ok(true)
}

fn root_loop_drain_new_flow_loader(
    active: &mut ActiveScreen,
    reason: NewFlowDrainReason,
) -> Result<bool> {
    let ActiveScreen::New(screen) = active else {
        return Ok(false);
    };

    root_loop_drain_helper(screen.as_mut(), reason)
}

pub fn run_root(app: &App<'_>, cwd: &Path) -> Result<UiExit> {
    let mut session = TerminalSession::enter()?;
    let mut active = ActiveScreen::Root(RootScreen::new());
    let mut global_error: Option<String> = None;
    const TICK_RATE: Duration = Duration::from_millis(120);

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

        let has_event = event::poll(TICK_RATE).context("failed to poll terminal event")?;
        if !has_event {
            if let Err(error) =
                root_loop_drain_new_flow_loader(&mut active, NewFlowDrainReason::Timeout)
            {
                global_error = Some(format!("{error:#}"));
            }
            continue;
        }

        let event = event::read().context("failed to read terminal event")?;
        let key = match event {
            Event::Resize(_, _) => {
                session.autoresize()?;
                continue;
            }
            Event::Mouse(mouse) => {
                if global_error.is_none()
                    && let ActiveScreen::New(screen) = &mut active
                    && let Err(error) = screen.on_mouse(mouse)
                {
                    global_error = Some(format!("{error:#}"));
                }
                if global_error.is_none()
                    && let Err(error) =
                        root_loop_drain_new_flow_loader(&mut active, NewFlowDrainReason::AfterInput)
                {
                    global_error = Some(format!("{error:#}"));
                }
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

        if global_error.is_none()
            && let Err(error) =
                root_loop_drain_new_flow_loader(&mut active, NewFlowDrainReason::AfterInput)
        {
            global_error = Some(format!("{error:#}"));
        }
    }
}

fn render_global_error(frame: &mut ratatui::Frame<'_>, message: &str) {
    let text = format!("Operation failed.\n\n{message}");
    render_error_modal(frame, &text, 88, 72, "Enter/Esc: continue");
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

    use super::{
        NewFlowDrainReason, RootLoopTickTarget, RootMenuExit, RootScreen, centered_rect,
        enter_with_ops, leave_with_ops, root_loop_drain_helper,
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    struct TickProbe {
        ticks: usize,
        drain_after_input: bool,
    }

    impl RootLoopTickTarget for TickProbe {
        fn on_tick(&mut self) -> anyhow::Result<()> {
            self.ticks += 1;
            Ok(())
        }

        fn should_drain_loader_after_input(&self) -> bool {
            self.drain_after_input
        }
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
    fn root_loop_drain_helper_runs_new_flow_tick_on_timeout_and_after_input() {
        let mut timeout_probe = TickProbe {
            ticks: 0,
            drain_after_input: false,
        };
        let timeout_did_tick =
            root_loop_drain_helper(&mut timeout_probe, NewFlowDrainReason::Timeout)
                .expect("timeout tick");
        assert!(timeout_did_tick);
        assert_eq!(timeout_probe.ticks, 1);

        let mut input_probe = TickProbe {
            ticks: 0,
            drain_after_input: true,
        };
        let input_did_tick =
            root_loop_drain_helper(&mut input_probe, NewFlowDrainReason::AfterInput)
                .expect("input tick");
        assert!(input_did_tick);
        assert_eq!(input_probe.ticks, 1);
    }

    #[test]
    fn root_loop_drains_new_flow_loader_after_handled_input_when_loading() {
        let mut probe = TickProbe {
            ticks: 0,
            drain_after_input: true,
        };

        let did_tick = root_loop_drain_helper(&mut probe, NewFlowDrainReason::AfterInput)
            .expect("input drain");
        assert!(did_tick);
        assert_eq!(probe.ticks, 1);
    }

    #[test]
    fn root_loop_drains_new_flow_loader_after_repeated_key_and_mouse_input_when_loading() {
        let mut probe = TickProbe {
            ticks: 0,
            drain_after_input: true,
        };

        for _ in 0..20 {
            let _ = root_loop_drain_helper(&mut probe, NewFlowDrainReason::AfterInput)
                .expect("repeated input drain");
        }

        assert_eq!(probe.ticks, 20);
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
                calls.borrow_mut().push("enable_mouse_capture");
                Ok(())
            },
            || {
                calls.borrow_mut().push("create_terminal");
                Ok(())
            },
            || {
                calls.borrow_mut().push("disable_mouse_capture");
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
                calls.borrow_mut().push("enable_mouse_capture");
                Ok(())
            },
            || {
                calls.borrow_mut().push("create_terminal");
                Err::<(), _>(anyhow!("create terminal failed"))
            },
            || {
                calls.borrow_mut().push("disable_mouse_capture");
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
            vec![
                "enable_raw_mode",
                "enter_alt_screen",
                "enable_mouse_capture",
                "create_terminal",
                "disable_mouse_capture",
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
                calls.borrow_mut().push("enable_mouse_capture");
                Ok(())
            },
            || {
                calls.borrow_mut().push("create_terminal");
                Err::<(), _>(anyhow!("create terminal failed"))
            },
            || {
                calls.borrow_mut().push("disable_mouse_capture");
                Err(anyhow!("disable mouse failed"))
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
                "enable_mouse_capture",
                "create_terminal",
                "disable_mouse_capture",
                "leave_alt_screen",
                "disable_raw_mode",
            ]
        );

        let message = format!("{error:#}");
        assert!(message.contains("create terminal failed"));
        assert!(message.contains("disable mouse failed"));
        assert!(message.contains("leave alt failed"));
        assert!(message.contains("disable raw failed"));
    }

    #[test]
    fn enter_with_ops_success_sequence_enables_mouse_after_alt_screen() {
        let calls = RefCell::new(Vec::<&'static str>::new());

        let value = enter_with_ops(
            || {
                calls.borrow_mut().push("enable_raw_mode");
                Ok(())
            },
            || {
                calls.borrow_mut().push("enter_alt_screen");
                Ok(())
            },
            || {
                calls.borrow_mut().push("enable_mouse_capture");
                Ok(())
            },
            || {
                calls.borrow_mut().push("create_terminal");
                Ok::<_, anyhow::Error>("terminal")
            },
            || {
                calls.borrow_mut().push("disable_mouse_capture");
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
        .expect("enter should succeed");

        assert_eq!(value, "terminal");
        assert_eq!(
            calls.into_inner(),
            vec![
                "enable_raw_mode",
                "enter_alt_screen",
                "enable_mouse_capture",
                "create_terminal",
            ]
        );
    }

    #[test]
    fn leave_with_ops_disables_mouse_before_alt_and_raw_teardown() {
        let calls = RefCell::new(Vec::<&'static str>::new());

        leave_with_ops(
            || {
                calls.borrow_mut().push("disable_mouse_capture");
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
        );

        assert_eq!(
            calls.into_inner(),
            vec![
                "disable_mouse_capture",
                "leave_alt_screen",
                "disable_raw_mode"
            ]
        );
    }
}
