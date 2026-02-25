use ratatui::Frame;
use ratatui::widgets::Clear;

use crate::centered_rect;
use crate::theme;
use crate::ui::text::wrapped_paragraph;

pub(crate) struct ModalSpec<'a> {
    pub(crate) title: &'a str,
    pub(crate) body: String,
    pub(crate) width_pct: u16,
    pub(crate) height_pct: u16,
}

pub(crate) fn render_modal(frame: &mut Frame<'_>, spec: ModalSpec<'_>) {
    let area = centered_rect(spec.width_pct, spec.height_pct, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        wrapped_paragraph(spec.body).block(theme::chrome(spec.title)),
        area,
    );
}

pub(crate) fn render_error_modal(
    frame: &mut Frame<'_>,
    message: &str,
    width_pct: u16,
    height_pct: u16,
    footer: &str,
) {
    render_modal(
        frame,
        ModalSpec {
            title: "Error",
            body: with_footer(message, footer),
            width_pct,
            height_pct,
        },
    );
}

pub(crate) fn render_notice_modal(
    frame: &mut Frame<'_>,
    title: &str,
    message: &str,
    width_pct: u16,
    height_pct: u16,
    footer: &str,
) {
    render_modal(
        frame,
        ModalSpec {
            title,
            body: with_footer(message, footer),
            width_pct,
            height_pct,
        },
    );
}

pub(crate) fn render_success_modal(
    frame: &mut Frame<'_>,
    message: &str,
    width_pct: u16,
    height_pct: u16,
    footer: &str,
) {
    render_modal(
        frame,
        ModalSpec {
            title: "Success",
            body: with_footer(message, footer),
            width_pct,
            height_pct,
        },
    );
}

fn with_footer(message: &str, footer: &str) -> String {
    let base = message.trim_end();
    if base.is_empty() {
        footer.to_string()
    } else {
        format!("{base}\n\n{footer}")
    }
}

#[cfg(test)]
mod tests {
    use super::with_footer;

    #[test]
    fn with_footer_separates_message_and_footer() {
        assert_eq!(
            with_footer("hello", "Enter/Esc to continue."),
            "hello\n\nEnter/Esc to continue."
        );
    }

    #[test]
    fn with_footer_handles_empty_message() {
        assert_eq!(with_footer("", "footer"), "footer");
    }
}
