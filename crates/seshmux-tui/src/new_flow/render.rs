use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, ListState, Paragraph, ScrollbarOrientation};
use tui_tree_widget::{Scrollbar as TreeScrollbar, Tree};

use super::picker::PickerState;
use super::{NewFlow, NewStartPoint, Step};
use crate::theme;
use crate::ui::modal::{ModalSpec, render_error_modal, render_modal};
use crate::ui::text::{compact_hint, wrapped_paragraph};

struct PickerRenderSpec<'a> {
    title: &'a str,
    search_label: &'a str,
    show_all_label: &'a str,
    empty_label: &'a str,
    highlight_color: Color,
}

impl NewFlow {
    pub(super) fn render(&self, frame: &mut ratatui::Frame<'_>) {
        match self.step {
            Step::GitignoreDecision => self.render_gitignore_decision(frame),
            Step::NameInput => self.render_name_input(frame),
            Step::StartPointMode => self.render_start_mode(frame),
            Step::BranchPicker => self.render_branch_picker(frame),
            Step::BranchSearchInput => self.render_branch_search_input(frame),
            Step::CommitPicker => self.render_commit_picker(frame),
            Step::CommitSearchInput => self.render_commit_search_input(frame),
            Step::ExtrasPicker => self.render_extras_picker(frame),
            Step::ConnectNow => self.render_connect_now(frame),
            Step::Review => self.render_review(frame),
            Step::Success => self.render_success(frame),
            Step::ErrorScreen => self.render_error(frame),
        }
    }

    fn render_gitignore_decision(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(3)])
            .areas(area);

        let prompt = wrapped_paragraph(format!(
            "Add 'worktrees/' to this repo's .gitignore?\n\nSelection: {}\nUse Space to toggle and Enter to continue.",
            self.gitignore_choice.selected_label()
        ))
        .block(theme::chrome("New: .gitignore"));
        frame.render_widget(prompt, body);

        let keys = wrapped_paragraph(compact_hint(
            area.width,
            "Space: toggle    Enter: continue    Esc: exit flow",
            "Space: toggle    Enter: continue    Esc: back",
            "Space toggle | Enter continue | Esc back",
        ))
        .block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_name_input(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(3)])
            .areas(area);

        let [input_area, info_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(4)])
            .areas(body);

        let width = input_area.width.saturating_sub(2) as usize;
        let scroll = self.name_input.visual_scroll(width);
        let input = Paragraph::new(self.name_input.value())
            .scroll((0, scroll as u16))
            .block(theme::chrome("New: Name"));
        frame.render_widget(input, input_area);

        if width > 0 {
            let visual = self.name_input.visual_cursor();
            let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
            frame.set_cursor_position((input_area.x + 1 + relative as u16, input_area.y + 1));
        }

        let mut details = vec![Line::from("Rule: ^[a-z0-9][a-z0-9_-]{0,47}$")];
        if let Some(error) = &self.name_error {
            details.push(Line::from(""));
            details.push(Line::from(format!("Invalid: {error}")));
        }
        let info = Paragraph::new(details).block(theme::chrome("Name validation"));
        frame.render_widget(info, info_area);

        let keys = wrapped_paragraph(compact_hint(
            area.width,
            "Type to edit    Enter: continue    Backspace: delete    Esc: back",
            "Type    Enter: continue    Backspace: delete    Esc: back",
            "Type | Enter continue | Backspace delete | Esc back",
        ))
        .block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_start_mode(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        let items = vec![
            ListItem::new("From current branch"),
            ListItem::new("From other branch"),
            ListItem::new("From commit"),
        ];
        let list = List::new(items)
            .block(theme::chrome("New: Start point"))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            );

        let mut state = ListState::default();
        state.select(Some(self.start_mode_selected));
        frame.render_stateful_widget(list, body, &mut state);

        let keys = wrapped_paragraph(compact_hint(
            area.width,
            "Up/Down or j/k: move    Enter: select    Esc: back",
            "j/k: move    Enter: select    Esc: back",
            "j/k move | Enter select | Esc back",
        ))
        .block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_branch_picker(&self, frame: &mut ratatui::Frame<'_>) {
        render_picker_step(
            frame,
            PickerRenderSpec {
                title: "New: Branch picker",
                search_label: "Search branches...",
                show_all_label: "Show all branches",
                empty_label: "No branches found",
                highlight_color: Color::Yellow,
            },
            self.branch_picker.as_ref(),
            |branch| branch.display.clone(),
        );
    }

    fn render_branch_search_input(&self, frame: &mut ratatui::Frame<'_>) {
        render_search_input_step(
            frame,
            "New: Branch search",
            "Enter branch filter and press Enter.",
            &self.branch_search_input,
        );
    }

    fn render_commit_picker(&self, frame: &mut ratatui::Frame<'_>) {
        render_picker_step(
            frame,
            PickerRenderSpec {
                title: "New: Commit picker",
                search_label: "Search commits by hash...",
                show_all_label: "Show latest 50 commits",
                empty_label: "No commits found",
                highlight_color: Color::Magenta,
            },
            self.commit_picker.as_ref(),
            |commit| commit.display.clone(),
        );
    }

    fn render_commit_search_input(&self, frame: &mut ratatui::Frame<'_>) {
        render_search_input_step(
            frame,
            "New: Commit search",
            "Enter commit hash filter and press Enter.",
            &self.commit_search_input,
        );
    }

    fn render_extras_picker(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [filter_area, body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(9),
                Constraint::Length(3),
            ])
            .areas(area);

        let width = filter_area.width.saturating_sub(2) as usize;
        let scroll = self.extras.filter.visual_scroll(width);
        let filter = Paragraph::new(self.extras.filter.value())
            .scroll((0, scroll as u16))
            .block(theme::chrome("Extras filter"));
        frame.render_widget(filter, filter_area);
        if self.extras.editing_filter && width > 0 {
            let visual = self.extras.filter.visual_cursor();
            let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
            frame.set_cursor_position((filter_area.x + 1 + relative as u16, filter_area.y + 1));
        }

        let items = self.extras.tree_items();
        if items.is_empty() {
            frame.render_widget(
                Paragraph::new("No extra files or directories were found.")
                    .wrap(ratatui::widgets::Wrap { trim: false })
                    .block(theme::chrome("New: Extras")),
                body,
            );
        } else {
            let mut state = self.extras.tree_state();
            let tree = Tree::new(&items)
                .expect("all extra tree identifiers are unique")
                .block(theme::chrome("New: Extras"))
                .experimental_scrollbar(Some(
                    TreeScrollbar::new(ScrollbarOrientation::VerticalRight)
                        .begin_symbol(None)
                        .end_symbol(None),
                ))
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(">> ");
            frame.render_stateful_widget(tree, body, &mut state);
        }

        let key_label = if self.extras.editing_filter {
            compact_hint(
                area.width,
                "Type: filter    Enter/Esc: finish filter edit",
                "Type filter    Enter/Esc: finish filter",
                "Type filter | Enter/Esc finish",
            )
        } else {
            compact_hint(
                area.width,
                "Up/Down or j/k: move    Tab: fold/unfold    Space: toggle    Enter: continue    a: all    n: none    /: filter    Esc: back",
                "j/k: move    Tab: fold    Space: toggle    Enter: continue    a: all    n: none    /: filter    Esc: back",
                "j/k move | Tab fold | Space toggle | Enter continue | a all | n none | / filter | Esc back",
            )
        };
        let keys = wrapped_paragraph(key_label).block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_connect_now(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(3)])
            .areas(area);

        let paragraph = wrapped_paragraph(format!(
            "Connect to the tmux session now?\n\nSelection: {}",
            self.connect_choice.selected_label()
        ))
        .block(theme::chrome("New: Connect now"));
        frame.render_widget(paragraph, body);

        let keys = wrapped_paragraph(compact_hint(
            area.width,
            "Space: toggle    Enter: continue    Esc: back",
            "Space toggle    Enter continue    Esc back",
            "Space toggle | Enter continue | Esc back",
        ))
        .block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_review(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        let start_point = match &self.start_point {
            Some(NewStartPoint::CurrentBranch) => "From current branch".to_string(),
            Some(NewStartPoint::Branch(name)) => format!("From branch: {name}"),
            Some(NewStartPoint::Commit(hash)) => format!("From commit: {hash}"),
            None => "UNCONFIRMED".to_string(),
        };

        let extras_count = self.extras.selected_for_copy().len();
        let review = wrapped_paragraph(format!(
            "Review before create:\n\nworktree: {}\nstart point: {}\nadd .gitignore entry: {}\nselected extras: {}\nconnect now: {}\n",
            self.name_input.value(),
            start_point,
            (!self.prepare.gitignore_has_worktrees_entry && self.gitignore_choice.yes_selected),
            extras_count,
            self.connect_choice.yes_selected
        ))
        .block(theme::chrome("New: Review"));
        frame.render_widget(review, body);

        let keys = wrapped_paragraph(compact_hint(
            area.width,
            "Enter: create worktree    Esc: back",
            "Enter: create    Esc: back",
            "Enter create | Esc back",
        ))
        .block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_success(&self, frame: &mut ratatui::Frame<'_>) {
        let success = if let Some(result) = &self.success {
            format!(
                "Worktree created.\n\nPath: {}\nSession: {}\nAttach: {}\nConnected now: {}\n\nEnter/Esc to exit.",
                result.worktree_path.display(),
                result.session_name,
                result.attach_command,
                result.connected_now
            )
        } else {
            "Worktree created.\n\nEnter/Esc to exit.".to_string()
        };

        render_modal(
            frame,
            ModalSpec {
                title: "Success",
                body: success,
                width_pct: 80,
                height_pct: 60,
            },
        );
    }

    fn render_error(&self, frame: &mut ratatui::Frame<'_>) {
        let message = self
            .error_message
            .as_deref()
            .unwrap_or("Unknown error while creating worktree.");
        let body = format!("Failed to create worktree.\n\n{message}");
        render_error_modal(frame, &body, 85, 70, "Enter/Esc to return to review.");
    }
}

fn render_picker_step<T, F>(
    frame: &mut ratatui::Frame<'_>,
    spec: PickerRenderSpec<'_>,
    picker: Option<&PickerState<T>>,
    item_label: F,
) where
    F: Fn(&T) -> String,
{
    let area = frame.area();
    let [body, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(3)])
        .areas(area);

    let mut rows = vec![ListItem::new(spec.search_label)];
    if let Some(picker) = picker {
        if picker.query.is_some() {
            rows.push(ListItem::new(spec.show_all_label));
        }
        rows.extend(
            picker
                .items
                .iter()
                .map(|item| ListItem::new(item_label(item))),
        );
    }

    if rows.len() == 1 {
        rows.push(ListItem::new(spec.empty_label));
    }

    let list = List::new(rows)
        .block(theme::chrome(spec.title))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(spec.highlight_color)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    state.select(Some(picker.map(|value| value.selected).unwrap_or(0)));
    frame.render_stateful_widget(list, body, &mut state);

    let keys = wrapped_paragraph(compact_hint(
        area.width,
        "Enter: choose    Up/Down or j/k: move    Esc: back",
        "Enter: choose    j/k: move    Esc: back",
        "Enter choose | j/k move | Esc back",
    ))
    .block(theme::key_block());
    frame.render_widget(keys, footer);
}

fn render_search_input_step(
    frame: &mut ratatui::Frame<'_>,
    title: &str,
    prompt: &str,
    input_state: &tui_input::Input,
) {
    let area = frame.area();
    let [body, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
        .areas(area);

    let [prompt_area, input_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .areas(body);
    frame.render_widget(
        wrapped_paragraph(prompt).block(theme::chrome(title)),
        prompt_area,
    );

    let width = input_area.width.saturating_sub(2) as usize;
    let scroll = input_state.visual_scroll(width);
    let input = Paragraph::new(input_state.value())
        .scroll((0, scroll as u16))
        .block(theme::chrome("Filter"));
    frame.render_widget(input, input_area);
    if width > 0 {
        let visual = input_state.visual_cursor();
        let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
        frame.set_cursor_position((input_area.x + 1 + relative as u16, input_area.y + 1));
    }

    let keys = wrapped_paragraph(compact_hint(
        area.width,
        "Type: filter    Enter: apply    Backspace: delete    Esc: back",
        "Type filter    Enter apply    Backspace delete    Esc back",
        "Type | Enter apply | Backspace | Esc back",
    ))
    .block(theme::key_block());
    frame.render_widget(keys, footer);
}
