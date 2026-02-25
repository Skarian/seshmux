use crossterm::event::{KeyCode, KeyEvent};

pub(crate) fn is_back(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc)
}

pub(crate) fn is_confirm(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Enter)
}

pub(crate) fn is_up(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Up | KeyCode::Char('k'))
}

pub(crate) fn is_down(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Down | KeyCode::Char('j'))
}

pub(crate) fn is_toggle(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(' '))
}

pub(crate) fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q'))
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{is_back, is_confirm, is_down, is_quit, is_toggle, is_up};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn up_keys_match_arrows_and_vim() {
        assert!(is_up(key(KeyCode::Up)));
        assert!(is_up(key(KeyCode::Char('k'))));
        assert!(!is_up(key(KeyCode::Char('j'))));
    }

    #[test]
    fn down_keys_match_arrows_and_vim() {
        assert!(is_down(key(KeyCode::Down)));
        assert!(is_down(key(KeyCode::Char('j'))));
        assert!(!is_down(key(KeyCode::Char('k'))));
    }

    #[test]
    fn confirm_back_toggle_and_quit_match_contract() {
        assert!(is_confirm(key(KeyCode::Enter)));
        assert!(is_back(key(KeyCode::Esc)));
        assert!(is_toggle(key(KeyCode::Char(' '))));
        assert!(is_quit(key(KeyCode::Char('q'))));
        assert!(!is_toggle(key(KeyCode::Char('y'))));
        assert!(!is_back(key(KeyCode::Enter)));
    }
}
