use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState,
};
use seshmux_app::{App, DeleteRequest, DeleteResult, ListResult, WorktreeRow};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use crate::{UiExit, centered_rect};

pub(crate) trait DeleteFlowOps {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult>;
    fn delete_worktree(&self, request: DeleteRequest) -> Result<DeleteResult>;
}

impl<'a> DeleteFlowOps for App<'a> {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult> {
        self.list(cwd)
    }

    fn delete_worktree(&self, request: DeleteRequest) -> Result<DeleteResult> {
        self.delete(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    SelectWorktree,
    Options,
    Confirm,
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
    rows: Vec<WorktreeRow>,
    filtered: Vec<usize>,
    selected: usize,
    query: Input,
    target: Option<WorktreeRow>,
    options: DeleteOptions,
    option_selected: usize,
    confirm_execute: bool,
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
        let mut flow = Self {
            cwd: cwd.to_path_buf(),
            step: Step::SelectWorktree,
            rows: result.rows,
            filtered: Vec::new(),
            selected: 0,
            query: Input::default(),
            target: None,
            options: DeleteOptions {
                kill_tmux_session: false,
                delete_branch: false,
            },
            option_selected: 0,
            confirm_execute: false,
            success_message: None,
            error_message: None,
        };
        flow.refresh_filtered();
        Ok(flow)
    }

    fn on_key(&mut self, key: KeyEvent, ops: &dyn DeleteFlowOps) -> Result<FlowSignal> {
        match self.step {
            Step::SelectWorktree => Ok(self.on_key_select(key)),
            Step::Options => Ok(self.on_key_options(key)),
            Step::Confirm => self.on_key_confirm(key, ops),
            Step::Notice => Ok(self.on_key_notice(key)),
            Step::Success => Ok(self.on_key_success(key)),
            Step::Error => Ok(self.on_key_error(key)),
        }
    }

    fn on_key_select(&mut self, key: KeyEvent) -> FlowSignal {
        match key.code {
            KeyCode::Esc => FlowSignal::Exit(UiExit::BackAtRoot),
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                FlowSignal::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.filtered.len() {
                    self.selected += 1;
                }
                FlowSignal::Continue
            }
            KeyCode::Enter => {
                let Some(target) = self.selected_row().cloned() else {
                    return FlowSignal::Continue;
                };
                self.target = Some(target);
                self.options = DeleteOptions {
                    kill_tmux_session: false,
                    delete_branch: false,
                };
                self.option_selected = 0;
                self.step = Step::Options;
                FlowSignal::Continue
            }
            _ => {
                if self.query.handle_event(&Event::Key(key)).is_some() {
                    self.refresh_filtered();
                }
                FlowSignal::Continue
            }
        }
    }

    fn on_key_options(&mut self, key: KeyEvent) -> FlowSignal {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::SelectWorktree;
                FlowSignal::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.option_selected = self.option_selected.saturating_sub(1);
                FlowSignal::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.option_selected + 1 < self.option_fields().len() {
                    self.option_selected += 1;
                }
                FlowSignal::Continue
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('n') => {
                self.set_current_option(false);
                FlowSignal::Continue
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('y') => {
                self.set_current_option(true);
                FlowSignal::Continue
            }
            KeyCode::Char(' ') => {
                self.toggle_current_option();
                FlowSignal::Continue
            }
            KeyCode::Enter => {
                self.confirm_execute = false;
                self.step = Step::Confirm;
                FlowSignal::Continue
            }
            _ => FlowSignal::Continue,
        }
    }

    fn on_key_confirm(&mut self, key: KeyEvent, ops: &dyn DeleteFlowOps) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::Options;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('n') => {
                self.confirm_execute = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('y') => {
                self.confirm_execute = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                if !self.confirm_execute {
                    self.success_message =
                        Some("Delete canceled. No changes were made.".to_string());
                    self.step = Step::Notice;
                    return Ok(FlowSignal::Continue);
                }

                self.execute_delete(ops)?;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_success(&mut self, key: KeyEvent) -> FlowSignal {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => FlowSignal::Exit(UiExit::Completed),
            _ => FlowSignal::Continue,
        }
    }

    fn on_key_notice(&mut self, key: KeyEvent) -> FlowSignal {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.step = Step::SelectWorktree;
                FlowSignal::Continue
            }
            _ => FlowSignal::Continue,
        }
    }

    fn on_key_error(&mut self, key: KeyEvent) -> FlowSignal {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.step = Step::SelectWorktree;
                FlowSignal::Continue
            }
            _ => FlowSignal::Continue,
        }
    }

    fn execute_delete(&mut self, ops: &dyn DeleteFlowOps) -> Result<()> {
        let Some(target) = self.target.clone() else {
            self.error_message = Some("no worktree selected".to_string());
            self.step = Step::Error;
            return Ok(());
        };

        match ops.delete_worktree(DeleteRequest {
            cwd: self.cwd.clone(),
            worktree_name: target.name.clone(),
            kill_tmux_session: self.options.kill_tmux_session,
            delete_branch: self.options.delete_branch,
        }) {
            Ok(result) => {
                let branch_summary = if self.options.delete_branch {
                    if result.branch_deleted {
                        "deleted".to_string()
                    } else {
                        "kept (not fully merged)".to_string()
                    }
                } else {
                    "kept (not requested)".to_string()
                };

                self.success_message = Some(format!(
                    "Deleted worktree '{}'. Session '{}'. Branch: {}.",
                    result.worktree_name, result.session_name, branch_summary
                ));
                self.error_message = None;
                self.rows.retain(|row| row.name != result.worktree_name);
                self.refresh_filtered();
                self.step = Step::Success;
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                self.step = Step::Error;
            }
        }

        Ok(())
    }

    fn option_fields(&self) -> [OptionField; 2] {
        [OptionField::KillSession, OptionField::DeleteBranch]
    }

    fn selected_option_field(&self) -> OptionField {
        self.option_fields()[self.option_selected]
    }

    fn set_current_option(&mut self, value: bool) {
        match self.selected_option_field() {
            OptionField::KillSession => {
                if self
                    .target
                    .as_ref()
                    .map(|value| value.session_running)
                    .unwrap_or(false)
                {
                    self.options.kill_tmux_session = value;
                }
            }
            OptionField::DeleteBranch => {
                self.options.delete_branch = value;
            }
        }
    }

    fn toggle_current_option(&mut self) {
        let next = match self.selected_option_field() {
            OptionField::KillSession => !self.options.kill_tmux_session,
            OptionField::DeleteBranch => !self.options.delete_branch,
        };
        self.set_current_option(next);
    }

    fn refresh_filtered(&mut self) {
        let query = self.query.value().trim().to_lowercase();
        self.filtered = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                if query.is_empty() {
                    return true;
                }

                row.name.to_lowercase().contains(&query)
                    || row.path.to_string_lossy().to_lowercase().contains(&query)
                    || row.branch.to_lowercase().contains(&query)
                    || row.created_at.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();

        if self.filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
    }

    fn selected_row(&self) -> Option<&WorktreeRow> {
        let index = *self.filtered.get(self.selected)?;
        self.rows.get(index)
    }

    fn render(&self, frame: &mut ratatui::Frame<'_>) {
        match self.step {
            Step::SelectWorktree => self.render_select(frame),
            Step::Options => self.render_options(frame),
            Step::Confirm => self.render_confirm(frame),
            Step::Notice => self.render_notice(frame),
            Step::Success => self.render_success(frame),
            Step::Error => self.render_error(frame),
        }
    }

    fn render_select(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [filter_area, table_area, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(3),
            ])
            .areas(area);

        self.render_filter(frame, filter_area);

        if self.filtered.is_empty() {
            let empty = Paragraph::new("No matching worktrees.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Delete: select worktree"),
            );
            frame.render_widget(empty, table_area);
        } else {
            let header = Row::new(["Name", "Created", "Branch", "Session"])
                .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
            let rows = self
                .filtered
                .iter()
                .filter_map(|index| self.rows.get(*index))
                .map(|row| {
                    let state = if row.session_running {
                        "running"
                    } else {
                        "not running"
                    };
                    Row::new(vec![
                        row.name.clone(),
                        row.created_at.clone(),
                        row.branch.clone(),
                        state.to_string(),
                    ])
                });

            let table = Table::new(
                rows,
                [
                    Constraint::Length(24),
                    Constraint::Length(28),
                    Constraint::Length(20),
                    Constraint::Length(14),
                ],
            )
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Delete: select worktree"),
            )
            .row_highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

            let mut state = TableState::new();
            state.select(Some(self.selected));
            frame.render_stateful_widget(table, table_area, &mut state);

            let viewport = table_area.height.saturating_sub(3) as usize;
            let mut scrollbar_state = ScrollbarState::new(self.filtered.len())
                .position(self.selected)
                .viewport_content_length(viewport);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                table_area.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }

        let keys =
            Paragraph::new("Type: filter    Enter: select    Up/Down or j/k: move    Esc: back")
                .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_filter(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let width = area.width.saturating_sub(2) as usize;
        let scroll = self.query.visual_scroll(width);
        let paragraph = Paragraph::new(self.query.value())
            .scroll((0, scroll as u16))
            .block(Block::default().borders(Borders::ALL).title("Filter"));
        frame.render_widget(paragraph, area);

        if width == 0 {
            return;
        }
        let visual = self.query.visual_cursor();
        let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
        frame.set_cursor_position((area.x + 1 + relative as u16, area.y + 1));
    }

    fn render_options(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(78, 56, frame.area());
        frame.render_widget(Clear, area);

        let target = self
            .target
            .as_ref()
            .map(|value| value.name.as_str())
            .unwrap_or("UNCONFIRMED");
        let session_running = self
            .target
            .as_ref()
            .map(|value| value.session_running)
            .unwrap_or(false);

        let rows = [
            (
                "Kill tmux session",
                if session_running {
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

        let mut body = String::new();
        body.push_str(&format!("Delete options for '{target}':\n\n"));
        for (index, (label, value)) in rows.iter().enumerate() {
            let marker = if self.option_selected == index {
                ">>"
            } else {
                "  "
            };
            body.push_str(&format!("{marker} {label}: {value}\n"));
        }
        body.push_str("\nLeft/Right or h/l: set No/Yes    Space: toggle\n");
        body.push_str("Up/Down or j/k: move option    Enter: continue    Esc: back");

        let widget =
            Paragraph::new(body).block(Block::default().borders(Borders::ALL).title("Delete"));
        frame.render_widget(widget, area);
    }

    fn render_confirm(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(78, 48, frame.area());
        frame.render_widget(Clear, area);

        let target = self
            .target
            .as_ref()
            .map(|value| value.name.as_str())
            .unwrap_or("UNCONFIRMED");
        let session_text = if self.options.kill_tmux_session {
            "Yes"
        } else {
            "No"
        };
        let delete_branch_text = if self.options.delete_branch {
            "Yes"
        } else {
            "No"
        };
        let selection = if self.confirm_execute { "Yes" } else { "No" };

        let text = format!(
            "Confirm delete for '{target}':\n\nKill tmux session: {session_text}\nDelete branch: {delete_branch_text}\n\nExecute now? {selection}\n\nLeft/Right or h/l: choose No/Yes    y/n: choose    Enter: confirm    Esc: back"
        );

        let widget =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Confirm"));
        frame.render_widget(widget, area);
    }

    fn render_success(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(75, 35, frame.area());
        frame.render_widget(Clear, area);

        let text = self
            .success_message
            .as_deref()
            .unwrap_or("Delete completed.");
        let widget = Paragraph::new(format!("{text}\n\nEnter/Esc to return."))
            .block(Block::default().borders(Borders::ALL).title("Success"));
        frame.render_widget(widget, area);
    }

    fn render_notice(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(70, 30, frame.area());
        frame.render_widget(Clear, area);

        let text = self
            .success_message
            .as_deref()
            .unwrap_or("No changes were made.");
        let widget = Paragraph::new(format!("{text}\n\nEnter/Esc to continue."))
            .block(Block::default().borders(Borders::ALL).title("Notice"));
        frame.render_widget(widget, area);
    }

    fn render_error(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(80, 40, frame.area());
        frame.render_widget(Clear, area);

        let text = self.error_message.as_deref().unwrap_or("Delete failed.");
        let widget = Paragraph::new(format!("{text}\n\nEnter/Esc to return."))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(widget, area);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use seshmux_app::{DeleteRequest, DeleteResult, ListResult, WorktreeRow};

    use super::{DeleteFlow, DeleteFlowOps, FlowSignal, Step};

    struct FakeOps {
        rows: Vec<WorktreeRow>,
        delete_calls: RefCell<Vec<DeleteRequest>>,
        branch_not_merged: bool,
    }

    impl FakeOps {
        fn new(session_running: bool, branch_not_merged: bool) -> Self {
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
                branch_not_merged,
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
            let branch_deleted = if request.delete_branch {
                !self.branch_not_merged
            } else {
                false
            };

            Ok(DeleteResult {
                worktree_name: request.worktree_name,
                worktree_path: PathBuf::from("/tmp/repo/worktrees/w1"),
                session_name: "repo/w1".to_string(),
                branch_name: "w1".to_string(),
                branch_deleted,
            })
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn apply_keys(flow: &mut DeleteFlow, ops: &FakeOps, keys: &[KeyCode]) {
        for code in keys {
            let _ = flow.on_key(key(*code), ops).expect("apply key");
        }
    }

    #[test]
    fn esc_on_first_step_exits_flow() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        let signal = flow.on_key(key(KeyCode::Esc), &ops).expect("signal");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
    }

    #[test]
    fn options_are_collected_and_passed_to_delete_request() {
        let ops = FakeOps::new(true, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Right,
                KeyCode::Down,
                KeyCode::Right,
                KeyCode::Enter,
                KeyCode::Right,
                KeyCode::Enter,
            ],
        );

        assert_eq!(flow.step, Step::Success);

        let calls = ops.delete_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].kill_tmux_session);
        assert!(calls[0].delete_branch);
    }

    #[test]
    fn branch_not_merged_still_returns_success() {
        let ops = FakeOps::new(false, true);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Right,
                KeyCode::Enter,
                KeyCode::Right,
                KeyCode::Enter,
            ],
        );

        assert_eq!(flow.step, Step::Success);
        assert!(
            flow.success_message
                .as_deref()
                .unwrap_or("")
                .contains("kept (not fully merged)")
        );
    }

    #[test]
    fn delete_cancel_shows_notice_and_makes_no_delete_call() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops).expect("select");
        assert_eq!(flow.step, Step::Options);

        flow.on_key(key(KeyCode::Enter), &ops).expect("go confirm");
        assert_eq!(flow.step, Step::Confirm);

        flow.on_key(key(KeyCode::Enter), &ops).expect("confirm no");
        assert_eq!(flow.step, Step::Notice);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("close notice");
        assert_eq!(flow.step, Step::SelectWorktree);

        let calls = ops.delete_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn option_screen_supports_h_and_l_keys() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops).expect("select");
        assert_eq!(flow.step, Step::Options);

        flow.on_key(key(KeyCode::Down), &ops)
            .expect("delete branch option");
        flow.on_key(key(KeyCode::Char('l')), &ops).expect("set yes");
        assert!(flow.options.delete_branch);

        flow.on_key(key(KeyCode::Char('h')), &ops).expect("set no");
        assert!(!flow.options.delete_branch);
    }

    #[test]
    fn select_step_enter_noop_when_filter_has_no_matches() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[KeyCode::Char('z'), KeyCode::Char('z'), KeyCode::Enter],
        );

        assert_eq!(flow.step, Step::SelectWorktree);
        assert!(flow.target.is_none());
        let calls = ops.delete_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn success_screen_enter_returns_completed_exit() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Right,
                KeyCode::Enter,
                KeyCode::Right,
                KeyCode::Enter,
            ],
        );
        assert_eq!(flow.step, Step::Success);

        let signal = flow
            .on_key(key(KeyCode::Enter), &ops)
            .expect("exit success");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::Completed));

        let calls = ops.delete_calls.borrow();
        assert_eq!(calls.len(), 1);
    }
}
