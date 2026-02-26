#[derive(Debug, Clone)]
pub(crate) struct PickerState<T> {
    pub(crate) items: Vec<T>,
    pub(crate) selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerAction {
    Pick(usize),
    Noop,
}

impl<T> PickerState<T> {
    pub(crate) fn from_items(items: Vec<T>) -> Self {
        Self { items, selected: 0 }
    }

    pub(crate) fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(crate) fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub(crate) fn on_enter(&self) -> PickerAction {
        let index = self.selected;
        if index < self.items.len() {
            PickerAction::Pick(index)
        } else {
            PickerAction::Noop
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PickerAction, PickerState};

    #[test]
    fn enter_contract_for_item_rows() {
        let mut picker = PickerState::from_items(vec!["a", "b"]);
        assert_eq!(picker.on_enter(), PickerAction::Pick(0));

        picker.selected = 1;
        assert_eq!(picker.on_enter(), PickerAction::Pick(1));
    }

    #[test]
    fn empty_picker_confirm_is_noop() {
        let mut picker = PickerState::from_items(Vec::<&str>::new());
        picker.selected = 0;
        assert_eq!(picker.on_enter(), PickerAction::Noop);
    }

    #[test]
    fn movement_is_bounded() {
        let mut picker = PickerState::from_items(vec!["a"]);
        picker.move_down();
        picker.move_down();
        assert_eq!(picker.selected, 0);

        picker.move_up();
        picker.move_up();
        assert_eq!(picker.selected, 0);
    }
}
