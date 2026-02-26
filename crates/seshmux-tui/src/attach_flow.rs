use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Color;
use ratatui::text::{Line, Text};
use seshmux_app::{App, AttachError, AttachRequest, AttachResult, ListResult};

use crate::UiExit;
use crate::keymap;
use crate::theme;
use crate::ui::binary_choice::{BinaryChoice, BinaryChoiceEvent};
use crate::ui::modal::{ModalSpec, render_error_modal, render_modal};
use crate::ui::select_step::{SelectSignal, SelectStepState};
use crate::ui::text::{
    compact_hint, focus_line, highlighted_label_value_line, key_hint_height, key_hint_paragraph,
    label_value_line, result_footer, yes_no,
};
use crate::ui::worktree_table::{TableColumn, WorktreeTableRender};

pub(crate) trait AttachFlowOps {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult>;
    fn attach_worktree(&self, request: AttachRequest) -> Result<AttachResult>;
}

impl<'a> AttachFlowOps for App<'a> {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult> {
        self.list(cwd)
    }

    fn attach_worktree(&self, request: AttachRequest) -> Result<AttachResult> {
        self.attach(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    SelectWorktree,
    MissingSessionPrompt,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSignal {
    Continue,
    Exit(UiExit),
}

#[derive(Debug)]
struct AttachFlow {
    cwd: PathBuf,
    step: Step,
    select: SelectStepState,
    missing_choice: BinaryChoice,
    pending_worktree_name: Option<String>,
    success_message: Option<String>,
    error_message: Option<String>,
}

pub(crate) struct AttachScreen {
    flow: AttachFlow,
}

impl AttachScreen {
    pub(crate) fn new(app: &App<'_>, cwd: &Path) -> Result<Self> {
        Ok(Self {
            flow: AttachFlow::new(app, cwd)?,
        })
    }

    pub(crate) fn render(&self, frame: &mut ratatui::Frame<'_>) {
        self.flow.render(frame);
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent, app: &App<'_>) -> Result<Option<UiExit>> {
        match self.flow.on_key(key, app)? {
            FlowSignal::Continue => Ok(None),
            FlowSignal::Exit(exit) => Ok(Some(exit)),
        }
    }
}

impl AttachFlow {
    fn new(ops: &dyn AttachFlowOps, cwd: &Path) -> Result<Self> {
        let result = ops.list_worktrees(cwd)?;
        Ok(Self {
            cwd: cwd.to_path_buf(),
            step: Step::SelectWorktree,
            select: SelectStepState::new(result.rows),
            missing_choice: BinaryChoice::new(true),
            pending_worktree_name: None,
            success_message: None,
            error_message: None,
        })
    }

    fn on_key(&mut self, key: KeyEvent, ops: &dyn AttachFlowOps) -> Result<FlowSignal> {
        match self.step {
            Step::SelectWorktree => self.on_key_select(key, ops),
            Step::MissingSessionPrompt => self.on_key_missing_prompt(key, ops),
            Step::Success => Ok(self.on_key_success(key)),
            Step::Error => Ok(self.on_key_error(key)),
        }
    }

    fn on_key_select(&mut self, key: KeyEvent, ops: &dyn AttachFlowOps) -> Result<FlowSignal> {
        match self.select.on_key(key) {
            SelectSignal::Back => return Ok(FlowSignal::Exit(UiExit::BackAtRoot)),
            SelectSignal::Continue => return Ok(FlowSignal::Continue),
            SelectSignal::Confirm => {}
        }

        let Some(row) = self.select.selected_row().cloned() else {
            return Ok(FlowSignal::Continue);
        };

        match ops.attach_worktree(AttachRequest {
            cwd: self.cwd.clone(),
            worktree_name: row.name,
            create_if_missing: false,
        }) {
            Ok(result) => {
                self.success_message = Some(success_message_for(&result));
                self.error_message = None;
                self.step = Step::Success;
            }
            Err(error) => {
                if let Some(AttachError::MissingSession { worktree_name, .. }) =
                    error.downcast_ref::<AttachError>()
                {
                    self.pending_worktree_name = Some(worktree_name.clone());
                    self.missing_choice = BinaryChoice::new(true);
                    self.step = Step::MissingSessionPrompt;
                } else {
                    self.error_message = Some(format!("{error:#}"));
                    self.success_message = None;
                    self.step = Step::Error;
                }
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_missing_prompt(
        &mut self,
        key: KeyEvent,
        ops: &dyn AttachFlowOps,
    ) -> Result<FlowSignal> {
        match self.missing_choice.on_key(key) {
            BinaryChoiceEvent::Back => {
                self.select.set_filter_focused(false);
                self.step = Step::SelectWorktree;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::Continue => Ok(FlowSignal::Continue),
            BinaryChoiceEvent::ConfirmNo => {
                self.select.set_filter_focused(false);
                self.step = Step::SelectWorktree;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::ConfirmYes => {
                let Some(worktree_name) = self.pending_worktree_name.clone() else {
                    self.step = Step::SelectWorktree;
                    return Ok(FlowSignal::Continue);
                };

                match ops.attach_worktree(AttachRequest {
                    cwd: self.cwd.clone(),
                    worktree_name,
                    create_if_missing: true,
                }) {
                    Ok(result) => {
                        self.success_message = Some(success_message_for(&result));
                        self.error_message = None;
                        self.step = Step::Success;
                    }
                    Err(error) => {
                        self.error_message = Some(format!("{error:#}"));
                        self.success_message = None;
                        self.step = Step::Error;
                    }
                }

                Ok(FlowSignal::Continue)
            }
        }
    }

    fn on_key_success(&mut self, key: KeyEvent) -> FlowSignal {
        if keymap::is_quit(key) {
            return FlowSignal::Exit(UiExit::Completed);
        }

        if keymap::is_back(key) || keymap::is_confirm(key) {
            return FlowSignal::Exit(UiExit::BackAtRoot);
        }

        FlowSignal::Continue
    }

    fn on_key_error(&mut self, key: KeyEvent) -> FlowSignal {
        if keymap::is_back(key) || keymap::is_confirm(key) {
            self.select.set_filter_focused(false);
            self.step = Step::SelectWorktree;
            FlowSignal::Continue
        } else {
            FlowSignal::Continue
        }
    }

    fn render(&self, frame: &mut ratatui::Frame<'_>) {
        match self.step {
            Step::SelectWorktree => self.render_select(frame),
            Step::MissingSessionPrompt => self.render_missing_prompt(frame),
            Step::Success => self.render_success(frame),
            Step::Error => self.render_error(frame),
        }
    }

    fn render_select(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let key_text = if self.select.filter_focused() {
            compact_hint(
                area.width,
                "Type: filter    Backspace: delete    /: list focus    Esc: back",
                "Type filter    Backspace delete    /: list    Esc: back",
                "Type filter | Backspace | / list | Esc back",
            )
        } else {
            compact_hint(
                area.width,
                "/: filter focus    Enter: attach    Up/Down or j/k: move    Esc: back",
                "/: filter    Enter: attach    j/k: move    Esc: back",
                "/ filter | Enter attach | j/k move | Esc back",
            )
        };
        let footer_height = key_hint_height(area.width, key_text);
        let [filter_area, table_area, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(footer_height),
            ])
            .areas(area);

        let filter_focused = self.select.filter_focused();
        self.select.render_filter(
            frame,
            filter_area,
            focus_line("Filter"),
            Line::from("Filter (/ to focus)"),
        );

        let columns = [
            TableColumn {
                title: "Name",
                width: Constraint::Length(24),
            },
            TableColumn {
                title: "Created",
                width: Constraint::Length(28),
            },
            TableColumn {
                title: "Branch",
                width: Constraint::Length(20),
            },
            TableColumn {
                title: "Session",
                width: Constraint::Length(12),
            },
        ];

        self.select.render_table(
            frame,
            table_area,
            WorktreeTableRender {
                title: if filter_focused {
                    Line::from("Choose worktree to attach (/ to focus)")
                } else {
                    focus_line("Choose worktree to attach")
                },
                empty_message: "No matching worktrees.",
                columns: &columns,
                header_style: theme::table_header(Color::Yellow),
                highlight_style: theme::table_highlight(Color::Yellow),
            },
            |row| {
                let state = if row.session_running {
                    "running"
                } else {
                    "missing"
                };
                vec![
                    row.name.clone(),
                    row.created_at.clone(),
                    row.branch.clone(),
                    state.to_string(),
                ]
            },
        );

        let keys = key_hint_paragraph(key_text).block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_missing_prompt(&self, frame: &mut ratatui::Frame<'_>) {
        let worktree = self
            .pending_worktree_name
            .as_deref()
            .unwrap_or("UNCONFIRMED");
        let text = Text::from(vec![
            label_value_line("Worktree", worktree),
            highlighted_label_value_line("Current Selection", self.missing_choice.selected_label()),
        ]);
        render_modal(
            frame,
            ModalSpec {
                title: "No tmux session was found. Create one now?",
                title_style: Some(theme::focus_prompt()),
                body: text,
                key_hint: Some("Space: toggle    Enter: continue    Esc: back"),
                width_pct: 70,
                height_pct: 40,
            },
        );
    }

    fn render_success(&self, frame: &mut ratatui::Frame<'_>) {
        let summary = self
            .success_message
            .clone()
            .unwrap_or_else(|| "Attached to tmux session".to_string());
        let lines: Vec<Line<'static>> = summary
            .lines()
            .map(|line| Line::from(line.to_string()))
            .collect();
        render_modal(
            frame,
            ModalSpec {
                title: "Success",
                title_style: Some(theme::success_prompt()),
                body: Text::from(lines),
                key_hint: Some(result_footer(frame.area().width)),
                width_pct: 70,
                height_pct: 40,
            },
        );
    }

    fn render_error(&self, frame: &mut ratatui::Frame<'_>) {
        let text = self.error_message.as_deref().unwrap_or("Attach failed");
        render_error_modal(frame, text, 80, 40, "Enter/Esc: back");
    }
}

fn success_message_for(result: &AttachResult) -> String {
    format!(
        "Attached worktree: {}\ntmux session name: {}\nCreated tmux session now: {}",
        result.worktree_name,
        result.session_name,
        yes_no(result.created_session)
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use seshmux_app::{AttachError, AttachRequest, AttachResult, ListResult, WorktreeRow};

    use super::{AttachFlow, AttachFlowOps, FlowSignal, Step};

    struct FakeOps {
        rows: Vec<WorktreeRow>,
        attach_calls: RefCell<Vec<AttachRequest>>,
    }

    impl FakeOps {
        fn new() -> Self {
            Self {
                rows: vec![WorktreeRow {
                    name: "w1".to_string(),
                    path: PathBuf::from("/tmp/repo/worktrees/w1"),
                    created_at: "2026-02-25T10:00:00Z".to_string(),
                    branch: "w1".to_string(),
                    session_name: "repo/w1".to_string(),
                    session_running: false,
                }],
                attach_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl AttachFlowOps for FakeOps {
        fn list_worktrees(&self, _cwd: &Path) -> Result<ListResult> {
            Ok(ListResult {
                repo_root: PathBuf::from("/tmp/repo"),
                rows: self.rows.clone(),
            })
        }

        fn attach_worktree(&self, request: AttachRequest) -> Result<AttachResult> {
            self.attach_calls.borrow_mut().push(request.clone());
            if !request.create_if_missing {
                return Err(AttachError::MissingSession {
                    worktree_name: request.worktree_name,
                    session_name: "repo/w1".to_string(),
                }
                .into());
            }

            Ok(AttachResult {
                worktree_name: "w1".to_string(),
                worktree_path: PathBuf::from("/tmp/repo/worktrees/w1"),
                session_name: "repo/w1".to_string(),
                created_session: true,
            })
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn render_output(flow: &AttachFlow, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| flow.render(frame))
            .expect("render attach flow");
        format!("{}", terminal.backend())
    }

    #[test]
    fn esc_on_first_step_exits_flow() {
        let ops = FakeOps::new();
        let mut flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        let signal = flow.on_key(key(KeyCode::Esc), &ops).expect("signal");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
    }

    #[test]
    fn missing_session_prompt_can_create_and_finish_attach() {
        let ops = FakeOps::new();
        let mut flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("attempt attach");
        assert_eq!(flow.step, Step::MissingSessionPrompt);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm create");
        assert_eq!(flow.step, Step::Success);

        let calls = ops.attach_calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].create_if_missing);
        assert!(calls[1].create_if_missing);
    }

    #[test]
    fn select_step_enter_noop_when_filter_has_no_matches() {
        let ops = FakeOps::new();
        let mut flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Char('/')), &ops)
            .expect("focus filter");
        flow.on_key(key(KeyCode::Char('z')), &ops)
            .expect("filter char");
        flow.on_key(key(KeyCode::Char('z')), &ops)
            .expect("filter char");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("enter on empty results");

        assert_eq!(flow.step, Step::SelectWorktree);
        let calls = ops.attach_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn slash_focus_routes_j_to_filter_without_attach_call() {
        let ops = FakeOps::new();
        let mut flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Char('/')), &ops)
            .expect("focus filter");
        assert!(flow.select.filter_focused());

        flow.on_key(key(KeyCode::Char('j')), &ops)
            .expect("type filter");
        assert_eq!(flow.step, Step::SelectWorktree);

        let calls = ops.attach_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn missing_session_prompt_uses_space_toggle() {
        let ops = FakeOps::new();
        let mut flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("attempt attach");
        assert_eq!(flow.step, Step::MissingSessionPrompt);

        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("toggle to no");
        flow.on_key(key(KeyCode::Enter), &ops).expect("confirm no");
        assert_eq!(flow.step, Step::SelectWorktree);

        let calls = ops.attach_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].create_if_missing);
    }

    #[test]
    fn success_screen_keys_follow_home_and_quit_contract() {
        let ops = FakeOps::new();

        let mut enter_flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        enter_flow.step = Step::Success;
        let enter_signal = enter_flow.on_key(key(KeyCode::Enter), &ops).expect("enter");
        assert_eq!(enter_signal, FlowSignal::Exit(super::UiExit::BackAtRoot));

        let mut esc_flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        esc_flow.step = Step::Success;
        let esc_signal = esc_flow.on_key(key(KeyCode::Esc), &ops).expect("esc");
        assert_eq!(esc_signal, FlowSignal::Exit(super::UiExit::BackAtRoot));

        let mut quit_flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        quit_flow.step = Step::Success;
        let quit_signal = quit_flow
            .on_key(key(KeyCode::Char('q')), &ops)
            .expect("quit");
        assert_eq!(quit_signal, FlowSignal::Exit(super::UiExit::Completed));
    }

    #[test]
    fn success_screen_uses_tmux_labels_and_quit_footer() {
        let ops = FakeOps::new();
        let mut flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        flow.step = Step::Success;
        flow.success_message = Some(
            "Attached worktree: w1\ntmux session name: repo/w1\nCreated tmux session now: Yes"
                .to_string(),
        );

        let output = render_output(&flow, 110, 22);
        assert!(output.contains("tmux session name"));
        assert!(!output.contains("Session:"));
        assert!(output.contains("q: quit"));
    }

    #[test]
    fn error_modal_wraps_long_message() {
        let ops = FakeOps::new();
        let mut flow = AttachFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        flow.step = Step::Error;
        flow.error_message = Some(
            "attach failed because a very long explanatory error should wrap across lines and keep the trailing token visible TOKEN_WRAP_ATTACH".to_string(),
        );

        let output = render_output(&flow, 56, 18);
        assert!(output.contains("TOKEN_WRAP_ATTACH"));
    }
}
