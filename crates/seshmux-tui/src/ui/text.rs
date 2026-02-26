use ratatui::layout::Alignment;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::theme;

pub(crate) fn wrapped_paragraph<'a, T>(text: T) -> Paragraph<'a>
where
    T: Into<Text<'a>>,
{
    Paragraph::new(text).wrap(Wrap { trim: false })
}

pub(crate) fn key_hint_paragraph<'a, T>(text: T) -> Paragraph<'a>
where
    T: Into<Text<'a>>,
{
    wrapped_paragraph(text).alignment(Alignment::Center)
}

pub(crate) fn key_hint_height(total_width: u16, text: &str) -> u16 {
    let content_width = total_width.saturating_sub(2).max(1) as usize;
    let lines = wrapped_line_count(text, content_width);
    lines.saturating_add(2).max(3)
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

pub(crate) fn focus_line(message: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(message.into(), theme::focus_prompt()))
}

pub(crate) fn label_value_line(
    label: impl Into<String>,
    value: impl Into<String>,
) -> Line<'static> {
    let label = label.into();
    let value = value.into();
    Line::from(vec![
        Span::styled(format!("{label}: "), theme::secondary_text()),
        Span::raw(value),
    ])
}

pub(crate) fn highlighted_label_value_line(
    label: impl Into<String>,
    value: impl Into<String>,
) -> Line<'static> {
    let label = label.into();
    let value = value.into();
    Line::from(vec![
        Span::styled(format!("{label}: "), theme::focus_prompt()),
        Span::styled(value, Style::default().add_modifier(Modifier::UNDERLINED)),
    ])
}

pub(crate) fn yes_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

pub(crate) fn result_footer(width: u16) -> &'static str {
    compact_hint(
        width,
        "Enter/Esc: back to home    q: quit seshmux",
        "Enter/Esc: home    q: quit",
        "Enter/Esc home | q quit",
    )
}

fn wrapped_line_count(text: &str, width: usize) -> u16 {
    if text.is_empty() {
        return 1;
    }

    let mut total = 0u16;
    for line in text.split('\n') {
        total = total.saturating_add(wrapped_line_count_single(line, width));
    }

    total.max(1)
}

fn wrapped_line_count_single(line: &str, width: usize) -> u16 {
    if line.is_empty() {
        return 1;
    }

    let mut lines = 1u16;
    let mut used = 0usize;
    for ch in line.chars() {
        let mut remaining = if ch == '\t' { 4 } else { 1 };
        while remaining > 0 {
            let space_left = width.saturating_sub(used);
            if space_left == 0 {
                lines = lines.saturating_add(1);
                used = 0;
                continue;
            }
            if remaining > space_left {
                remaining -= space_left;
                lines = lines.saturating_add(1);
                used = 0;
            } else {
                used += remaining;
                remaining = 0;
            }
        }
    }

    lines.max(1)
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier};

    use super::{
        compact_hint, focus_line, highlighted_label_value_line, key_hint_height, label_value_line,
        result_footer, wrapped_line_count_single, yes_no,
    };

    #[test]
    fn compact_hint_selects_variant_by_width() {
        assert_eq!(compact_hint(120, "full", "medium", "compact"), "full");
        assert_eq!(compact_hint(90, "full", "medium", "compact"), "medium");
        assert_eq!(compact_hint(60, "full", "medium", "compact"), "compact");
    }

    #[test]
    fn key_hint_height_is_single_line_when_hint_fits() {
        let height = key_hint_height(80, "Enter: continue    Esc: back");
        assert_eq!(height, 3);
    }

    #[test]
    fn key_hint_height_grows_when_hint_wraps() {
        let height = key_hint_height(20, "Enter: continue    Up/Down or j/k: move    Esc: back");
        assert!(height > 3);
    }

    #[test]
    fn focus_line_uses_blue_bold_style() {
        let line = focus_line("choose an option");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content.as_ref(), "choose an option");
        assert_eq!(line.spans[0].style.fg, Some(Color::Blue));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn label_value_line_formats_with_colon() {
        let line = label_value_line("tmux session name", "repo/w1");
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content.as_ref(), "tmux session name: ");
        assert_eq!(line.spans[1].content.as_ref(), "repo/w1");
    }

    #[test]
    fn wrapped_line_count_preserves_repeated_spaces() {
        assert_eq!(wrapped_line_count_single("a    b", 3), 2);
        assert_eq!(wrapped_line_count_single("a b", 3), 1);
    }

    #[test]
    fn highlighted_label_value_line_formats_single_focus_span() {
        let line = highlighted_label_value_line("Current selection", "Yes");
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content.as_ref(), "Current selection: ");
        assert_eq!(line.spans[1].content.as_ref(), "Yes");
        assert_eq!(line.spans[0].style.fg, Some(Color::Blue));
        assert!(
            line.spans[1]
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn yes_no_maps_boolean_values() {
        assert_eq!(yes_no(true), "Yes");
        assert_eq!(yes_no(false), "No");
    }

    #[test]
    fn result_footer_compacts_by_width() {
        assert_eq!(
            result_footer(120),
            "Enter/Esc: back to home    q: quit seshmux"
        );
        assert_eq!(result_footer(90), "Enter/Esc: home    q: quit");
        assert_eq!(result_footer(60), "Enter/Esc home | q quit");
    }
}
