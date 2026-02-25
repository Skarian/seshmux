use crossterm::event::KeyEvent;

use crate::keymap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BinaryChoice {
    pub(crate) yes_selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryChoiceEvent {
    Continue,
    ConfirmYes,
    ConfirmNo,
    Back,
}

impl BinaryChoice {
    pub(crate) fn new(default_yes: bool) -> Self {
        Self {
            yes_selected: default_yes,
        }
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent) -> BinaryChoiceEvent {
        if keymap::is_back(key) {
            return BinaryChoiceEvent::Back;
        }

        if keymap::is_toggle(key) {
            self.yes_selected = !self.yes_selected;
            return BinaryChoiceEvent::Continue;
        }

        if keymap::is_confirm(key) {
            if self.yes_selected {
                BinaryChoiceEvent::ConfirmYes
            } else {
                BinaryChoiceEvent::ConfirmNo
            }
        } else {
            BinaryChoiceEvent::Continue
        }
    }

    pub(crate) fn selected_label(&self) -> &'static str {
        if self.yes_selected { "Yes" } else { "No" }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{BinaryChoice, BinaryChoiceEvent};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn space_toggles_selection() {
        let mut choice = BinaryChoice::new(true);
        assert_eq!(choice.selected_label(), "Yes");

        assert_eq!(
            choice.on_key(key(KeyCode::Char(' '))),
            BinaryChoiceEvent::Continue
        );
        assert_eq!(choice.selected_label(), "No");

        assert_eq!(
            choice.on_key(key(KeyCode::Char(' '))),
            BinaryChoiceEvent::Continue
        );
        assert_eq!(choice.selected_label(), "Yes");
    }

    #[test]
    fn enter_confirms_current_selection() {
        let mut choice = BinaryChoice::new(true);
        assert_eq!(
            choice.on_key(key(KeyCode::Enter)),
            BinaryChoiceEvent::ConfirmYes
        );

        let mut choice = BinaryChoice::new(false);
        assert_eq!(
            choice.on_key(key(KeyCode::Enter)),
            BinaryChoiceEvent::ConfirmNo
        );
    }

    #[test]
    fn esc_returns_back() {
        let mut choice = BinaryChoice::new(false);
        assert_eq!(choice.on_key(key(KeyCode::Esc)), BinaryChoiceEvent::Back);
        assert_eq!(choice.selected_label(), "No");
    }

    #[test]
    fn unrelated_keys_do_not_change_state() {
        let mut choice = BinaryChoice::new(true);
        assert_eq!(
            choice.on_key(key(KeyCode::Char('y'))),
            BinaryChoiceEvent::Continue
        );
        assert_eq!(choice.selected_label(), "Yes");
    }
}
