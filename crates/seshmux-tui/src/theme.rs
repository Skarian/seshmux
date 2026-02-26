use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};

pub(crate) fn chrome<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    Block::default().borders(Borders::ALL).title(title)
}

pub(crate) fn key_block() -> Block<'static> {
    chrome("Keys")
}

pub(crate) fn table_header(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub(crate) fn table_highlight(color: Color) -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(color)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn focus_prompt() -> Style {
    Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn success_prompt() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn error_prompt() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub(crate) fn secondary_text() -> Style {
    Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
}
