#[derive(Debug, Clone)]
pub(crate) struct PickerState<T> {
    pub(crate) query: Option<String>,
    pub(crate) items: Vec<T>,
    pub(crate) selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerAction {
    OpenSearch,
    ShowAll,
    Pick(usize),
    Noop,
}

impl<T> PickerState<T> {
    pub(crate) fn from_items(query: Option<String>, items: Vec<T>) -> Self {
        Self {
            query,
            items,
            selected: 0,
        }
    }

    pub(crate) fn row_count(&self) -> usize {
        self.action_row_count() + self.items.len()
    }

    pub(crate) fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(crate) fn move_down(&mut self) {
        if self.selected + 1 < self.row_count() {
            self.selected += 1;
        }
    }

    pub(crate) fn on_enter(&self) -> PickerAction {
        if self.selected == 0 {
            return PickerAction::OpenSearch;
        }

        if self.query.is_some() && self.selected == 1 {
            return PickerAction::ShowAll;
        }

        let index = self.selected.saturating_sub(self.action_row_count());
        if index < self.items.len() {
            PickerAction::Pick(index)
        } else {
            PickerAction::Noop
        }
    }

    pub(crate) fn action_row_count(&self) -> usize {
        if self.query.is_some() { 2 } else { 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::{PickerAction, PickerState};

    #[test]
    fn enter_contract_for_search_and_item_rows() {
        let picker = PickerState::from_items(None, vec!["a", "b"]);
        assert_eq!(picker.on_enter(), PickerAction::OpenSearch);

        let mut picker = PickerState::from_items(None, vec!["a", "b"]);
        picker.selected = 1;
        assert_eq!(picker.on_enter(), PickerAction::Pick(0));
    }

    #[test]
    fn show_all_row_exists_only_when_query_is_present() {
        let mut picker = PickerState::from_items(Some("abc".to_string()), vec!["a"]);
        picker.selected = 1;
        assert_eq!(picker.on_enter(), PickerAction::ShowAll);

        let mut picker = PickerState::from_items(None, Vec::<&str>::new());
        picker.selected = 1;
        assert_eq!(picker.on_enter(), PickerAction::Noop);
    }

    #[test]
    fn movement_is_bounded() {
        let mut picker = PickerState::from_items(None, vec!["a"]);
        picker.move_down();
        picker.move_down();
        assert_eq!(picker.selected, 1);

        picker.move_up();
        picker.move_up();
        assert_eq!(picker.selected, 0);
    }
}
