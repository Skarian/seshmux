use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState,
};
use seshmux_app::{App, AttachError, AttachRequest, AttachResult, ListResult, WorktreeRow};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use crate::{UiExit, centered_rect};

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
    rows: Vec<WorktreeRow>,
    filtered: Vec<usize>,
    selected: usize,
    query: Input,
    missing_yes_selected: bool,
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
        let mut flow = Self {
            cwd: cwd.to_path_buf(),
            step: Step::SelectWorktree,
            rows: result.rows,
            filtered: Vec::new(),
            selected: 0,
            query: Input::default(),
            missing_yes_selected: true,
            pending_worktree_name: None,
            success_message: None,
            error_message: None,
        };
        flow.refresh_filtered();
        Ok(flow)
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
            KeyCode::Enter => {
                let Some(row) = self.selected_row().cloned() else {
                    return Ok(FlowSignal::Continue);
                };

                match ops.attach_worktree(AttachRequest {
                    cwd: self.cwd.clone(),
                    worktree_name: row.name.clone(),
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
                            self.missing_yes_selected = true;
                            self.step = Step::MissingSessionPrompt;
                        } else {
                            self.error_message = Some(error.to_string());
                            self.success_message = None;
                            self.step = Step::Error;
                        }
                    }
                }

                Ok(FlowSignal::Continue)
            }
            _ => {
                if self.query.handle_event(&Event::Key(key)).is_some() {
                    self.refresh_filtered();
                }
                Ok(FlowSignal::Continue)
            }
        }
    }

    fn on_key_missing_prompt(
        &mut self,
        key: KeyEvent,
        ops: &dyn AttachFlowOps,
    ) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::SelectWorktree;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Left | KeyCode::Up => {
                self.missing_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Right | KeyCode::Down => {
                self.missing_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                if !self.missing_yes_selected {
                    self.step = Step::SelectWorktree;
                    return Ok(FlowSignal::Continue);
                }

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
                        self.error_message = Some(error.to_string());
                        self.success_message = None;
                        self.step = Step::Error;
                    }
                }
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

    fn on_key_error(&mut self, key: KeyEvent) -> FlowSignal {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.step = Step::SelectWorktree;
                FlowSignal::Continue
            }
            _ => FlowSignal::Continue,
        }
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
            Step::MissingSessionPrompt => self.render_missing_prompt(frame),
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
                    .title("Attach: select worktree"),
            );
            frame.render_widget(empty, table_area);
        } else {
            let header = Row::new(["Name", "Created", "Branch", "Session"]).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
            let rows = self
                .filtered
                .iter()
                .filter_map(|index| self.rows.get(*index))
                .map(|row| {
                    let state = if row.session_running {
                        "running"
                    } else {
                        "missing"
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
                    Constraint::Length(12),
                ],
            )
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Attach: select worktree"),
            )
            .row_highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
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

        let keys = Paragraph::new("Type: filter    Enter: attach    Up/Down: move    Esc: back")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_filter(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let width = area.width.saturating_sub(2) as usize;
        let scroll = self.query.visual_scroll(width);
        let query_text = self.query.value();
        let paragraph = Paragraph::new(query_text)
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

    fn render_missing_prompt(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(70, 35, frame.area());
        frame.render_widget(Clear, area);

        let selection = if self.missing_yes_selected {
            "Yes"
        } else {
            "No"
        };
        let worktree = self
            .pending_worktree_name
            .as_deref()
            .unwrap_or("UNCONFIRMED");
        let text = format!(
            "No tmux session found for '{worktree}'.\nWould you like to create one?\n\nSelection: {selection}\n\nLeft/Right: choose    Enter: continue    Esc: back"
        );
        let widget =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Attach"));
        frame.render_widget(widget, area);
    }

    fn render_success(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(70, 35, frame.area());
        frame.render_widget(Clear, area);

        let text = self
            .success_message
            .as_deref()
            .unwrap_or("Attached.\n\nEnter/Esc to return.");
        let widget =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Success"));
        frame.render_widget(widget, area);
    }

    fn render_error(&self, frame: &mut ratatui::Frame<'_>) {
        let area = centered_rect(80, 40, frame.area());
        frame.render_widget(Clear, area);

        let text = self.error_message.as_deref().unwrap_or("Attach failed.");
        let widget = Paragraph::new(format!("{text}\n\nEnter/Esc to return."))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(widget, area);
    }
}

fn success_message_for(result: &AttachResult) -> String {
    format!(
        "Connected to {}.\nSession: {}\nCreated session now: {}\n\nEnter/Esc to return.",
        result.worktree_name, result.session_name, result.created_session
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
}
