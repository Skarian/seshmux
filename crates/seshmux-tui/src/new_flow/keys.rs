use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use tui_input::backend::crossterm::EventHandler;

use crate::UiExit;
use crate::keymap;
use crate::ui::binary_choice::BinaryChoiceEvent;

use super::picker::{PickerAction, PickerState};
use super::{
    ConnectBackTarget, FlowSignal, NewFlow, NewFlowErrorOrigin, NewFlowErrorState, NewFlowOps, Step,
};
use seshmux_app::{NewRequest, NewStartPoint};

impl NewFlow {
    pub(super) fn on_key(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        match &self.step {
            Step::GitignoreDecision => self.on_key_gitignore(key),
            Step::NameInput => self.on_key_name(key),
            Step::StartPointMode => self.on_key_start_mode(key, ops),
            Step::BranchPicker => self.on_key_branch_picker(key, ops),
            Step::CommitPicker => self.on_key_commit_picker(key, ops),
            Step::CopyExtrasDecision => self.on_key_copy_extras_decision(key, ops),
            Step::ExtrasIndexing => self.on_key_extras_indexing(key, ops),
            Step::ExtrasPicker => self.on_key_extras(key),
            Step::ConnectNow => self.on_key_connect_now(key),
            Step::Review => self.on_key_review(key, ops),
            Step::Success => self.on_key_success(key),
            Step::ErrorScreen(_) => self.on_key_error(key),
        }
    }

    pub(super) fn on_mouse(&mut self, mouse: MouseEvent) {
        if self.step != Step::ExtrasPicker {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => self.extras.move_up_by(3),
            MouseEventKind::ScrollDown => self.extras.move_down_by(3),
            _ => {}
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
                    self.step = Step::CopyExtrasDecision;
                }
                1 => {
                    let query = self.branch_search_input.value().trim().to_string();
                    self.branch_picker = Some(self.load_branches(ops, &query)?);
                    self.branch_filter_focused = false;
                    self.step = Step::BranchPicker;
                }
                _ => {
                    let query = self.commit_search_input.value().trim().to_string();
                    self.commit_picker = Some(self.load_commits(ops, &query)?);
                    self.commit_filter_focused = false;
                    self.step = Step::CommitPicker;
                }
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_branch_picker(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        if keymap::is_back(key) {
            self.branch_filter_focused = false;
            self.step = Step::StartPointMode;
            return Ok(FlowSignal::Continue);
        }

        if matches!(key.code, KeyCode::Char('/')) {
            self.branch_filter_focused = !self.branch_filter_focused;
            return Ok(FlowSignal::Continue);
        }

        if self.branch_filter_focused {
            if self
                .branch_search_input
                .handle_event(&Event::Key(key))
                .is_some()
            {
                let query = self.branch_search_input.value().trim().to_string();
                self.branch_picker = Some(self.load_branches(ops, &query)?);
            }
            return Ok(FlowSignal::Continue);
        }

        if self.branch_picker.is_none() {
            let query = self.branch_search_input.value().trim().to_string();
            self.branch_picker = Some(self.load_branches(ops, &query)?);
        }

        if keymap::is_up(key) {
            if let Some(picker) = &mut self.branch_picker {
                picker.move_up();
            }
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_down(key) {
            if let Some(picker) = &mut self.branch_picker {
                picker.move_down();
            }
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_confirm(key) {
            let selection = self.branch_picker.as_ref().and_then(|picker| {
                if let PickerAction::Pick(index) = picker.on_enter() {
                    picker.items.get(index).map(|branch| branch.name.clone())
                } else {
                    None
                }
            });
            if let Some(branch_name) = selection {
                self.start_point = Some(NewStartPoint::Branch(branch_name));
                self.branch_filter_focused = false;
                self.step = Step::CopyExtrasDecision;
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_commit_picker(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        if keymap::is_back(key) {
            self.commit_filter_focused = false;
            self.step = Step::StartPointMode;
            return Ok(FlowSignal::Continue);
        }

        if matches!(key.code, KeyCode::Char('/')) {
            self.commit_filter_focused = !self.commit_filter_focused;
            return Ok(FlowSignal::Continue);
        }

        if self.commit_filter_focused {
            if self
                .commit_search_input
                .handle_event(&Event::Key(key))
                .is_some()
            {
                let query = self.commit_search_input.value().trim().to_string();
                self.commit_picker = Some(self.load_commits(ops, &query)?);
            }
            return Ok(FlowSignal::Continue);
        }

        if self.commit_picker.is_none() {
            let query = self.commit_search_input.value().trim().to_string();
            self.commit_picker = Some(self.load_commits(ops, &query)?);
        }

        if keymap::is_up(key) {
            if let Some(picker) = &mut self.commit_picker {
                picker.move_up();
            }
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_down(key) {
            if let Some(picker) = &mut self.commit_picker {
                picker.move_down();
            }
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_confirm(key) {
            let selection = self.commit_picker.as_ref().and_then(|picker| {
                if let PickerAction::Pick(index) = picker.on_enter() {
                    picker.items.get(index).map(|commit| commit.hash.clone())
                } else {
                    None
                }
            });
            if let Some(commit_hash) = selection {
                self.start_point = Some(NewStartPoint::Commit(commit_hash));
                self.commit_filter_focused = false;
                self.step = Step::CopyExtrasDecision;
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_copy_extras_decision(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
    ) -> Result<FlowSignal> {
        match self.copy_extras_choice.on_key(key) {
            BinaryChoiceEvent::Back => {
                self.pending_skip_buckets_to_persist_after_create = None;
                self.step = self.start_point_step();
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::Continue => Ok(FlowSignal::Continue),
            BinaryChoiceEvent::ConfirmNo => {
                self.pending_skip_buckets_to_persist_after_create = None;
                self.connect_choice = crate::ui::binary_choice::BinaryChoice::new(true);
                self.connect_back_target = ConnectBackTarget::CopyExtrasDecision;
                self.step = Step::ConnectNow;
                Ok(FlowSignal::Continue)
            }
            BinaryChoiceEvent::ConfirmYes => {
                self.begin_extras_indexing(ops);
                Ok(FlowSignal::Continue)
            }
        }
    }

    fn on_key_extras_indexing(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
    ) -> Result<FlowSignal> {
        if self.skip_modal_open() {
            if keymap::is_back(key) {
                self.cancel_skip_modal();
                return Ok(FlowSignal::Continue);
            }
            if keymap::is_up(key) {
                self.skip_modal_move_up();
                return Ok(FlowSignal::Continue);
            }
            if keymap::is_down(key) {
                self.skip_modal_move_down();
                return Ok(FlowSignal::Continue);
            }
            if keymap::is_toggle(key) {
                self.skip_modal_toggle_current();
                return Ok(FlowSignal::Continue);
            }
            if keymap::is_confirm(key) {
                self.confirm_skip_modal_and_start_build(ops);
                return Ok(FlowSignal::Continue);
            }
            if matches!(key.code, KeyCode::Char('a')) {
                self.skip_modal_toggle_persist();
                return Ok(FlowSignal::Continue);
            }
            return Ok(FlowSignal::Continue);
        }

        if keymap::is_back(key) {
            self.invalidate_extras_indexing();
            self.pending_skip_buckets_to_persist_after_create = None;
            self.step = Step::CopyExtrasDecision;
        }
        Ok(FlowSignal::Continue)
    }

    fn on_key_extras(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if keymap::is_back(key) {
            self.extras.editing_filter = false;
            self.branch_filter_focused = false;
            self.commit_filter_focused = false;
            self.pending_skip_buckets_to_persist_after_create = None;
            self.step = Step::CopyExtrasDecision;
            return Ok(FlowSignal::Continue);
        }

        if matches!(key.code, KeyCode::Char('/')) {
            self.extras.toggle_filter_editing();
            return Ok(FlowSignal::Continue);
        }

        if self.extras.editing_filter {
            self.extras.edit_filter(key);
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
            self.connect_back_target = ConnectBackTarget::ExtrasPicker;
            self.step = Step::ConnectNow;
            return Ok(FlowSignal::Continue);
        }

        match key.code {
            KeyCode::Tab => self.extras.toggle_fold_current(),
            KeyCode::Char('a') => self.extras.select_all(),
            KeyCode::Char('n') => self.extras.select_none(),
            _ => {}
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_connect_now(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match self.connect_choice.on_key(key) {
            BinaryChoiceEvent::Back => {
                self.step = match self.connect_back_target {
                    ConnectBackTarget::CopyExtrasDecision => Step::CopyExtrasDecision,
                    ConnectBackTarget::ExtrasPicker => Step::ExtrasPicker,
                };
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
                selected_extras: if self.copy_extras_choice.yes_selected {
                    self.extras.selected_for_copy()
                } else {
                    Vec::new()
                },
                connect_now: self.connect_choice.yes_selected,
            };

            match ops.execute_new(request) {
                Ok(result) => {
                    if self.copy_extras_choice.yes_selected
                        && let Some(buckets) =
                            self.pending_skip_buckets_to_persist_after_create.take()
                        && let Err(error) =
                            ops.save_always_skip_buckets(&self.prepare.repo_root, &buckets)
                    {
                        self.success = Some(result);
                        self.success_notice = Some(format!(
                            "Worktree created, but failed to persist extras skip settings: {error}"
                        ));
                        self.step = Step::Success;
                        return Ok(FlowSignal::Continue);
                    }

                    self.pending_skip_buckets_to_persist_after_create = None;
                    self.success = Some(result);
                    self.success_notice = None;
                    self.step = Step::Success;
                }
                Err(error) => {
                    self.step = Step::ErrorScreen(NewFlowErrorState {
                        origin: NewFlowErrorOrigin::ReviewSubmit,
                        message: format!("{error:#}"),
                    });
                }
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_success(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if keymap::is_quit(key) {
            return Ok(FlowSignal::Exit(UiExit::Completed));
        }

        if keymap::is_back(key) || keymap::is_confirm(key) {
            return Ok(FlowSignal::Exit(UiExit::BackAtRoot));
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_error(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if keymap::is_back(key) || keymap::is_confirm(key) {
            let Step::ErrorScreen(error) = &self.step else {
                return Ok(FlowSignal::Continue);
            };
            self.step = match error.origin {
                NewFlowErrorOrigin::ExtrasIndexing => Step::CopyExtrasDecision,
                NewFlowErrorOrigin::ReviewSubmit => Step::Review,
            };
        }
        Ok(FlowSignal::Continue)
    }

    fn load_branches(
        &mut self,
        ops: &dyn NewFlowOps,
        query: &str,
    ) -> Result<PickerState<seshmux_core::git::BranchRef>> {
        let items = ops
            .query_branches(&self.prepare.repo_root, query)
            .with_context(|| "failed to load branch list".to_string())?;
        Ok(PickerState::from_items(items))
    }

    fn load_commits(
        &mut self,
        ops: &dyn NewFlowOps,
        query: &str,
    ) -> Result<PickerState<seshmux_core::git::CommitRef>> {
        let items = ops
            .query_commits(&self.prepare.repo_root, query, 50)
            .with_context(|| "failed to load commit list".to_string())?;
        Ok(PickerState::from_items(items))
    }
}
