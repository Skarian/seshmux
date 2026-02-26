use crossterm::event::{Event, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Margin, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{
    Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState,
};
use seshmux_app::WorktreeRow;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

#[derive(Debug, Clone, Copy)]
pub(crate) struct TableColumn {
    pub(crate) title: &'static str,
    pub(crate) width: Constraint,
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeTableRender<'a> {
    pub(crate) title: Line<'a>,
    pub(crate) empty_message: &'a str,
    pub(crate) columns: &'a [TableColumn],
    pub(crate) header_style: Style,
    pub(crate) highlight_style: Style,
}

#[derive(Debug)]
pub(crate) struct WorktreeTableState {
    rows: Vec<WorktreeRow>,
    filtered: Vec<usize>,
    selected: usize,
    query: Input,
}

impl WorktreeTableState {
    pub(crate) fn new(rows: Vec<WorktreeRow>) -> Self {
        let mut state = Self {
            rows,
            filtered: Vec::new(),
            selected: 0,
            query: Input::default(),
        };
        state.refresh_filtered();
        state
    }

    pub(crate) fn set_rows(&mut self, rows: Vec<WorktreeRow>) {
        self.rows = rows;
        self.refresh_filtered();
    }

    pub(crate) fn remove_by_name(&mut self, name: &str) {
        self.rows.retain(|row| row.name != name);
        self.refresh_filtered();
    }

    pub(crate) fn on_filter_key(&mut self, key: KeyEvent) {
        if self.query.handle_event(&Event::Key(key)).is_some() {
            self.refresh_filtered();
        }
    }

    pub(crate) fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(crate) fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    pub(crate) fn selected_row(&self) -> Option<&WorktreeRow> {
        let index = *self.filtered.get(self.selected)?;
        self.rows.get(index)
    }

    #[cfg(test)]
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    #[cfg(test)]
    pub(crate) fn filtered_len(&self) -> usize {
        self.filtered.len()
    }

    pub(crate) fn render_filter(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: Line<'_>,
        show_cursor: bool,
    ) {
        let width = area.width.saturating_sub(2) as usize;
        let scroll = self.query.visual_scroll(width);
        let paragraph = Paragraph::new(self.query.value())
            .scroll((0, scroll as u16))
            .block(crate::theme::chrome(title));
        frame.render_widget(paragraph, area);

        if !show_cursor || width == 0 {
            return;
        }

        let visual = self.query.visual_cursor();
        let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
        frame.set_cursor_position((area.x + 1 + relative as u16, area.y + 1));
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
        if self.filtered.is_empty() {
            let empty = Paragraph::new(render.empty_message)
                .block(crate::theme::chrome(render.title.clone()));
            frame.render_widget(empty, area);
            return;
        }

        let header =
            Row::new(render.columns.iter().map(|column| column.title)).style(render.header_style);
        let rows = self
            .filtered
            .iter()
            .filter_map(|index| self.rows.get(*index))
            .map(|row| Row::new(row_builder(row)));
        let widths: Vec<Constraint> = render.columns.iter().map(|column| column.width).collect();

        let table = Table::new(rows, widths)
            .header(header)
            .block(crate::theme::chrome(render.title))
            .row_highlight_style(render.highlight_style)
            .highlight_symbol(">> ");

        let mut state = TableState::new();
        state.select(Some(self.selected));
        frame.render_stateful_widget(table, area, &mut state);

        let viewport = area.height.saturating_sub(3) as usize;
        let mut scrollbar_state = ScrollbarState::new(self.filtered.len())
            .position(self.selected)
            .viewport_content_length(viewport);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
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
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use seshmux_app::WorktreeRow;

    use super::WorktreeTableState;

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
    fn filtering_reduces_visible_rows() {
        let mut state = WorktreeTableState::new(vec![row("alpha"), row("beta")]);

        state.on_filter_key(key(KeyCode::Char('b')));

        assert_eq!(state.filtered_len(), 1);
        assert_eq!(state.selected_row().expect("selected row").name, "beta");
    }

    #[test]
    fn selection_movement_stays_in_bounds() {
        let mut state = WorktreeTableState::new(vec![row("one"), row("two")]);

        state.move_down();
        state.move_down();
        assert_eq!(state.selected(), 1);

        state.move_up();
        state.move_up();
        assert_eq!(state.selected(), 0);
    }

    #[test]
    fn selected_row_is_none_when_filter_is_empty() {
        let mut state = WorktreeTableState::new(vec![row("one")]);
        state.on_filter_key(key(KeyCode::Char('q')));
        state.on_filter_key(key(KeyCode::Char('q')));
        state.on_filter_key(key(KeyCode::Char('q')));

        assert_eq!(state.filtered_len(), 0);
        assert!(state.selected_row().is_none());
    }
}
