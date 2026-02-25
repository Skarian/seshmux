use anyhow::{Context, Result};
use crossterm::event::{Event, KeyEvent};
use tui_input::backend::crossterm::EventHandler;

use crate::UiExit;
use crate::keymap;
use crate::ui::binary_choice::BinaryChoiceEvent;

use super::picker::{PickerAction, PickerState};
use super::{FlowSignal, NewFlow, NewFlowOps, Step};
use seshmux_app::{NewRequest, NewStartPoint};

enum PickerInputEvent {
    Back,
    Continue,
    Enter(PickerAction),
}

enum SearchInputEvent {
    Back,
    Continue,
    Submit(String),
}

fn handle_picker_input<T>(picker: &mut PickerState<T>, key: KeyEvent) -> PickerInputEvent {
    if keymap::is_back(key) {
        return PickerInputEvent::Back;
    }

    if keymap::is_up(key) {
        picker.move_up();
        return PickerInputEvent::Continue;
    }

    if keymap::is_down(key) {
        picker.move_down();
        return PickerInputEvent::Continue;
    }

    if keymap::is_confirm(key) {
        return PickerInputEvent::Enter(picker.on_enter());
    }

    PickerInputEvent::Continue
}

fn handle_search_input(input: &mut tui_input::Input, key: KeyEvent) -> SearchInputEvent {
    if keymap::is_back(key) {
        return SearchInputEvent::Back;
    }

    if keymap::is_confirm(key) {
        return SearchInputEvent::Submit(input.value().trim().to_string());
    }

    let _ = input.handle_event(&Event::Key(key));
    SearchInputEvent::Continue
}

impl NewFlow {
    pub(super) fn on_key(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        match self.step {
            Step::GitignoreDecision => self.on_key_gitignore(key),
            Step::NameInput => self.on_key_name(key),
            Step::StartPointMode => self.on_key_start_mode(key, ops),
            Step::BranchPicker => self.on_key_branch_picker(key, ops),
            Step::BranchSearchInput => self.on_key_branch_search_input(key, ops),
            Step::CommitPicker => self.on_key_commit_picker(key, ops),
            Step::CommitSearchInput => self.on_key_commit_search_input(key, ops),
            Step::ExtrasPicker => self.on_key_extras(key),
            Step::ConnectNow => self.on_key_connect_now(key),
            Step::Review => self.on_key_review(key, ops),
            Step::Success => self.on_key_success(key),
            Step::ErrorScreen => self.on_key_error(key),
        }
    }

    fn on_key_gitignore(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match self.gitignore_choice.on_key(key) {
            BinaryChoiceEvent::Back => Ok(FlowSignal::Exit(UiExit::BackAtRoot)),
            BinaryChoiceEvent::Continue => Ok(FlowSignal::Continue),
            BinaryChoiceEvent::ConfirmYes | BinaryChoiceEvent::ConfirmNo => {
                self.step = Step::NameInput;
                Ok(FlowSignal::Continue)
            }
        }
    }

    fn on_key_name(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if keymap::is_back(key) {
            if self.prepare.gitignore_has_worktrees_entry {
                return Ok(FlowSignal::Exit(UiExit::BackAtRoot));
            }
            self.step = Step::GitignoreDecision;
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_confirm(key) {
            let candidate = self.name_input.value().trim().to_string();
            match seshmux_core::names::validate_worktree_name(&candidate) {
                Ok(()) => {
                    self.name_input = tui_input::Input::new(candidate);
                    self.name_error = None;
                    self.step = Step::StartPointMode;
                }
                Err(error) => {
                    self.name_error = Some(error.to_string());
                }
            }
            return Ok(FlowSignal::Continue);
        }

        if self.name_input.handle_event(&Event::Key(key)).is_some() {
            self.name_error = None;
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_start_mode(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        if keymap::is_back(key) {
            self.step = Step::NameInput;
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_up(key) {
            self.start_mode_selected = self.start_mode_selected.saturating_sub(1);
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_down(key) {
            if self.start_mode_selected < 2 {
                self.start_mode_selected += 1;
            }
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_confirm(key) {
            match self.start_mode_selected {
                0 => {
                    self.start_point = Some(NewStartPoint::CurrentBranch);
                    self.step = Step::ExtrasPicker;
                }
                1 => {
                    self.branch_picker = Some(self.load_branches(ops, "")?);
                    self.step = Step::BranchPicker;
                }
                _ => {
                    self.commit_picker = Some(self.load_commits(ops, "")?);
                    self.step = Step::CommitPicker;
                }
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_branch_picker(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        let picker = self.branch_picker.take();
        let (signal, picker) = self.on_key_picker_generic(
            key,
            ops,
            picker,
            |flow, ops, query| flow.load_branches(ops, query),
            |flow, query| {
                flow.branch_search_input = tui_input::Input::new(query);
                flow.step = Step::BranchSearchInput;
            },
            |flow, branch: &seshmux_core::git::BranchRef| {
                flow.start_point = Some(NewStartPoint::Branch(branch.name.clone()));
                flow.step = Step::ExtrasPicker;
            },
        )?;
        self.branch_picker = picker;
        Ok(signal)
    }

    fn on_key_branch_search_input(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
    ) -> Result<FlowSignal> {
        let mut input = std::mem::take(&mut self.branch_search_input);
        let signal = self.on_key_search_input_generic(
            key,
            ops,
            &mut input,
            Step::BranchPicker,
            |flow, ops, query| {
                flow.branch_picker = Some(flow.load_branches(ops, query)?);
                Ok(())
            },
        );
        self.branch_search_input = input;
        signal
    }

    fn on_key_commit_picker(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        let picker = self.commit_picker.take();
        let (signal, picker) = self.on_key_picker_generic(
            key,
            ops,
            picker,
            |flow, ops, query| flow.load_commits(ops, query),
            |flow, query| {
                flow.commit_search_input = tui_input::Input::new(query);
                flow.step = Step::CommitSearchInput;
            },
            |flow, commit: &seshmux_core::git::CommitRef| {
                flow.start_point = Some(NewStartPoint::Commit(commit.hash.clone()));
                flow.step = Step::ExtrasPicker;
            },
        )?;
        self.commit_picker = picker;
        Ok(signal)
    }

    fn on_key_commit_search_input(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
    ) -> Result<FlowSignal> {
        let mut input = std::mem::take(&mut self.commit_search_input);
        let signal = self.on_key_search_input_generic(
            key,
            ops,
            &mut input,
            Step::CommitPicker,
            |flow, ops, query| {
                flow.commit_picker = Some(flow.load_commits(ops, query)?);
                Ok(())
            },
        );
        self.commit_search_input = input;
        signal
    }

    fn on_key_picker_generic<T, FLoad, FOpenSearch, FPick>(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
        picker: Option<PickerState<T>>,
        mut load_picker: FLoad,
        mut open_search: FOpenSearch,
        mut pick_item: FPick,
    ) -> Result<(FlowSignal, Option<PickerState<T>>)>
    where
        FLoad: FnMut(&mut Self, &dyn NewFlowOps, &str) -> Result<PickerState<T>>,
        FOpenSearch: FnMut(&mut Self, String),
        FPick: FnMut(&mut Self, &T),
    {
        let mut picker = match picker {
            Some(value) => value,
            None => {
                return Ok((FlowSignal::Continue, Some(load_picker(self, ops, "")?)));
            }
        };

        match handle_picker_input(&mut picker, key) {
            PickerInputEvent::Back => {
                self.step = Step::StartPointMode;
            }
            PickerInputEvent::Continue => {}
            PickerInputEvent::Enter(action) => match action {
                PickerAction::OpenSearch => {
                    open_search(self, picker.query.clone().unwrap_or_default());
                }
                PickerAction::ShowAll => {
                    return Ok((FlowSignal::Continue, Some(load_picker(self, ops, "")?)));
                }
                PickerAction::Pick(index) => {
                    if let Some(item) = picker.items.get(index) {
                        pick_item(self, item);
                    }
                }
                PickerAction::Noop => {}
            },
        }

        Ok((FlowSignal::Continue, Some(picker)))
    }

    fn on_key_search_input_generic<FLoad>(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
        input: &mut tui_input::Input,
        picker_step: Step,
        mut load_picker: FLoad,
    ) -> Result<FlowSignal>
    where
        FLoad: FnMut(&mut Self, &dyn NewFlowOps, &str) -> Result<()>,
    {
        match handle_search_input(input, key) {
            SearchInputEvent::Back => {
                self.step = picker_step;
            }
            SearchInputEvent::Continue => {}
            SearchInputEvent::Submit(query) => {
                load_picker(self, ops, &query)?;
                self.step = picker_step;
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_extras(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if self.extras.editing_filter {
            if keymap::is_back(key) || keymap::is_confirm(key) {
                self.extras.toggle_filter_editing();
            } else {
                self.extras.edit_filter(key);
            }
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_back(key) {
            self.step = match self.start_point {
                Some(NewStartPoint::CurrentBranch) => Step::StartPointMode,
                Some(NewStartPoint::Branch(_)) => Step::BranchPicker,
                Some(NewStartPoint::Commit(_)) => Step::CommitPicker,
                None => Step::StartPointMode,
            };
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_up(key) {
            self.extras.move_up();
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_down(key) {
            self.extras.move_down();
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_toggle(key) {
            self.extras.toggle_current();
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_confirm(key) {
            self.connect_choice = crate::ui::binary_choice::BinaryChoice::new(true);
            self.step = Step::ConnectNow;
            return Ok(FlowSignal::Continue);
        }

        match key.code {
            crossterm::event::KeyCode::Tab => self.extras.toggle_fold_current(),
            crossterm::event::KeyCode::Char('/') => self.extras.toggle_filter_editing(),
            crossterm::event::KeyCode::Char('a') => self.extras.select_all(),
            crossterm::event::KeyCode::Char('n') => self.extras.select_none(),
            _ => {}
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_connect_now(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match self.connect_choice.on_key(key) {
            BinaryChoiceEvent::Back => {
                self.step = Step::ExtrasPicker;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::Continue => Ok(FlowSignal::Continue),
            BinaryChoiceEvent::ConfirmYes | BinaryChoiceEvent::ConfirmNo => {
                self.step = Step::Review;
                Ok(FlowSignal::Continue)
            }
        }
    }

    fn on_key_review(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        if keymap::is_back(key) {
            self.step = Step::ConnectNow;
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_confirm(key) {
            let Some(start_point) = self.start_point.clone() else {
                return Ok(FlowSignal::Continue);
            };

            let request = NewRequest {
                cwd: self.cwd.clone(),
                worktree_name: self.name_input.value().to_string(),
                start_point,
                add_worktrees_gitignore_entry: !self.prepare.gitignore_has_worktrees_entry
                    && self.gitignore_choice.yes_selected,
                selected_extras: self.extras.selected_for_copy(),
                connect_now: self.connect_choice.yes_selected,
            };

            match ops.execute_new(request) {
                Ok(result) => {
                    self.success = Some(result);
                    self.step = Step::Success;
                    self.error_message = None;
                }
                Err(error) => {
                    self.error_message = Some(format!("{error:#}"));
                    self.step = Step::ErrorScreen;
                }
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_success(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if keymap::is_back(key) || keymap::is_confirm(key) {
            Ok(FlowSignal::Exit(UiExit::Completed))
        } else {
            Ok(FlowSignal::Continue)
        }
    }

    fn on_key_error(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if keymap::is_back(key) || keymap::is_confirm(key) {
            self.step = Step::Review;
            Ok(FlowSignal::Continue)
        } else {
            Ok(FlowSignal::Continue)
        }
    }

    fn load_branches(
        &mut self,
        ops: &dyn NewFlowOps,
        query: &str,
    ) -> Result<PickerState<seshmux_core::git::BranchRef>> {
        let items = ops
            .query_branches(&self.prepare.repo_root, query)
            .with_context(|| "failed to load branch list".to_string())?;
        Ok(PickerState::from_items(
            if query.trim().is_empty() {
                None
            } else {
                Some(query.trim().to_string())
            },
            items,
        ))
    }

    fn load_commits(
        &mut self,
        ops: &dyn NewFlowOps,
        query: &str,
    ) -> Result<PickerState<seshmux_core::git::CommitRef>> {
        let items = ops
            .query_commits(&self.prepare.repo_root, query, 50)
            .with_context(|| "failed to load commit list".to_string())?;
        Ok(PickerState::from_items(
            if query.trim().is_empty() {
                None
            } else {
                Some(query.trim().to_string())
            },
            items,
        ))
    }
}
