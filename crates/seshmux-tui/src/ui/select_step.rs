use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use seshmux_app::WorktreeRow;

use crate::keymap;

use super::worktree_table::{WorktreeTableRender, WorktreeTableState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectSignal {
    Continue,
    Back,
    Confirm,
}

#[derive(Debug)]
pub(crate) struct SelectStepState {
    table: WorktreeTableState,
    filter_focused: bool,
}

impl SelectStepState {
    pub(crate) fn new(rows: Vec<WorktreeRow>) -> Self {
        Self {
            table: WorktreeTableState::new(rows),
            filter_focused: false,
        }
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent) -> SelectSignal {
        if keymap::is_back(key) {
            return SelectSignal::Back;
        }

        if matches!(key.code, KeyCode::Tab) {
            self.filter_focused = !self.filter_focused;
            return SelectSignal::Continue;
        }

        if self.filter_focused {
            self.table.on_filter_key(key);
            return SelectSignal::Continue;
        }

        if keymap::is_up(key) {
            self.table.move_up();
            return SelectSignal::Continue;
        }

        if keymap::is_down(key) {
            self.table.move_down();
            return SelectSignal::Continue;
        }

        if keymap::is_confirm(key) {
            return SelectSignal::Confirm;
        }

        SelectSignal::Continue
    }

    pub(crate) fn render_filter(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focused_title: &str,
        unfocused_title: &str,
    ) {
        let title = if self.filter_focused {
            focused_title
        } else {
            unfocused_title
        };
        self.table
            .render_filter(frame, area, title, self.filter_focused);
    }

    pub(crate) fn render_table<F>(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        render: WorktreeTableRender<'_>,
        row_builder: F,
    ) where
        F: Fn(&WorktreeRow) -> Vec<String>,
    {
        self.table.render_table(frame, area, render, row_builder);
    }

    pub(crate) fn selected_row(&self) -> Option<&WorktreeRow> {
        self.table.selected_row()
    }

    pub(crate) fn set_rows(&mut self, rows: Vec<WorktreeRow>) {
        self.table.set_rows(rows);
    }

    pub(crate) fn remove_by_name(&mut self, name: &str) {
        self.table.remove_by_name(name);
    }

    pub(crate) fn filter_focused(&self) -> bool {
        self.filter_focused
    }

    pub(crate) fn set_filter_focused(&mut self, value: bool) {
        self.filter_focused = value;
    }

    #[cfg(test)]
    pub(crate) fn selected(&self) -> usize {
        self.table.selected()
    }

    #[cfg(test)]
    pub(crate) fn filtered_len(&self) -> usize {
        self.table.filtered_len()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use seshmux_app::WorktreeRow;

    use super::{SelectSignal, SelectStepState};

    fn row(name: &str) -> WorktreeRow {
        WorktreeRow {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/repo/worktrees/{name}")),
            created_at: "2026-02-25T10:00:00Z".to_string(),
            branch: name.to_string(),
            session_name: format!("repo/{name}"),
            session_running: false,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn tab_toggles_focus_and_routes_filter_input() {
        let mut state = SelectStepState::new(vec![row("alpha"), row("beta")]);
        assert!(!state.filter_focused());

        assert_eq!(state.on_key(key(KeyCode::Tab)), SelectSignal::Continue);
        assert!(state.filter_focused());

        state.on_key(key(KeyCode::Char('b')));
        assert_eq!(state.filtered_len(), 1);
    }

    #[test]
    fn movement_and_confirm_work_in_list_focus() {
        let mut state = SelectStepState::new(vec![row("one"), row("two")]);

        assert_eq!(
            state.on_key(key(KeyCode::Char('j'))),
            SelectSignal::Continue
        );
        assert_eq!(state.selected(), 1);

        assert_eq!(state.on_key(key(KeyCode::Enter)), SelectSignal::Confirm);
        assert_eq!(state.on_key(key(KeyCode::Esc)), SelectSignal::Back);
    }
}
