use ratatui::text::Text;
use ratatui::widgets::{Paragraph, Wrap};

pub(crate) fn wrapped_paragraph<'a, T>(text: T) -> Paragraph<'a>
where
    T: Into<Text<'a>>,
{
    Paragraph::new(text).wrap(Wrap { trim: false })
}

pub(crate) fn compact_hint<'a>(
    width: u16,
    full: &'a str,
    medium: &'a str,
    compact: &'a str,
) -> &'a str {
    if width >= 110 {
        full
    } else if width >= 78 {
        medium
    } else {
        compact
    }
}

#[cfg(test)]
mod tests {
    use super::compact_hint;

    #[test]
    fn compact_hint_selects_variant_by_width() {
        assert_eq!(compact_hint(120, "full", "medium", "compact"), "full");
        assert_eq!(compact_hint(90, "full", "medium", "compact"), "medium");
        assert_eq!(compact_hint(60, "full", "medium", "compact"), "compact");
    }
}
