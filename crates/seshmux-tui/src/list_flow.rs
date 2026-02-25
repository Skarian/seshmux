use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Color;
use seshmux_app::{App, ListResult};

use crate::UiExit;
use crate::theme;
use crate::ui::select_step::{SelectSignal, SelectStepState};
use crate::ui::text::{compact_hint, wrapped_paragraph};
use crate::ui::worktree_table::{TableColumn, WorktreeTableRender};

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
    select: SelectStepState,
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
            select: SelectStepState::new(result.rows),
        })
    }

    fn on_key(&mut self, key: KeyEvent, ops: &dyn ListFlowOps, cwd: &Path) -> Result<FlowSignal> {
        match self.select.on_key(key) {
            SelectSignal::Back => return Ok(FlowSignal::Exit(UiExit::BackAtRoot)),
            SelectSignal::Continue => {}
            SelectSignal::Confirm => {
                let result = ops.list_worktrees(cwd)?;
                self.select.set_rows(result.rows);
                return Ok(FlowSignal::Continue);
            }
        }

        if key.code == KeyCode::Char('r') && !self.select.filter_focused() {
            let result = ops.list_worktrees(cwd)?;
            self.select.set_rows(result.rows);
        }

        Ok(FlowSignal::Continue)
    }

    fn render(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [filter_area, body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(3),
            ])
            .areas(area);

        self.select.render_filter(
            frame,
            filter_area,
            "Filter (focused)",
            "Filter (Tab to focus)",
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
            TableColumn {
                title: "Path",
                width: Constraint::Min(24),
            },
        ];

        self.select.render_table(
            frame,
            body,
            WorktreeTableRender {
                title: "List: worktrees",
                empty_message: "No worktrees are registered.",
                columns: &columns,
                header_style: theme::table_header(Color::Cyan),
                highlight_style: theme::table_highlight(Color::Cyan),
            },
            |row| {
                let status = if row.session_running {
                    "running"
                } else {
                    "not running"
                };
                vec![
                    row.name.clone(),
                    row.created_at.clone(),
                    row.branch.clone(),
                    status.to_string(),
                    row.path.display().to_string(),
                ]
            },
        );

        let key_text = if self.select.filter_focused() {
            compact_hint(
                area.width,
                "Type: filter    Backspace: delete    Tab: list focus    Esc: back",
                "Type filter    Backspace delete    Tab: list    Esc: back",
                "Type filter | Backspace | Tab list | Esc back",
            )
        } else {
            compact_hint(
                area.width,
                "Tab: filter focus    Up/Down or j/k: move    Enter/r: refresh    Esc: back",
                "Tab: filter    j/k: move    Enter/r: refresh    Esc: back",
                "Tab filter | j/k move | Enter refresh | Esc back",
            )
        };
        let keys = wrapped_paragraph(key_text).block(theme::key_block());
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

    #[test]
    fn vim_navigation_moves_selection() {
        let ops = FakeOps {
            rows: vec![
                WorktreeRow {
                    name: "w1".to_string(),
                    path: PathBuf::from("/tmp/repo/worktrees/w1"),
                    created_at: "2026-02-25T10:00:00Z".to_string(),
                    branch: "w1".to_string(),
                    session_name: "repo/w1".to_string(),
                    session_running: false,
                },
                WorktreeRow {
                    name: "w2".to_string(),
                    path: PathBuf::from("/tmp/repo/worktrees/w2"),
                    created_at: "2026-02-25T11:00:00Z".to_string(),
                    branch: "w2".to_string(),
                    session_name: "repo/w2".to_string(),
                    session_running: false,
                },
            ],
        };
        let mut flow = ListFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Char('j')), &ops, Path::new("/tmp/repo"))
            .expect("down");
        assert_eq!(flow.select.selected(), 1);

        flow.on_key(key(KeyCode::Char('k')), &ops, Path::new("/tmp/repo"))
            .expect("up");
        assert_eq!(flow.select.selected(), 0);
    }

    #[test]
    fn enter_refreshes_rows() {
        let ops = FakeOps {
            rows: vec![WorktreeRow {
                name: "w1".to_string(),
                path: PathBuf::from("/tmp/repo/worktrees/w1"),
                created_at: "2026-02-25T10:00:00Z".to_string(),
                branch: "w1".to_string(),
                session_name: "repo/w1".to_string(),
                session_running: false,
            }],
        };
        let mut flow = ListFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Enter), &ops, Path::new("/tmp/repo"))
            .expect("refresh");
        assert_eq!(flow.select.filtered_len(), 1);
    }

    #[test]
    fn tab_focus_routes_text_input_to_filter() {
        let ops = FakeOps {
            rows: vec![
                WorktreeRow {
                    name: "w1".to_string(),
                    path: PathBuf::from("/tmp/repo/worktrees/w1"),
                    created_at: "2026-02-25T10:00:00Z".to_string(),
                    branch: "w1".to_string(),
                    session_name: "repo/w1".to_string(),
                    session_running: false,
                },
                WorktreeRow {
                    name: "w2".to_string(),
                    path: PathBuf::from("/tmp/repo/worktrees/w2"),
                    created_at: "2026-02-25T11:00:00Z".to_string(),
                    branch: "w2".to_string(),
                    session_name: "repo/w2".to_string(),
                    session_running: false,
                },
            ],
        };
        let mut flow = ListFlow::new(&ops, Path::new("/tmp/repo")).expect("flow");

        flow.on_key(key(KeyCode::Tab), &ops, Path::new("/tmp/repo"))
            .expect("focus filter");
        assert!(flow.select.filter_focused());

        flow.on_key(key(KeyCode::Char('j')), &ops, Path::new("/tmp/repo"))
            .expect("type filter");
        assert_eq!(flow.select.selected(), 0);
        assert_eq!(flow.select.filtered_len(), 0);
    }
}
