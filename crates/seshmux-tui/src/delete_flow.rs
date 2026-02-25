use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use seshmux_app::{App, DeleteError, DeleteRequest, DeleteResult, ListResult, WorktreeRow};

use crate::{UiExit, centered_rect};

pub(crate) trait DeleteFlowOps {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult>;
    fn delete_worktree(&self, request: DeleteRequest) -> Result<DeleteResult>;
    fn force_delete_branch(&self, cwd: &Path, branch_name: &str) -> Result<()>;
}

impl<'a> DeleteFlowOps for App<'a> {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult> {
        self.list(cwd)
    }

    fn delete_worktree(&self, request: DeleteRequest) -> Result<DeleteResult> {
        self.delete(request)
    }

    fn force_delete_branch(&self, cwd: &Path, branch_name: &str) -> Result<()> {
        self.force_delete_branch(cwd, branch_name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    SelectWorktree,
    ConfirmDelete,
    ConfirmKillSession,
    ConfirmDeleteBranch,
    ConfirmForceBranchDelete,
    Notice,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSignal {
    Continue,
    Exit(UiExit),
}

#[derive(Debug)]
struct DeleteFlow {
    cwd: PathBuf,
    step: Step,
    rows: Vec<WorktreeRow>,
    filtered: Vec<usize>,
    selected: usize,
    query: String,
    target: Option<WorktreeRow>,
    confirm_yes_selected: bool,
    kill_yes_selected: bool,
    delete_branch_yes_selected: bool,
    force_delete_yes_selected: bool,
    pending_branch_for_force: Option<String>,
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
            query: String::new(),
            target: None,
            confirm_yes_selected: true,
            kill_yes_selected: false,
            delete_branch_yes_selected: false,
            force_delete_yes_selected: false,
            pending_branch_for_force: None,
            success_message: None,
            error_message: None,
        };
        flow.refresh_filtered();
        Ok(flow)
    }

    fn on_key(&mut self, key: KeyEvent, ops: &dyn DeleteFlowOps) -> Result<FlowSignal> {
        match self.step {
            Step::SelectWorktree => self.on_key_select(key),
            Step::ConfirmDelete => self.on_key_confirm_delete(key),
            Step::ConfirmKillSession => self.on_key_confirm_kill_session(key),
            Step::ConfirmDeleteBranch => self.on_key_confirm_delete_branch(key, ops),
            Step::ConfirmForceBranchDelete => self.on_key_confirm_force_branch_delete(key, ops),
            Step::Notice => self.on_key_notice(key),
            Step::Success => self.on_key_success(key),
            Step::Error => self.on_key_error(key),
        }
    }

    fn on_key_select(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => Ok(FlowSignal::Exit(UiExit::BackAtRoot)),
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Ok(FlowSignal::Continue)
            }
            KeyCode::Down => {
                if self.selected + 1 < self.filtered.len() {
                    self.selected += 1;
                }
                Ok(FlowSignal::Continue)
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refresh_filtered();
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.query.push(character);
                self.refresh_filtered();
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                let Some(target) = self.selected_row().cloned() else {
                    return Ok(FlowSignal::Continue);
                };

                self.target = Some(target);
                self.confirm_yes_selected = true;
                self.delete_branch_yes_selected = false;
                self.kill_yes_selected = false;
                self.step = Step::ConfirmDelete;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_confirm_delete(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::SelectWorktree;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Left | KeyCode::Up => {
                self.confirm_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'y' || character == 'Y' => {
                self.confirm_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Right | KeyCode::Down => {
                self.confirm_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'n' || character == 'N' => {
                self.confirm_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                if !self.confirm_yes_selected {
                    self.success_message =
                        Some("Delete canceled. No changes were made.".to_string());
                    self.step = Step::Notice;
                    return Ok(FlowSignal::Continue);
                }

                if self
                    .target
                    .as_ref()
                    .map(|value| value.session_running)
                    .unwrap_or(false)
                {
                    self.kill_yes_selected = false;
                    self.step = Step::ConfirmKillSession;
                } else {
                    self.delete_branch_yes_selected = false;
                    self.step = Step::ConfirmDeleteBranch;
                }

                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_confirm_kill_session(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::ConfirmDelete;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Left | KeyCode::Up => {
                self.kill_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'y' || character == 'Y' => {
                self.kill_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Right | KeyCode::Down => {
                self.kill_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'n' || character == 'N' => {
                self.kill_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                self.delete_branch_yes_selected = false;
                self.step = Step::ConfirmDeleteBranch;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_confirm_delete_branch(
        &mut self,
        key: KeyEvent,
        ops: &dyn DeleteFlowOps,
    ) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                if self
                    .target
                    .as_ref()
                    .map(|value| value.session_running)
                    .unwrap_or(false)
                {
                    self.step = Step::ConfirmKillSession;
                } else {
                    self.step = Step::ConfirmDelete;
                }
                Ok(FlowSignal::Continue)
            }
            KeyCode::Left | KeyCode::Up => {
                self.delete_branch_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'y' || character == 'Y' => {
                self.delete_branch_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Right | KeyCode::Down => {
                self.delete_branch_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'n' || character == 'N' => {
                self.delete_branch_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                self.execute_delete(ops)?;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_confirm_force_branch_delete(
        &mut self,
        key: KeyEvent,
        ops: &dyn DeleteFlowOps,
    ) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Left | KeyCode::Up => {
                self.force_delete_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'y' || character == 'Y' => {
                self.force_delete_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Right | KeyCode::Down => {
                self.force_delete_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character) if character == 'n' || character == 'N' => {
                self.force_delete_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Esc | KeyCode::Enter if !self.force_delete_yes_selected => {
                self.success_message =
                    Some("Worktree removed. Branch kept (not force deleted).".to_string());
                self.step = Step::Success;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                let Some(branch_name) = self.pending_branch_for_force.clone() else {
                    self.step = Step::Error;
                    self.error_message = Some("missing branch name for force delete".to_string());
                    return Ok(FlowSignal::Continue);
                };

                match ops.force_delete_branch(&self.cwd, &branch_name) {
                    Ok(()) => {
                        self.success_message = Some(format!(
                            "Worktree removed and branch '{branch_name}' force deleted."
                        ));
                        self.step = Step::Success;
                        self.error_message = None;
                    }
                    Err(error) => {
                        self.error_message = Some(error.to_string());
                        self.step = Step::Error;
                    }
                }
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_success(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => Ok(FlowSignal::Exit(UiExit::Completed)),
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_notice(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.step = Step::SelectWorktree;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_error(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.step = Step::SelectWorktree;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
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
            kill_tmux_session: self.kill_yes_selected,
            delete_branch: self.delete_branch_yes_selected,
        }) {
            Ok(result) => {
                self.success_message = Some(format!(
                    "Deleted worktree '{}'. Session '{}'. Branch deleted: {}.",
                    result.worktree_name, result.session_name, result.branch_deleted
                ));
                self.error_message = None;
                self.rows.retain(|row| row.name != result.worktree_name);
                self.refresh_filtered();
                self.step = Step::Success;
            }
            Err(error) => {
                if let Some(DeleteError::BranchRequiresForce { branch }) =
                    error.downcast_ref::<DeleteError>()
                {
                    self.pending_branch_for_force = Some(branch.clone());
                    self.force_delete_yes_selected = false;
                    self.step = Step::ConfirmForceBranchDelete;
                } else {
                    self.error_message = Some(error.to_string());
                    self.step = Step::Error;
                }
            }
        }
        Ok(())
    }

    fn refresh_filtered(&mut self) {
        let query = self.query.trim().to_lowercase();
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
            Step::ConfirmDelete => self.render_confirm_delete(frame),
            Step::ConfirmKillSession => self.render_confirm_kill_session(frame),
            Step::ConfirmDeleteBranch => self.render_confirm_delete_branch(frame),
            Step::ConfirmForceBranchDelete => self.render_confirm_force_branch_delete(frame),
            Step::Notice => self.render_notice(frame),
            Step::Success => self.render_success(frame),
            Step::Error => self.render_error(frame),
        }
    }

    fn render_select(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        let title = format!("Delete: select worktree (filter: {})", self.query);
        let mut items = Vec::<ListItem<'_>>::new();

        if self.filtered.is_empty() {
            items.push(ListItem::new("No matching worktrees."));
        } else {
            for index in &self.filtered {
                if let Some(row) = self.rows.get(*index) {
                    let state = if row.session_running {
                        "session: running"
                    } else {
                        "session: not running"
                    };
                    items.push(ListItem::new(format!(
                        "{} | {} | {} | {}",
                        row.name, row.created_at, row.branch, state
                    )));
                }
            }
        }

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            );

        let mut state = ListState::default();
        if !self.filtered.is_empty() {
            state.select(Some(self.selected));
        }
        frame.render_stateful_widget(list, body, &mut state);

        let keys = Paragraph::new(
            "Type: filter    Enter: select    Up/Down: move    Backspace: delete char    Esc: back",
        )
        .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_confirm_delete(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(70, 35, frame.area());
        frame.render_widget(Clear, area);

        let target = self
            .target
            .as_ref()
            .map(|value| value.name.as_str())
            .unwrap_or("UNCONFIRMED");
        let selection = if self.confirm_yes_selected {
            "Yes"
        } else {
            "No"
        };
        let text = format!(
            "Delete worktree '{target}'?\n\nSelection: {selection}\n\nLeft/Right: choose    Enter: continue    Esc: back"
        );
        let widget =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Delete"));
        frame.render_widget(widget, area);
    }

    fn render_confirm_kill_session(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(70, 35, frame.area());
        frame.render_widget(Clear, area);

        let target = self
            .target
            .as_ref()
            .map(|value| value.name.as_str())
            .unwrap_or("UNCONFIRMED");
        let selection = if self.kill_yes_selected { "Yes" } else { "No" };
        let text = format!(
            "Kill running tmux session for '{target}'?\n\nSelection: {selection}\n\nLeft/Right: choose    Enter: continue    Esc: back"
        );
        let widget =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Delete"));
        frame.render_widget(widget, area);
    }

    fn render_confirm_delete_branch(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(70, 35, frame.area());
        frame.render_widget(Clear, area);

        let target = self
            .target
            .as_ref()
            .map(|value| value.name.as_str())
            .unwrap_or("UNCONFIRMED");
        let selection = if self.delete_branch_yes_selected {
            "Yes"
        } else {
            "No"
        };
        let text = format!(
            "Delete branch '{target}' as part of cleanup?\n\nSelection: {selection}\n\nLeft/Right: choose    Enter: continue    Esc: back"
        );
        let widget =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Delete"));
        frame.render_widget(widget, area);
    }

    fn render_confirm_force_branch_delete(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(75, 40, frame.area());
        frame.render_widget(Clear, area);

        let branch = self
            .pending_branch_for_force
            .as_deref()
            .unwrap_or("UNCONFIRMED");
        let selection = if self.force_delete_yes_selected {
            "Yes"
        } else {
            "No"
        };
        let text = format!(
            "Branch '{branch}' is not fully merged.\nForce delete with -D?\n\nSelection: {selection}\n\nLeft/Right: choose    Enter: continue    Esc: keep branch"
        );
        let widget =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Delete"));
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
    use seshmux_app::{DeleteError, DeleteRequest, DeleteResult, ListResult, WorktreeRow};

    use super::{DeleteFlow, DeleteFlowOps, FlowSignal, Step};

    struct FakeOps {
        rows: Vec<WorktreeRow>,
        delete_calls: RefCell<Vec<DeleteRequest>>,
        force_calls: RefCell<Vec<String>>,
        require_force: bool,
    }

    impl FakeOps {
        fn new(session_running: bool, require_force: bool) -> Self {
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
                force_calls: RefCell::new(Vec::new()),
                require_force,
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
            if self.require_force && request.delete_branch {
                return Err(DeleteError::BranchRequiresForce {
                    branch: "w1".to_string(),
                }
                .into());
            }

            Ok(DeleteResult {
                worktree_name: request.worktree_name,
                worktree_path: PathBuf::from("/tmp/repo/worktrees/w1"),
                session_name: "repo/w1".to_string(),
                branch_name: "w1".to_string(),
                branch_deleted: request.delete_branch,
            })
        }

        fn force_delete_branch(&self, _cwd: &Path, branch_name: &str) -> Result<()> {
            self.force_calls.borrow_mut().push(branch_name.to_string());
            Ok(())
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
    fn delete_flow_passes_kill_and_branch_choices_to_usecase() {
        let ops = FakeOps::new(true, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops).expect("select");
        assert_eq!(flow.step, Step::ConfirmDelete);

        flow.on_key(key(KeyCode::Left), &ops).expect("confirm yes");
        flow.on_key(key(KeyCode::Enter), &ops).expect("confirm");
        assert_eq!(flow.step, Step::ConfirmKillSession);

        flow.on_key(key(KeyCode::Left), &ops).expect("kill yes");
        flow.on_key(key(KeyCode::Enter), &ops).expect("continue");
        assert_eq!(flow.step, Step::ConfirmDeleteBranch);

        flow.on_key(key(KeyCode::Right), &ops)
            .expect("branch delete no");
        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");
        assert_eq!(flow.step, Step::Success);

        let calls = ops.delete_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].kill_tmux_session);
        assert!(!calls[0].delete_branch);
    }

    #[test]
    fn branch_requires_force_path_calls_force_delete_when_confirmed() {
        let ops = FakeOps::new(false, true);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops).expect("select");
        flow.on_key(key(KeyCode::Left), &ops).expect("confirm yes");
        flow.on_key(key(KeyCode::Enter), &ops).expect("continue");
        assert_eq!(flow.step, Step::ConfirmDeleteBranch);

        flow.on_key(key(KeyCode::Left), &ops).expect("branch yes");
        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");
        assert_eq!(flow.step, Step::ConfirmForceBranchDelete);

        flow.on_key(key(KeyCode::Left), &ops).expect("force yes");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("force execute");
        assert_eq!(flow.step, Step::Success);

        let force_calls = ops.force_calls.borrow();
        assert_eq!(force_calls.as_slice(), &["w1".to_string()]);
    }

    #[test]
    fn delete_cancel_shows_notice_and_makes_no_delete_call() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops).expect("select");
        assert_eq!(flow.step, Step::ConfirmDelete);

        flow.on_key(key(KeyCode::Char('n')), &ops).expect("set no");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm default no");
        assert_eq!(flow.step, Step::Notice);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("close notice");
        assert_eq!(flow.step, Step::SelectWorktree);

        let calls = ops.delete_calls.borrow();
        assert!(calls.is_empty());
    }

    #[test]
    fn delete_confirm_defaults_to_yes_and_executes() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops).expect("select");
        assert_eq!(flow.step, Step::ConfirmDelete);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm default yes");
        assert_eq!(flow.step, Step::ConfirmDeleteBranch);

        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");
        assert_eq!(flow.step, Step::Success);

        let calls = ops.delete_calls.borrow();
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn enter_and_escape_navigation_is_deterministic_before_delete_executes() {
        let ops = FakeOps::new(false, false);
        let mut flow = DeleteFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        apply_keys(
            &mut flow,
            &ops,
            &[
                KeyCode::Enter,
                KeyCode::Esc,
                KeyCode::Enter,
                KeyCode::Enter,
                KeyCode::Esc,
                KeyCode::Esc,
            ],
        );

        assert_eq!(flow.step, Step::SelectWorktree);
        let calls = ops.delete_calls.borrow();
        assert!(calls.is_empty());
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
            &[KeyCode::Enter, KeyCode::Enter, KeyCode::Enter],
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
