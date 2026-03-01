use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, ScrollbarOrientation};
use tui_tree_widget::{Scrollbar as TreeScrollbar, Tree};

use super::picker::PickerState;
use super::{
    ExtrasIndexingPhase, NewFlow, NewFlowErrorOrigin, NewFlowErrorState, NewStartPoint,
    SkipModalState, Step,
};
use crate::theme;
use crate::ui::loading::render_loading_modal;
use crate::ui::modal::{ModalSpec, render_modal};
use crate::ui::text::{
    compact_hint, focus_line, highlighted_label_value_line, key_hint_height, key_hint_paragraph,
    label_value_line, result_footer, wrapped_paragraph, yes_no,
};

struct PickerRenderSpec<'a> {
    title: &'a str,
    filter_title: &'a str,
    empty_label: &'a str,
    highlight_color: Color,
}

impl NewFlow {
    pub(super) fn render(&self, frame: &mut ratatui::Frame<'_>) {
        match &self.step {
            Step::GitignoreDecision => self.render_gitignore_decision(frame),
            Step::NameInput => self.render_name_input(frame),
            Step::StartPointMode => self.render_start_mode(frame),
            Step::BranchPicker => self.render_branch_picker(frame),
            Step::CommitPicker => self.render_commit_picker(frame),
            Step::CopyExtrasDecision => self.render_copy_extras_decision(frame),
            Step::ExtrasIndexing => self.render_extras_indexing(frame),
            Step::ExtrasPicker => self.render_extras_picker(frame),
            Step::ConnectNow => self.render_connect_now(frame),
            Step::Review => self.render_review(frame),
            Step::Success => self.render_success(frame),
            Step::ErrorScreen(error) => self.render_error(frame, error),
        }
    }

    fn render_gitignore_decision(&self, frame: &mut ratatui::Frame<'_>) {
        let key_text = compact_hint(
            frame.area().width,
            "Space: toggle    Enter: continue    Esc: back",
            "Space: toggle    Enter: continue    Esc: back",
            "Space toggle | Enter continue | Esc back",
        );
        render_modal(
            frame,
            ModalSpec {
                title: "Add worktrees/ to .gitignore",
                title_style: Some(theme::focus_prompt()),
                body: Text::from(vec![
                    Line::from(""),
                    highlighted_label_value_line(
                        "Current Selection",
                        self.gitignore_choice.selected_label(),
                    ),
                ]),
                key_hint: Some(key_text),
                width_pct: 70,
                height_pct: 42,
            },
        );
    }

    fn render_name_input(&self, frame: &mut ratatui::Frame<'_>) {
        let key_text = compact_hint(
            frame.area().width,
            "Type to edit    Enter: continue    Backspace: delete    Esc: back",
            "Type    Enter: continue    Backspace: delete    Esc: back",
            "Type | Enter continue | Backspace delete | Esc back",
        );
        let rendered = render_modal(
            frame,
            ModalSpec {
                title: "New worktree name",
                title_style: Some(theme::focus_prompt()),
                body: Text::from(vec![Line::from("")]),
                key_hint: Some(key_text),
                width_pct: 72,
                height_pct: 44,
            },
        );

        let inner = rendered.body_area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let input_area = ratatui::layout::Rect::new(inner.x, inner.y, inner.width, 1);
        let width = input_area.width as usize;
        let scroll = self.name_input.visual_scroll(width);
        let input = Paragraph::new(self.name_input.value()).scroll((0, scroll as u16));
        frame.render_widget(input, input_area);

        if let Some(error) = &self.name_error {
            if inner.height > 1 {
                let error_area = ratatui::layout::Rect::new(
                    inner.x,
                    inner.y + 1,
                    inner.width,
                    inner.height.saturating_sub(1),
                );
                frame.render_widget(wrapped_paragraph(format!("Invalid: {error}")), error_area);
            }
        }

        if width > 0 {
            let visual = self.name_input.visual_cursor();
            let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
            frame.set_cursor_position((input_area.x + relative as u16, input_area.y));
        }
    }

    fn render_start_mode(&self, frame: &mut ratatui::Frame<'_>) {
        let key_text = compact_hint(
            frame.area().width,
            "Up/Down or j/k: move    Enter: select    Esc: back",
            "j/k: move    Enter: select    Esc: back",
            "j/k move | Enter select | Esc back",
        );
        let options = ["From current branch", "From other branch", "From commit"];
        let mut body_lines = Vec::with_capacity(options.len());
        for (index, option) in options.iter().enumerate() {
            let line = format!(
                "{} {option}",
                if self.start_mode_selected == index {
                    ">>"
                } else {
                    "  "
                }
            );
            if self.start_mode_selected == index {
                body_lines.push(Line::from(Span::styled(
                    line,
                    theme::table_highlight(Color::Green),
                )));
            } else {
                body_lines.push(Line::from(line));
            }
        }

        render_modal(
            frame,
            ModalSpec {
                title: "Choose how this worktree should start",
                title_style: Some(theme::focus_prompt()),
                body: Text::from(body_lines),
                key_hint: Some(key_text),
                width_pct: 74,
                height_pct: 46,
            },
        );
    }

    fn render_branch_picker(&self, frame: &mut ratatui::Frame<'_>) {
        render_searchable_picker_step(
            frame,
            PickerRenderSpec {
                title: "Choose branch",
                filter_title: "Filter branches",
                empty_label: "No branches found",
                highlight_color: Color::Yellow,
            },
            self.branch_picker.as_ref(),
            &self.branch_search_input,
            self.branch_filter_focused,
            |branch| branch.display.clone(),
        );
    }

    fn render_commit_picker(&self, frame: &mut ratatui::Frame<'_>) {
        render_searchable_picker_step(
            frame,
            PickerRenderSpec {
                title: "Choose commit",
                filter_title: "Filter commits",
                empty_label: "No commits found",
                highlight_color: Color::Magenta,
            },
            self.commit_picker.as_ref(),
            &self.commit_search_input,
            self.commit_filter_focused,
            |commit| commit.display.clone(),
        );
    }

    fn render_copy_extras_decision(&self, frame: &mut ratatui::Frame<'_>) {
        let key_text = compact_hint(
            frame.area().width,
            "Space: toggle    Enter: continue    Esc: back",
            "Space toggle    Enter continue    Esc back",
            "Space toggle | Enter continue | Esc back",
        );
        let body = Text::from(vec![
            Line::from(""),
            highlighted_label_value_line(
                "Current Selection",
                self.copy_extras_choice.selected_label(),
            ),
        ]);
        render_modal(
            frame,
            ModalSpec {
                title: "Copy untracked / gitignored files?",
                title_style: Some(theme::focus_prompt()),
                body,
                key_hint: Some(key_text),
                width_pct: 74,
                height_pct: 44,
            },
        );
    }

    fn render_extras_indexing(&self, frame: &mut ratatui::Frame<'_>) {
        let Some(indexing) = &self.extras_indexing else {
            return;
        };
        let (message, key_hint) = match indexing.phase {
            ExtrasIndexingPhase::Collecting => {
                ("Collecting extras list from git".to_string(), "Esc: back")
            }
            ExtrasIndexingPhase::Classifying { candidate_count } => (
                format!("Classifying {candidate_count} candidates"),
                "Esc: back",
            ),
            ExtrasIndexingPhase::AwaitingSkipDecision {
                flagged_bucket_count,
            } => (
                format!("Awaiting skip decision ({flagged_bucket_count} buckets)"),
                "Esc: cancel",
            ),
            ExtrasIndexingPhase::Building { filtered_count } => (
                format!("Building extras index ({filtered_count} candidates)"),
                "Esc: back",
            ),
        };
        render_loading_modal(
            frame,
            "Preparing extras",
            &message,
            key_hint,
            &indexing.loading,
        );

        if let Some(skip_modal) = &indexing.skip_modal {
            self.render_skip_modal(frame, skip_modal);
        }
    }

    fn render_skip_modal(&self, frame: &mut ratatui::Frame<'_>, modal: &SkipModalState) {
        let key_text = compact_hint(
            frame.area().width,
            "Up/Down: move    Space: toggle skip    a: toggle persist    Enter: confirm    Esc: cancel (set as always in config is fixed)",
            "Up/Down move    Space toggle    a persist    Enter confirm    Esc cancel    config-always fixed",
            "Up/Down | Space toggle | a persist | Enter confirm | Esc cancel | config-always fixed",
        );

        let rendered = render_modal(
            frame,
            ModalSpec {
                title: "Skip large buckets?",
                title_style: Some(theme::focus_prompt()),
                body: Text::from(vec![Line::from("")]),
                key_hint: Some(key_text),
                width_pct: 86,
                height_pct: 72,
            },
        );

        let inner = rendered.body_area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        if inner.width == 0 || inner.height < 4 {
            return;
        }

        let [header_area, list_area, footer_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .areas(inner);

        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from("Large artifact buckets detected. Choose what to skip for this run."),
                Line::from(""),
            ])),
            header_area,
        );

        let visible_rows = list_area.height as usize;
        if visible_rows > 0 {
            let total = modal.choices.len();
            let mut window_start = modal.selected.saturating_sub(visible_rows / 2);
            let max_start = total.saturating_sub(visible_rows);
            if window_start > max_start {
                window_start = max_start;
            }
            let window_end = (window_start + visible_rows).min(total);

            let mut list_lines = Vec::<Line<'_>>::new();
            for (index, choice) in modal.choices[window_start..window_end].iter().enumerate() {
                let absolute_index = window_start + index;
                let marker = if absolute_index == modal.selected {
                    ">>"
                } else {
                    "  "
                };
                let selected = if choice.skip { "Yes" } else { "No" };
                let config_note = if choice.locked_in_config {
                    " (set as always in config)"
                } else {
                    ""
                };
                let line = format!(
                    "{marker} skip={selected:>3}  {}{}  ({} files)",
                    choice.bucket, config_note, choice.count
                );
                if absolute_index == modal.selected {
                    list_lines.push(Line::from(Span::styled(
                        line,
                        theme::table_highlight(Color::Cyan),
                    )));
                } else {
                    list_lines.push(Line::from(line));
                }
            }
            frame.render_widget(Paragraph::new(Text::from(list_lines)), list_area);
        }

        frame.render_widget(
            Paragraph::new(format!(
                "Always skip selected buckets in this repo: {} (a to toggle)",
                yes_no(modal.persist_always_skip),
            )),
            footer_area,
        );
    }

    fn render_extras_picker(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let key_label = if self.extras.editing_filter {
            compact_hint(
                area.width,
                "Type: filter    Backspace: delete    /: list focus    Esc: back",
                "Type filter    Backspace delete    /: list    Esc: back",
                "Type filter | Backspace | / list | Esc back",
            )
        } else {
            compact_hint(
                area.width,
                "Up/Down or j/k: move    Tab: fold/unfold    Space: toggle    Enter: continue    a: all    n: none    /: filter    Esc: back",
                "j/k: move    Tab: fold    Space: toggle    Enter: continue    a: all    n: none    /: filter    Esc: back",
                "j/k move | Tab fold | Space toggle | Enter continue | a all | n none | / filter | Esc back",
            )
        };
        let footer_height = key_hint_height(area.width, key_label);
        let [filter_area, body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(9),
                Constraint::Length(footer_height),
            ])
            .areas(area);

        let width = filter_area.width.saturating_sub(2) as usize;
        let scroll = self.extras.filter.visual_scroll(width);
        let filter = Paragraph::new(self.extras.filter.value())
            .scroll((0, scroll as u16))
            .block(theme::chrome(if self.extras.editing_filter {
                focus_line("Filter extras")
            } else {
                Line::from("Filter extras (/ to focus)")
            }));
        frame.render_widget(filter, filter_area);
        if self.extras.editing_filter && width > 0 {
            let visual = self.extras.filter.visual_cursor();
            let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
            frame.set_cursor_position((filter_area.x + 1 + relative as u16, filter_area.y + 1));
        }

        let extras_title = if self.extras.editing_filter {
            Line::from("Select untracked / gitignored extras (/ to focus)")
        } else {
            focus_line("Select untracked / gitignored extras")
        };
        let items = self.extras.tree_items();
        if items.is_empty() {
            frame.render_widget(
                Paragraph::new("No untracked or gitignored files/folders were found.")
                    .wrap(ratatui::widgets::Wrap { trim: false })
                    .block(theme::chrome(extras_title.clone())),
                body,
            );
        } else {
            let mut state = self.extras.tree_state();
            let tree = Tree::new(&items)
                .expect("all extra tree identifiers are unique")
                .block(theme::chrome(extras_title))
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

        let keys = key_hint_paragraph(key_label).block(theme::key_block());
        frame.render_widget(keys, footer);
    }

    fn render_connect_now(&self, frame: &mut ratatui::Frame<'_>) {
        let key_text = compact_hint(
            frame.area().width,
            "Space: toggle    Enter: continue    Esc: back",
            "Space toggle    Enter continue    Esc back",
            "Space toggle | Enter continue | Esc back",
        );
        let body = Text::from(vec![highlighted_label_value_line(
            "Current Selection",
            self.connect_choice.selected_label(),
        )]);
        render_modal(
            frame,
            ModalSpec {
                title: "Attach to the tmux session now?",
                title_style: Some(theme::focus_prompt()),
                body,
                key_hint: Some(key_text),
                width_pct: 70,
                height_pct: 42,
            },
        );
    }

    fn render_review(&self, frame: &mut ratatui::Frame<'_>) {
        let key_text = compact_hint(
            frame.area().width,
            "Enter: create worktree    Esc: back",
            "Enter: create    Esc: back",
            "Enter create | Esc back",
        );

        let start_point = match &self.start_point {
            Some(NewStartPoint::CurrentBranch) => "Current branch".to_string(),
            Some(NewStartPoint::Branch(name)) => format!("Branch: {name}"),
            Some(NewStartPoint::Commit(hash)) => format!("Commit: {hash}"),
            None => "UNCONFIRMED".to_string(),
        };

        let extras_count = self.review_selected_extras_count();
        let review = Text::from(vec![
            label_value_line("Worktree name", self.name_input.value()),
            label_value_line("Start from", start_point),
            label_value_line(
                "Add worktrees/ to .gitignore",
                yes_no(
                    !self.prepare.gitignore_has_worktrees_entry
                        && self.gitignore_choice.yes_selected,
                ),
            ),
            label_value_line(
                "Copy untracked / gitignored files",
                yes_no(self.copy_extras_choice.yes_selected),
            ),
            label_value_line(
                "Untracked / gitignored files selected",
                extras_count.to_string(),
            ),
            label_value_line(
                "Connect to tmux now",
                yes_no(self.connect_choice.yes_selected),
            ),
        ]);
        render_modal(
            frame,
            ModalSpec {
                title: "Confirm settings before creating the worktree",
                title_style: Some(theme::focus_prompt()),
                body: review,
                key_hint: Some(key_text),
                width_pct: 82,
                height_pct: 62,
            },
        );
    }

    fn render_success(&self, frame: &mut ratatui::Frame<'_>) {
        let footer = result_footer(frame.area().width);
        let success = if let Some(result) = &self.success {
            let mut lines = vec![
                label_value_line("Worktree path", result.worktree_path.display().to_string()),
                label_value_line("tmux session name", result.session_name.clone()),
                label_value_line("Attach command", result.attach_command.clone()),
                label_value_line("Connected in this terminal", yes_no(result.connected_now)),
            ];
            if let Some(notice) = &self.success_notice {
                lines.push(Line::from(""));
                lines.push(label_value_line("Notice", notice.clone()));
            }
            Text::from(lines)
        } else {
            Text::from(vec![Line::from("")])
        };

        render_modal(
            frame,
            ModalSpec {
                title: "Success",
                title_style: Some(theme::success_prompt()),
                body: success,
                key_hint: Some(footer),
                width_pct: 80,
                height_pct: 60,
            },
        );
    }

    fn render_error(&self, frame: &mut ratatui::Frame<'_>, error: &NewFlowErrorState) {
        let headline = match error.origin {
            NewFlowErrorOrigin::ExtrasIndexing => "Failed to prepare extras selection",
            NewFlowErrorOrigin::ReviewSubmit => "Failed to create worktree",
        };
        let body = Text::from(vec![
            Line::from(headline),
            Line::from(""),
            Line::from(error.message.clone()),
        ]);
        render_modal(
            frame,
            ModalSpec {
                title: "Error",
                title_style: Some(theme::error_prompt()),
                body,
                key_hint: Some("Enter/Esc: back"),
                width_pct: 85,
                height_pct: 70,
            },
        );
    }
}

fn render_searchable_picker_step<T, F>(
    frame: &mut ratatui::Frame<'_>,
    spec: PickerRenderSpec<'_>,
    picker: Option<&PickerState<T>>,
    filter_input: &tui_input::Input,
    filter_focused: bool,
    item_label: F,
) where
    F: Fn(&T) -> String,
{
    let area = frame.area();
    let key_text = if filter_focused {
        compact_hint(
            area.width,
            "Type: filter    Backspace: delete    /: list focus    Esc: back",
            "Type filter    Backspace delete    /: list    Esc: back",
            "Type filter | Backspace | / list | Esc back",
        )
    } else {
        compact_hint(
            area.width,
            "/: filter focus    Enter: choose    Up/Down or j/k: move    Esc: back",
            "/: filter    Enter: choose    j/k: move    Esc: back",
            "/ filter | Enter choose | j/k move | Esc back",
        )
    };
    let footer_height = key_hint_height(area.width, key_text);
    let [filter_area, body, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(footer_height),
        ])
        .areas(area);

    let width = filter_area.width.saturating_sub(2) as usize;
    let scroll = filter_input.visual_scroll(width);
    let filter_title = if filter_focused {
        focus_line(spec.filter_title)
    } else {
        Line::from(format!("{} (/ to focus)", spec.filter_title))
    };
    let filter = Paragraph::new(filter_input.value())
        .scroll((0, scroll as u16))
        .block(theme::chrome(filter_title));
    frame.render_widget(filter, filter_area);
    if filter_focused && width > 0 {
        let visual = filter_input.visual_cursor();
        let relative = visual.saturating_sub(scroll).min(width.saturating_sub(1));
        frame.set_cursor_position((filter_area.x + 1 + relative as u16, filter_area.y + 1));
    }

    let list_title = if filter_focused {
        Line::from(format!("{} (/ to focus)", spec.title))
    } else {
        focus_line(spec.title)
    };

    let mut rows = Vec::new();
    if let Some(picker) = picker {
        rows.extend(
            picker
                .items
                .iter()
                .map(|item| ListItem::new(item_label(item))),
        );
    }

    if rows.is_empty() {
        frame.render_widget(
            wrapped_paragraph(spec.empty_label).block(theme::chrome(list_title)),
            body,
        );
    } else {
        let list = List::new(rows)
            .block(theme::chrome(list_title))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(spec.highlight_color)
                    .add_modifier(Modifier::BOLD),
            );
        let mut state = ListState::default();
        state.select(Some(picker.map(|value| value.selected).unwrap_or(0)));
        frame.render_stateful_widget(list, body, &mut state);
    }

    let keys = key_hint_paragraph(key_text).block(theme::key_block());
    frame.render_widget(keys, footer);
}
