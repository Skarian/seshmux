use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Clear;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::Line as TextLine,
};

use crate::centered_rect;
use crate::theme;
use crate::ui::text::{key_hint_height, key_hint_paragraph, wrapped_paragraph};

pub(crate) struct ModalSpec<'a> {
    pub(crate) title: &'a str,
    pub(crate) title_style: Option<Style>,
    pub(crate) body: Text<'a>,
    pub(crate) key_hint: Option<&'a str>,
    pub(crate) width_pct: u16,
    pub(crate) height_pct: u16,
}

pub(crate) struct ModalRenderResult {
    pub(crate) body_area: Rect,
}

pub(crate) fn render_modal(frame: &mut Frame<'_>, spec: ModalSpec<'_>) -> ModalRenderResult {
    let area = centered_rect(spec.width_pct, spec.height_pct, frame.area());
    let title = if let Some(style) = spec.title_style {
        Line::from(Span::styled(spec.title.to_string(), style))
    } else {
        Line::from(spec.title.to_string())
    };
    let mut body_area = area;
    let key_area = if let Some(key_hint) = spec.key_hint {
        let footer_height = key_hint_height(area.width, key_hint);
        choose_key_area(frame.area(), area, footer_height).or_else(|| {
            let [inner_body, inner_key] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(footer_height)])
                .areas(area);
            body_area = inner_body;
            Some(inner_key)
        })
    } else {
        None
    };

    frame.render_widget(Clear, body_area);
    frame.render_widget(
        wrapped_paragraph(spec.body).block(theme::chrome(title)),
        body_area,
    );

    if let (Some(key_hint), Some(key_area)) = (spec.key_hint, key_area) {
        frame.render_widget(Clear, key_area);
        frame.render_widget(
            key_hint_paragraph(key_hint).block(theme::key_block()),
            key_area,
        );
    }

    ModalRenderResult { body_area }
}

fn choose_key_area(screen: Rect, body: Rect, footer_height: u16) -> Option<Rect> {
    let screen_top = screen.y;
    let screen_bottom = screen.y.saturating_add(screen.height);
    let below_y = body.y.saturating_add(body.height);
    if below_y.saturating_add(footer_height) <= screen_bottom {
        return Some(Rect::new(body.x, below_y, body.width, footer_height));
    }

    let above_y = body.y.saturating_sub(footer_height);
    if above_y >= screen_top {
        return Some(Rect::new(body.x, above_y, body.width, footer_height));
    }

    None
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
            title_style: Some(crate::theme::error_prompt()),
            body: text_from_message(message),
            key_hint: Some(footer),
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
            title_style: Some(crate::theme::focus_prompt()),
            body: text_from_message(message),
            key_hint: Some(footer),
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
            title_style: Some(crate::theme::success_prompt()),
            body: text_from_message(message),
            key_hint: Some(footer),
            width_pct,
            height_pct,
        },
    );
}

fn text_from_message(message: &str) -> Text<'static> {
    let base = message.trim_end();
    let lines: Vec<TextLine<'static>> = if base.is_empty() {
        vec![TextLine::from("")]
    } else {
        base.lines()
            .map(|line| TextLine::from(line.to_string()))
            .collect()
    };
    Text::from(lines)
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::{choose_key_area, text_from_message};

    #[test]
    fn text_from_message_preserves_lines() {
        let text = text_from_message("hello\nworld");
        assert_eq!(text.lines.len(), 2);
        assert_eq!(text.lines[0].spans[0].content.as_ref(), "hello");
        assert_eq!(text.lines[1].spans[0].content.as_ref(), "world");
    }

    #[test]
    fn text_from_message_handles_empty_message() {
        let text = text_from_message("");
        assert_eq!(text.lines.len(), 1);
        assert!(text.lines[0].spans.is_empty());
    }

    #[test]
    fn choose_key_area_above_respects_non_zero_screen_origin() {
        let screen = Rect::new(10, 20, 80, 20);
        let body = Rect::new(15, 28, 60, 12);
        let key_area = choose_key_area(screen, body, 4);
        assert_eq!(key_area, Some(Rect::new(15, 24, 60, 4)));
    }

    #[test]
    fn choose_key_area_returns_none_when_no_space_outside_modal() {
        let screen = Rect::new(10, 20, 80, 10);
        let body = Rect::new(15, 22, 60, 8);
        let key_area = choose_key_area(screen, body, 3);
        assert_eq!(key_area, None);
    }
}
