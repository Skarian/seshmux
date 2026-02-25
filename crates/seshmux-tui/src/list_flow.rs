use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState,
};
use seshmux_app::{App, ListResult, WorktreeRow};

use crate::UiExit;

pub(crate) trait ListFlowOps {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult>;
}

impl<'a> ListFlowOps for App<'a> {
    fn list_worktrees(&self, cwd: &Path) -> Result<ListResult> {
        self.list(cwd)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSignal {
    Continue,
    Exit(UiExit),
}

#[derive(Debug)]
struct ListFlow {
    rows: Vec<WorktreeRow>,
    selected: usize,
}

pub(crate) struct ListScreen {
    flow: ListFlow,
    cwd: PathBuf,
}

impl ListScreen {
    pub(crate) fn new(app: &App<'_>, cwd: &Path) -> Result<Self> {
        Ok(Self {
            flow: ListFlow::new(app, cwd)?,
            cwd: cwd.to_path_buf(),
        })
    }

    pub(crate) fn render(&self, frame: &mut ratatui::Frame<'_>) {
        self.flow.render(frame);
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent, app: &App<'_>) -> Result<Option<UiExit>> {
        match self.flow.on_key(key, app, &self.cwd)? {
            FlowSignal::Continue => Ok(None),
            FlowSignal::Exit(exit) => Ok(Some(exit)),
        }
    }
}

impl ListFlow {
    fn new(ops: &dyn ListFlowOps, cwd: &Path) -> Result<Self> {
        let result = ops.list_worktrees(cwd)?;
        Ok(Self {
            rows: result.rows,
            selected: 0,
        })
    }

    fn on_key(&mut self, key: KeyEvent, ops: &dyn ListFlowOps, cwd: &Path) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => Ok(FlowSignal::Exit(UiExit::BackAtRoot)),
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Ok(FlowSignal::Continue)
            }
            KeyCode::Down => {
                if self.selected + 1 < self.rows.len() {
                    self.selected += 1;
                }
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char('r') | KeyCode::Enter => {
                let result = ops.list_worktrees(cwd)?;
                self.rows = result.rows;
                if self.rows.is_empty() {
                    self.selected = 0;
                } else if self.selected >= self.rows.len() {
                    self.selected = self.rows.len() - 1;
                }
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn render(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        if self.rows.is_empty() {
            let empty = Paragraph::new("No worktrees are registered.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("List: worktrees"),
            );
            frame.render_widget(empty, body);
        } else {
            let header = Row::new(["Name", "Created", "Branch", "Session", "Path"]).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
            let rows = self.rows.iter().map(|row| {
                let status = if row.session_running {
                    "running"
                } else {
                    "not running"
                };
                Row::new(vec![
                    row.name.clone(),
                    row.created_at.clone(),
                    row.branch.clone(),
                    status.to_string(),
                    row.path.display().to_string(),
                ])
            });
            let table = Table::new(
                rows,
                [
                    Constraint::Length(24),
                    Constraint::Length(28),
                    Constraint::Length(20),
                    Constraint::Length(14),
                    Constraint::Min(24),
                ],
            )
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("List: worktrees"),
            )
            .row_highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

            let mut state = TableState::new();
            state.select(Some(self.selected));
            frame.render_stateful_widget(table, body, &mut state);

            let viewport = body.height.saturating_sub(3) as usize;
            let mut scrollbar_state = ScrollbarState::new(self.rows.len())
                .position(self.selected)
                .viewport_content_length(viewport);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                body.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }

        let keys =
            Paragraph::new("Up/Down: move    Enter/r: refresh    Esc: back    Ctrl+C: cancel")
                .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use seshmux_app::{ListResult, WorktreeRow};

    use super::{FlowSignal, ListFlow, ListFlowOps};

    struct FakeOps {
        rows: Vec<WorktreeRow>,
    }

    impl ListFlowOps for FakeOps {
        fn list_worktrees(&self, _cwd: &Path) -> Result<ListResult> {
            Ok(ListResult {
                repo_root: PathBuf::from("/tmp/repo"),
                rows: self.rows.clone(),
            })
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn esc_on_first_step_exits_flow() {
        let ops = FakeOps { rows: Vec::new() };
        let mut flow = ListFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");
        let signal = flow
            .on_key(key(KeyCode::Esc), &ops, Path::new("/tmp/repo"))
            .expect("signal");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
    }
}
