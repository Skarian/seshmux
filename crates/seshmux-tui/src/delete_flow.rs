use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Color;
use ratatui::text::{Line, Text};
use seshmux_app::{App, DeleteError, DeleteRequest, DeleteResult, ListResult};

use crate::UiExit;
use crate::keymap;
use crate::theme;
use crate::ui::binary_choice::{BinaryChoice, BinaryChoiceEvent};
use crate::ui::modal::{
    ModalSpec, render_error_modal, render_modal, render_notice_modal, render_success_modal,
};
use crate::ui::select_step::{SelectSignal, SelectStepState};
use crate::ui::text::{
    compact_hint, focus_line, highlighted_label_value_line, key_hint_height, key_hint_paragraph,
    label_value_line, result_footer, yes_no,
};
use crate::ui::worktree_table::{TableColumn, WorktreeTableRender};

pub(crate) trait DeleteFlowOps {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult>;
    fn delete_worktree(&self, request: DeleteRequest) -> Result<DeleteResult>;
    fn force_delete_branch(&self, repo_root: &Path, branch_name: &str) -> Result<()>;
}

impl<'a> DeleteFlowOps for App<'a> {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult> {
        self.list(cwd)
    }

    fn delete_worktree(&self, request: DeleteRequest) -> Result<DeleteResult> {
        self.delete(request)
    }

    fn force_delete_branch(&self, repo_root: &Path, branch_name: &str) -> Result<()> {
        App::force_delete_branch(self, repo_root.to_path_buf(), branch_name.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    SelectWorktree,
    Options,
    Confirm,
    WorktreeForcePrompt,
    BranchForcePrompt,
    Notice,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSignal {
    Continue,
    Exit(UiExit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionField {
    KillSession,
    DeleteBranch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeleteOptions {
    kill_tmux_session: bool,
    delete_branch: bool,
}

#[derive(Debug)]
struct DeleteFlow {
    cwd: PathBuf,
    step: Step,
    select: SelectStepState,
    target_name: Option<String>,
    target_session_running: bool,
    options: DeleteOptions,
    option_selected: usize,
    confirm_choice: BinaryChoice,
    worktree_force_choice: BinaryChoice,
    branch_force_choice: BinaryChoice,
    pending_result: Option<DeleteResult>,
    success_message: Option<String>,
    error_message: Option<String>,
}

pub(crate) struct DeleteScreen {
    flow: DeleteFlow,
}

impl DeleteScreen {
    pub(crate) fn new(app: &App<'_>, cwd: &Path) -> Result<Self> {
        Ok(Self {
            flow: DeleteFlow::new(app, cwd)?,
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

impl DeleteFlow {
    fn new(ops: &dyn DeleteFlowOps, cwd: &Path) -> Result<Self> {
        let result = ops.list_worktrees(cwd)?;
        Ok(Self {
            cwd: cwd.to_path_buf(),
            step: Step::SelectWorktree,
            select: SelectStepState::new(result.rows),
            target_name: None,
            target_session_running: false,
            options: DeleteOptions {
                kill_tmux_session: false,
                delete_branch: false,
            },
            option_selected: 0,
            confirm_choice: BinaryChoice::new(false),
            worktree_force_choice: BinaryChoice::new(false),
            branch_force_choice: BinaryChoice::new(false),
            pending_result: None,
            success_message: None,
            error_message: None,
        })
    }

    fn on_key(&mut self, key: KeyEvent, ops: &dyn DeleteFlowOps) -> Result<FlowSignal> {
        match self.step {
            Step::SelectWorktree => Ok(self.on_key_select(key)),
            Step::Options => Ok(self.on_key_options(key)),
            Step::Confirm => self.on_key_confirm(key, ops),
            Step::WorktreeForcePrompt => self.on_key_worktree_force_prompt(key, ops),
            Step::BranchForcePrompt => self.on_key_branch_force_prompt(key, ops),
            Step::Notice => Ok(self.on_key_notice(key)),
            Step::Success => Ok(self.on_key_success(key)),
            Step::Error => Ok(self.on_key_error(key)),
        }
    }

    fn on_key_select(&mut self, key: KeyEvent) -> FlowSignal {
        match self.select.on_key(key) {
            SelectSignal::Back => return FlowSignal::Exit(UiExit::BackAtRoot),
            SelectSignal::Continue => return FlowSignal::Continue,
            SelectSignal::Confirm => {}
        }

        let Some(row) = self.select.selected_row() else {
            return FlowSignal::Continue;
        };

        self.target_name = Some(row.name.clone());
        self.target_session_running = row.session_running;
        self.options = DeleteOptions {
            kill_tmux_session: false,
            delete_branch: false,
        };
        self.option_selected = 0;
        self.confirm_choice = BinaryChoice::new(false);
        self.worktree_force_choice = BinaryChoice::new(false);
        self.branch_force_choice = BinaryChoice::new(false);
        self.pending_result = None;
        self.success_message = None;
        self.error_message = None;
        self.select.set_filter_focused(false);
        self.step = Step::Options;

        FlowSignal::Continue
    }

    fn on_key_options(&mut self, key: KeyEvent) -> FlowSignal {
        if keymap::is_back(key) {
            self.select.set_filter_focused(false);
            self.step = Step::SelectWorktree;
            return FlowSignal::Continue;
        }

        if keymap::is_up(key) {
            self.option_selected = self.option_selected.saturating_sub(1);
            return FlowSignal::Continue;
        }

        if keymap::is_down(key) {
            if self.option_selected + 1 < self.option_fields().len() {
                self.option_selected += 1;
            }
            return FlowSignal::Continue;
        }

        if keymap::is_toggle(key) {
            self.toggle_current_option();
            return FlowSignal::Continue;
        }

        if keymap::is_confirm(key) {
            self.confirm_choice = BinaryChoice::new(false);
            self.step = Step::Confirm;
            return FlowSignal::Continue;
        }

        FlowSignal::Continue
    }

    fn on_key_confirm(&mut self, key: KeyEvent, ops: &dyn DeleteFlowOps) -> Result<FlowSignal> {
        match self.confirm_choice.on_key(key) {
            BinaryChoiceEvent::Back => {
                self.step = Step::Options;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::Continue => Ok(FlowSignal::Continue),
            BinaryChoiceEvent::ConfirmNo => {
                self.success_message = Some("Delete canceled. No changes were made.".to_string());
                self.step = Step::Notice;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::ConfirmYes => {
                self.execute_delete(ops, false)?;
                Ok(FlowSignal::Continue)
            }
        }
    }

    fn on_key_worktree_force_prompt(
        &mut self,
        key: KeyEvent,
        ops: &dyn DeleteFlowOps,
    ) -> Result<FlowSignal> {
        match self.worktree_force_choice.on_key(key) {
            BinaryChoiceEvent::Back => {
                self.step = Step::Confirm;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::Continue => Ok(FlowSignal::Continue),
            BinaryChoiceEvent::ConfirmNo => {
                self.success_message = Some("Delete canceled. No changes were made.".to_string());
                self.step = Step::Notice;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::ConfirmYes => {
                self.execute_delete(ops, true)?;
                Ok(FlowSignal::Continue)
            }
        }
    }

    fn on_key_branch_force_prompt(
        &mut self,
        key: KeyEvent,
        ops: &dyn DeleteFlowOps,
    ) -> Result<FlowSignal> {
        match self.branch_force_choice.on_key(key) {
            BinaryChoiceEvent::Back | BinaryChoiceEvent::ConfirmNo => {
                let Some(result) = self.pending_result.take() else {
                    self.step = Step::SelectWorktree;
                    return Ok(FlowSignal::Continue);
                };

                self.success_message = Some(self.branch_kept_message(&result));
                self.step = Step::Notice;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::Continue => Ok(FlowSignal::Continue),
            BinaryChoiceEvent::ConfirmYes => {
                let Some(mut result) = self.pending_result.take() else {
                    self.step = Step::SelectWorktree;
                    return Ok(FlowSignal::Continue);
                };

                match ops.force_delete_branch(&result.repo_root, &result.branch_name) {
                    Ok(()) => {
                        result.branch_deleted = true;
                        result.branch_delete_error = None;
                        self.success_message = Some(self.success_summary(&result));
                        self.step = Step::Success;
                    }
                    Err(error) => {
                        self.success_message = Some(format!(
                            "Deleted worktree '{}'. tmux session '{}'. Branch kept (force delete failed: {error:#}).",
                            result.worktree_name, result.session_name
                        ));
                        self.step = Step::Notice;
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

    fn on_key_notice(&mut self, key: KeyEvent) -> FlowSignal {
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

    fn execute_delete(&mut self, ops: &dyn DeleteFlowOps, force_worktree: bool) -> Result<()> {
        let Some(worktree_name) = self.target_name.clone() else {
            self.error_message = Some("no worktree selected".to_string());
            self.step = Step::Error;
            return Ok(());
        };

        match ops.delete_worktree(DeleteRequest {
            cwd: self.cwd.clone(),
            worktree_name,
            kill_tmux_session: self.options.kill_tmux_session,
            delete_branch: self.options.delete_branch,
            force_worktree,
        }) {
            Ok(result) => {
                self.select.remove_by_name(&result.worktree_name);
                self.error_message = None;

                if self.options.delete_branch && result.branch_delete_error.is_some() {
                    self.pending_result = Some(result);
                    self.branch_force_choice = BinaryChoice::new(false);
                    self.step = Step::BranchForcePrompt;
                } else {
                    self.success_message = Some(self.success_summary(&result));
                    self.step = Step::Success;
                }
            }
            Err(error) => {
                if !force_worktree {
                    if let Some(DeleteError::WorktreeDeleteFailed { message }) =
                        error.downcast_ref::<DeleteError>()
                    {
                        self.error_message = Some(message.clone());
                        self.worktree_force_choice = BinaryChoice::new(false);
                        self.step = Step::WorktreeForcePrompt;
                        return Ok(());
                    }
                }

                self.error_message = Some(format!("{error:#}"));
                self.step = Step::Error;
            }
        }

        Ok(())
    }

    fn success_summary(&self, result: &DeleteResult) -> String {
        let branch_summary = if self.options.delete_branch {
            if result.branch_deleted {
                "deleted".to_string()
            } else if let Some(error) = &result.branch_delete_error {
                format!("kept (delete failed: {error})")
            } else {
                "kept".to_string()
            }
        } else {
            "kept (not requested)".to_string()
        };

        format!(
            "Deleted worktree '{}'. tmux session '{}'. Branch status: {}.",
            result.worktree_name, result.session_name, branch_summary
        )
    }

    fn branch_kept_message(&self, result: &DeleteResult) -> String {
        let failure = result
            .branch_delete_error
            .as_deref()
            .unwrap_or("unknown error");
        format!(
            "Deleted worktree '{}'. tmux session '{}'. Branch kept (safe delete failed: {}).",
            result.worktree_name, result.session_name, failure
        )
    }

    fn option_fields(&self) -> [OptionField; 2] {
        [OptionField::KillSession, OptionField::DeleteBranch]
    }

    fn selected_option_field(&self) -> OptionField {
        self.option_fields()[self.option_selected]
    }

    fn toggle_current_option(&mut self) {
        match self.selected_option_field() {
            OptionField::KillSession => {
                if self.target_session_running {
                    self.options.kill_tmux_session = !self.options.kill_tmux_session;
                }
            }
            OptionField::DeleteBranch => {
                self.options.delete_branch = !self.options.delete_branch;
            }
        }
    }

    fn render(&self, frame: &mut ratatui::Frame<'_>) {
        match self.step {
            Step::SelectWorktree => self.render_select(frame),
            Step::Options => self.render_options(frame),
            Step::Confirm => self.render_confirm(frame),
            Step::WorktreeForcePrompt => self.render_worktree_force_prompt(frame),
            Step::BranchForcePrompt => self.render_branch_force_prompt(frame),
            Step::Notice => self.render_notice(frame),
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
                "/: filter focus    Enter: select    Up/Down or j/k: move    Esc: back",
                "/: filter    Enter: select    j/k: move    Esc: back",
                "/ filter | Enter select | j/k move | Esc back",
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
                width: Constraint::Length(14),
            },
        ];

        self.select.render_table(
            frame,
            table_area,
            WorktreeTableRender {
                title: if filter_focused {
                    Line::from("Choose worktree to delete (/ to focus)")
                } else {
                    focus_line("Choose worktree to delete")
                },
                empty_message: "No matching worktrees.",
                columns: &columns,
                header_style: theme::table_header(Color::Red),
                highlight_style: theme::table_highlight(Color::Red),
            },
            |row| {
                let state = if row.session_running {
                    "running"
                } else {
                    "not running"
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

    fn render_options(&self, frame: &mut ratatui::Frame<'_>) {
        let target = self.target_name.as_deref().unwrap_or("UNCONFIRMED");

        let rows = [
            (
                "Kill tmux session",
                if self.target_session_running {
                    if self.options.kill_tmux_session {
                        "Yes"
                    } else {
                        "No"
                    }
                } else {
                    "N/A (not running)"
                },
            ),
            (
                "Delete branch",
                if self.options.delete_branch {
                    "Yes"
                } else {
                    "No"
                },
            ),
        ];

        let mut lines = vec![label_value_line("Worktree", target), Line::from("")];
        for (index, (label, value)) in rows.iter().enumerate() {
            let marker = if self.option_selected == index {
                ">>"
            } else {
                "  "
            };
            let row_line = format!("{marker} {label}: {value}");
            if self.option_selected == index {
                lines.push(focus_line(row_line));
            } else {
                lines.push(Line::from(row_line));
            }
        }

        render_modal(
            frame,
            ModalSpec {
                title: "Choose delete options for this worktree",
                title_style: Some(theme::focus_prompt()),
                body: Text::from(lines),
                key_hint: Some(
                    "Space: toggle selected option    Up/Down or j/k: move option    Enter: continue    Esc: back",
                ),
                width_pct: 78,
                height_pct: 56,
            },
        );
    }

    fn render_confirm(&self, frame: &mut ratatui::Frame<'_>) {
        let target = self.target_name.as_deref().unwrap_or("UNCONFIRMED");
        let text = Text::from(vec![
            label_value_line("Worktree", target),
            label_value_line("Kill tmux session", yes_no(self.options.kill_tmux_session)),
            label_value_line("Delete branch", yes_no(self.options.delete_branch)),
            Line::from(""),
            highlighted_label_value_line("Current Selection", self.confirm_choice.selected_label()),
        ]);

        render_modal(
            frame,
            ModalSpec {
                title: "Confirm worktree deletion",
                title_style: Some(theme::focus_prompt()),
                body: text,
                key_hint: Some("Space: toggle    Enter: confirm    Esc: back"),
                width_pct: 78,
                height_pct: 48,
            },
        );
    }

    fn render_worktree_force_prompt(&self, frame: &mut ratatui::Frame<'_>) {
        let error = self.error_message.as_deref().unwrap_or("unknown error");
        let text = Text::from(vec![
            Line::from("Safe delete error:"),
            Line::from(error.to_string()),
            Line::from(""),
            highlighted_label_value_line(
                "Current Selection",
                self.worktree_force_choice.selected_label(),
            ),
        ]);
        render_modal(
            frame,
            ModalSpec {
                title: "Safe deletion failed. Force delete this worktree?",
                title_style: Some(theme::focus_prompt()),
                body: text,
                key_hint: Some("Space: toggle    Enter: continue    Esc: back"),
                width_pct: 85,
                height_pct: 55,
            },
        );
    }

    fn render_branch_force_prompt(&self, frame: &mut ratatui::Frame<'_>) {
        let result = self.pending_result.as_ref();
        let branch_name = result
            .map(|value| value.branch_name.as_str())
            .unwrap_or("UNCONFIRMED");
        let error = result
            .and_then(|value| value.branch_delete_error.as_deref())
            .unwrap_or("unknown error");

        let text = Text::from(vec![
            label_value_line("Branch", branch_name),
            Line::from("Safe delete error:"),
            Line::from(error.to_string()),
            Line::from(""),
            highlighted_label_value_line(
                "Current Selection",
                self.branch_force_choice.selected_label(),
            ),
        ]);
        render_modal(
            frame,
            ModalSpec {
                title: "Safe branch delete failed. Force delete the branch?",
                title_style: Some(theme::focus_prompt()),
                body: text,
                key_hint: Some("Space: toggle    Enter: continue    Esc: keep branch"),
                width_pct: 85,
                height_pct: 55,
            },
        );
    }

    fn render_success(&self, frame: &mut ratatui::Frame<'_>) {
        let text = self
            .success_message
            .as_deref()
            .unwrap_or("Delete completed");
        render_success_modal(frame, text, 75, 35, result_footer(frame.area().width));
    }

    fn render_notice(&self, frame: &mut ratatui::Frame<'_>) {
        let text = self
            .success_message
            .as_deref()
            .unwrap_or("No changes were made");
        render_notice_modal(
            frame,
            "Delete notice",
            text,
            70,
            30,
            result_footer(frame.area().width),
        );
    }

    fn render_error(&self, frame: &mut ratatui::Frame<'_>) {
        let text = self.error_message.as_deref().unwrap_or("Delete failed");
        render_error_modal(frame, text, 80, 40, "Enter/Esc: back");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use anyhow::{Result, anyhow};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use seshmux_app::{DeleteError, DeleteRequest, DeleteResult, ListResult, WorktreeRow};

    use super::{DeleteFlow, DeleteFlowOps, FlowSignal, Step};

    struct FakeOps {
        rows: Vec<WorktreeRow>,
        delete_calls: RefCell<Vec<DeleteRequest>>,
        force_branch_calls: RefCell<Vec<(PathBuf, String)>>,
        worktree_delete_fail_once: RefCell<bool>,
        branch_safe_fails: bool,
        branch_force_fails: bool,
    }

    impl FakeOps {
        fn new(
            session_running: bool,
            worktree_delete_fail_once: bool,
            branch_safe_fails: bool,
            branch_force_fails: bool,
        ) -> Self {
            Self {
                rows: vec![WorktreeRow {
                    name: "w1".to_string(),
                    path: PathBuf::from("/tmp/repo/worktrees/w1"),
                    created_at: "2026-02-25T10:00:00Z".to_string(),
                    branch: "w1".to_string(),
                    session_name: "repo/w1".to_string(),
                    session_running,
                }],
                delete_calls: RefCell::new(Vec::new()),
                force_branch_calls: RefCell::new(Vec::new()),
                worktree_delete_fail_once: RefCell::new(worktree_delete_fail_once),
                branch_safe_fails,
                branch_force_fails,
            }
        }
    }

    impl DeleteFlowOps for FakeOps {
        fn list_worktrees(&self, _cwd: &Path) -> Result<ListResult> {
            Ok(ListResult {
                repo_root: PathBuf::from("/tmp/repo"),
                rows: self.rows.clone(),
            })
        }

        fn delete_worktree(&self, request: DeleteRequest) -> Result<DeleteResult> {
            self.delete_calls.borrow_mut().push(request.clone());

            if !request.force_worktree && *self.worktree_delete_fail_once.borrow() {
                *self.worktree_delete_fail_once.borrow_mut() = false;
                return Err(DeleteError::WorktreeDeleteFailed {
                    message: "git worktree remove failed: worktree contains modified files"
                        .to_string(),
                }
                .into());
            }

            let branch_delete_error = if request.delete_branch && self.branch_safe_fails {
                Some("branch safe delete failed: not fully merged".to_string())
            } else {
                None
            };
            let branch_deleted = request.delete_branch && branch_delete_error.is_none();

            Ok(DeleteResult {
                worktree_name: request.worktree_name,
                repo_root: PathBuf::from("/tmp/repo"),
                worktree_path: PathBuf::from("/tmp/repo/worktrees/w1"),
                session_name: "repo/w1".to_string(),
                branch_name: "w1".to_string(),
                branch_deleted,
                branch_delete_error,
            })
        }

        fn force_delete_branch(&self, repo_root: &Path, branch_name: &str) -> Result<()> {
            self.force_branch_calls
                .borrow_mut()
                .push((repo_root.to_path_buf(), branch_name.to_string()));
            if self.branch_force_fails {
                return Err(anyhow!(
                    "force delete failed: branch is checked out in another worktree"
                ));
            }
            Ok(())
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn render_output(flow: &DeleteFlow, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| flow.render(frame))
            .expect("render delete flow");
        format!("{}", terminal.backend())
    }

    fn apply_keys(flow: &mut DeleteFlow, ops: &FakeOps, keys: &[KeyCode]) {
        for code in keys {
            let _ = flow.on_key(key(*code), ops).expect("apply key");
        }
    }

    #[test]
    fn esc_on_first_step_exits_flow() {
        let ops = FakeOps::new(false, false, false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        let signal = flow.on_key(key(KeyCode::Esc), &ops).expect("signal");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
    }

    #[test]
    fn options_are_collected_and_passed_to_delete_request() {
        let ops = FakeOps::new(true, false, false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );

        assert_eq!(flow.step, Step::Success);

        let calls = ops.delete_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].kill_tmux_session);
        assert!(calls[0].delete_branch);
        assert!(!calls[0].force_worktree);
    }

    #[test]
    fn worktree_delete_failure_prompts_force_and_retries_with_force() {
        let ops = FakeOps::new(false, true, false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );

        assert_eq!(flow.step, Step::Success);
        let calls = ops.delete_calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].force_worktree);
        assert!(calls[1].force_worktree);
    }

    #[test]
    fn branch_delete_failure_prompts_force_option() {
        let ops = FakeOps::new(false, false, true, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );

        assert_eq!(flow.step, Step::BranchForcePrompt);
    }

    #[test]
    fn branch_force_no_keeps_branch_and_shows_notice() {
        let ops = FakeOps::new(false, false, true, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Enter,
            ],
        );

        assert_eq!(flow.step, Step::Notice);
        assert!(
            flow.success_message
                .as_deref()
                .unwrap_or("")
                .contains("Branch kept")
        );
    }

    #[test]
    fn branch_force_esc_keeps_branch_and_shows_notice() {
        let ops = FakeOps::new(false, false, true, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );
        assert_eq!(flow.step, Step::BranchForcePrompt);

        flow.on_key(key(KeyCode::Esc), &ops)
            .expect("esc keeps branch");
        assert_eq!(flow.step, Step::Notice);
        assert!(
            flow.success_message
                .as_deref()
                .unwrap_or("")
                .contains("Branch kept")
        );
        assert!(ops.force_branch_calls.borrow().is_empty());
    }

    #[test]
    fn branch_force_yes_calls_force_delete_and_succeeds() {
        let ops = FakeOps::new(false, false, true, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );

        assert_eq!(flow.step, Step::Success);
        let calls = ops.force_branch_calls.borrow();
        assert_eq!(
            calls.as_slice(),
            [(PathBuf::from("/tmp/repo"), "w1".to_string())]
        );
    }

    #[test]
    fn delete_cancel_shows_notice_and_makes_no_delete_call() {
        let ops = FakeOps::new(false, false, false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops).expect("select");
        assert_eq!(flow.step, Step::Options);

        flow.on_key(key(KeyCode::Enter), &ops).expect("go confirm");
        assert_eq!(flow.step, Step::Confirm);

        flow.on_key(key(KeyCode::Enter), &ops).expect("confirm no");
        assert_eq!(flow.step, Step::Notice);

        let signal = flow
            .on_key(key(KeyCode::Enter), &ops)
            .expect("close notice");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));

        let calls = ops.delete_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn select_step_enter_noop_when_filter_has_no_matches() {
        let ops = FakeOps::new(false, false, false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Char('/')), &ops)
            .expect("focus filter");
        apply_keys(
            &mut flow,
            &ops,
            &[KeyCode::Char('z'), KeyCode::Char('z'), KeyCode::Enter],
        );

        assert_eq!(flow.step, Step::SelectWorktree);
        assert!(flow.target_name.is_none());
        let calls = ops.delete_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn slash_focus_routes_j_to_filter_without_advancing_step() {
        let ops = FakeOps::new(false, false, false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Char('/')), &ops)
            .expect("focus filter");
        assert!(flow.select.filter_focused());

        flow.on_key(key(KeyCode::Char('j')), &ops)
            .expect("type filter");
        assert_eq!(flow.step, Step::SelectWorktree);
        assert!(flow.target_name.is_none());

        let calls = ops.delete_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn success_screen_keys_follow_home_and_quit_contract() {
        let ops = FakeOps::new(false, false, false, false);
        let mut enter_flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        apply_keys(
            &mut enter_flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );
        assert_eq!(enter_flow.step, Step::Success);

        let enter_signal = enter_flow.on_key(key(KeyCode::Enter), &ops).expect("enter");
        assert_eq!(enter_signal, FlowSignal::Exit(super::UiExit::BackAtRoot));

        let mut esc_flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        apply_keys(
            &mut esc_flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );
        assert_eq!(esc_flow.step, Step::Success);
        let esc_signal = esc_flow.on_key(key(KeyCode::Esc), &ops).expect("esc");
        assert_eq!(esc_signal, FlowSignal::Exit(super::UiExit::BackAtRoot));

        let mut quit_flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        apply_keys(
            &mut quit_flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Enter,
                KeyCode::Char(' '),
                KeyCode::Enter,
            ],
        );
        assert_eq!(quit_flow.step, Step::Success);
        let quit_signal = quit_flow
            .on_key(key(KeyCode::Char('q')), &ops)
            .expect("quit");
        assert_eq!(quit_signal, FlowSignal::Exit(super::UiExit::Completed));
    }

    #[test]
    fn notice_screen_keys_follow_home_and_quit_contract() {
        let ops = FakeOps::new(false, false, false, false);
        let mut enter_flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        enter_flow.step = Step::Notice;
        let enter_signal = enter_flow.on_key(key(KeyCode::Enter), &ops).expect("enter");
        assert_eq!(enter_signal, FlowSignal::Exit(super::UiExit::BackAtRoot));

        let mut esc_flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        esc_flow.step = Step::Notice;
        let esc_signal = esc_flow.on_key(key(KeyCode::Esc), &ops).expect("esc");
        assert_eq!(esc_signal, FlowSignal::Exit(super::UiExit::BackAtRoot));

        let mut quit_flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        quit_flow.step = Step::Notice;
        let quit_signal = quit_flow
            .on_key(key(KeyCode::Char('q')), &ops)
            .expect("quit");
        assert_eq!(quit_signal, FlowSignal::Exit(super::UiExit::Completed));
    }

    #[test]
    fn notice_modal_wraps_long_message_and_shows_quit_footer() {
        let ops = FakeOps::new(false, false, false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        flow.step = Step::Notice;
        flow.success_message = Some(
            "Deleted worktree but branch remained due to a very long explanatory note that must wrap in the notice modal and keep TOKEN_WRAP_DELETE visible".to_string(),
        );

        let output = render_output(&flow, 110, 20);
        assert!(output.contains("TOKEN_WRAP_DELETE"));
        assert!(output.contains("q: quit"));
    }
}
