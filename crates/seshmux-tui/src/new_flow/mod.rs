pub(crate) mod extras;
mod keys;
mod picker;
mod render;

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use anyhow::{Result, anyhow};
use crossterm::event::{KeyEvent, MouseEvent};
use seshmux_app::{App, NewPrepare, NewRequest, NewResult, NewStartPoint};
use seshmux_core::git::{BranchRef, CommitRef};
use tui_input::Input;

use crate::UiExit;
use crate::ui::binary_choice::BinaryChoice;
use crate::ui::loading::{
    BucketPlan, ExtrasLoadEvent, ExtrasLoader, LoadingState, SystemExtrasLoader,
};

use self::extras::ExtrasState;
use self::picker::PickerState;

pub(crate) trait NewFlowOps {
    fn prepare(&self, cwd: &Path) -> Result<NewPrepare>;
    fn query_branches(&self, repo_root: &Path, query: &str) -> Result<Vec<BranchRef>>;
    fn query_commits(&self, repo_root: &Path, query: &str, limit: usize) -> Result<Vec<CommitRef>>;
    fn load_always_skip_buckets_for_indexing(
        &self,
        repo_root: &Path,
    ) -> Result<seshmux_core::registry::AlwaysSkipBucketsLoad>;
    fn save_always_skip_buckets(&self, repo_root: &Path, buckets: &BTreeSet<String>) -> Result<()>;
    fn execute_new(&self, request: NewRequest) -> Result<NewResult>;
}

impl<'a> NewFlowOps for App<'a> {
    fn prepare(&self, cwd: &Path) -> Result<NewPrepare> {
        self.new_prepare(cwd)
    }

    fn query_branches(&self, repo_root: &Path, query: &str) -> Result<Vec<BranchRef>> {
        self.new_query_branches(repo_root, query)
    }

    fn query_commits(&self, repo_root: &Path, query: &str, limit: usize) -> Result<Vec<CommitRef>> {
        self.new_query_commits(repo_root, query, limit)
    }

    fn load_always_skip_buckets_for_indexing(
        &self,
        repo_root: &Path,
    ) -> Result<seshmux_core::registry::AlwaysSkipBucketsLoad> {
        self.new_load_always_skip_buckets_for_indexing(repo_root)
    }

    fn save_always_skip_buckets(&self, repo_root: &Path, buckets: &BTreeSet<String>) -> Result<()> {
        self.new_save_always_skip_buckets(repo_root, buckets)
    }

    fn execute_new(&self, request: NewRequest) -> Result<NewResult> {
        self.new_execute(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    GitignoreDecision,
    NameInput,
    StartPointMode,
    BranchPicker,
    CommitPicker,
    CopyExtrasDecision,
    ExtrasIndexing,
    ExtrasPicker,
    ConnectNow,
    Review,
    Success,
    ErrorScreen(NewFlowErrorState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectBackTarget {
    CopyExtrasDecision,
    ExtrasPicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewFlowErrorOrigin {
    ExtrasIndexing,
    ReviewSubmit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewFlowErrorState {
    origin: NewFlowErrorOrigin,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtrasIndexingPhase {
    Collecting,
    Classifying { candidate_count: usize },
    AwaitingSkipDecision { flagged_bucket_count: usize },
    Building { filtered_count: usize },
}

#[derive(Debug, Clone)]
struct SkipBucketChoice {
    bucket: String,
    count: usize,
    skip: bool,
    locked_in_config: bool,
}

#[derive(Debug, Clone, Default)]
struct SkipModalState {
    choices: Vec<SkipBucketChoice>,
    selected: usize,
    persist_always_skip: bool,
}

impl SkipModalState {
    fn from_plan(plan: &BucketPlan, configured_buckets: &BTreeSet<String>) -> Self {
        let configured_patterns = configured_buckets
            .iter()
            .filter_map(|value| normalized_path_components(value))
            .collect::<Vec<_>>();
        let choices = plan
            .flagged
            .iter()
            .map(|item| SkipBucketChoice {
                bucket: item.bucket.clone(),
                count: item.count,
                skip: true,
                locked_in_config: bucket_locked_in_config(&item.bucket, &configured_patterns),
            })
            .collect();

        Self {
            choices,
            selected: 0,
            persist_always_skip: false,
        }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.choices.len() {
            self.selected += 1;
        }
    }

    fn toggle_current(&mut self) {
        let Some(choice) = self.choices.get_mut(self.selected) else {
            return;
        };
        if choice.locked_in_config {
            return;
        }
        choice.skip = !choice.skip;
    }

    fn skipped_buckets(&self) -> BTreeSet<String> {
        self.choices
            .iter()
            .filter(|choice| choice.skip)
            .map(|choice| choice.bucket.clone())
            .collect()
    }
}

fn normalized_path_components(value: &str) -> Option<Vec<String>> {
    let normalized = seshmux_core::extras::normalize_extra_relative_path(Path::new(value)).ok()?;
    let mut components = Vec::<String>::new();
    for component in normalized.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        components.push(part.to_string_lossy().to_string());
    }
    if components.is_empty() {
        None
    } else {
        Some(components)
    }
}

fn bucket_locked_in_config(bucket: &str, configured_patterns: &[Vec<String>]) -> bool {
    let Some(bucket_components) = normalized_path_components(bucket) else {
        return false;
    };

    configured_patterns
        .iter()
        .any(|pattern| bucket_components.ends_with(pattern))
}

#[derive(Debug)]
struct ExtrasIndexingState {
    token: u64,
    phase: ExtrasIndexingPhase,
    loading: LoadingState,
    collect_receiver: Option<Receiver<ExtrasLoadEvent>>,
    build_receiver: Option<Receiver<ExtrasLoadEvent>>,
    collect_candidates: Option<Vec<PathBuf>>,
    collect_plan: Option<BucketPlan>,
    skip_modal: Option<SkipModalState>,
    persisted_skip_rules: BTreeSet<String>,
    configured_skip_rules: BTreeSet<String>,
    defer_skip_persist_until_create: bool,
}

struct NewFlow {
    cwd: PathBuf,
    prepare: NewPrepare,
    loader: Arc<dyn ExtrasLoader>,
    step: Step,
    gitignore_choice: BinaryChoice,
    name_input: Input,
    name_error: Option<String>,
    start_mode_selected: usize,
    start_point: Option<NewStartPoint>,
    branch_picker: Option<PickerState<BranchRef>>,
    branch_search_input: Input,
    branch_filter_focused: bool,
    commit_picker: Option<PickerState<CommitRef>>,
    commit_search_input: Input,
    commit_filter_focused: bool,
    copy_extras_choice: BinaryChoice,
    extras_indexing: Option<ExtrasIndexingState>,
    active_extras_index_token: Option<u64>,
    next_extras_index_token: u64,
    extras: ExtrasState,
    pending_skip_buckets_to_persist_after_create: Option<BTreeSet<String>>,
    connect_choice: BinaryChoice,
    connect_back_target: ConnectBackTarget,
    success: Option<NewResult>,
    success_notice: Option<String>,
}

pub(crate) struct NewScreen {
    flow: NewFlow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSignal {
    Continue,
    Exit(UiExit),
}

impl NewScreen {
    pub(crate) fn new(app: &App<'_>, cwd: &Path) -> Result<Self> {
        Ok(Self {
            flow: NewFlow::new(app, cwd)?,
        })
    }

    pub(crate) fn render(&self, frame: &mut ratatui::Frame<'_>) {
        self.flow.render(frame);
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent, app: &App<'_>) -> Result<Option<UiExit>> {
        match self.flow.on_key(key, app)? {
            FlowSignal::Continue => Ok(None),
            FlowSignal::Exit(exit) => Ok(Some(exit)),
        }
    }

    pub(crate) fn on_tick(&mut self) -> Result<()> {
        self.flow.on_tick();
        Ok(())
    }

    pub(crate) fn on_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        self.flow.on_mouse(mouse);
        Ok(())
    }

    pub(crate) fn should_drain_loader_after_input(&self) -> bool {
        self.flow.should_drain_loader_after_input()
    }
}

impl NewFlow {
    fn new(ops: &dyn NewFlowOps, cwd: &Path) -> Result<Self> {
        Self::new_with_loader(ops, cwd, Arc::new(SystemExtrasLoader::new()))
    }

    fn new_with_loader(
        ops: &dyn NewFlowOps,
        cwd: &Path,
        loader: Arc<dyn ExtrasLoader>,
    ) -> Result<Self> {
        let prepare = ops.prepare(cwd)?;
        let commits = ops.query_commits(&prepare.repo_root, "", 1)?;
        if commits.is_empty() {
            return Err(anyhow!(
                "current branch/HEAD has no commits yet; create an initial commit on this branch before starting seshmux"
            ));
        }
        let extras = ExtrasState::from_candidates(&[])?;

        let first_step = if prepare.gitignore_has_worktrees_entry {
            Step::NameInput
        } else {
            Step::GitignoreDecision
        };

        Ok(Self {
            cwd: cwd.to_path_buf(),
            prepare,
            loader,
            step: first_step,
            gitignore_choice: BinaryChoice::new(true),
            name_input: Input::default(),
            name_error: None,
            start_mode_selected: 0,
            start_point: None,
            branch_picker: None,
            branch_search_input: Input::default(),
            branch_filter_focused: false,
            commit_picker: None,
            commit_search_input: Input::default(),
            commit_filter_focused: false,
            copy_extras_choice: BinaryChoice::new(false),
            extras_indexing: None,
            active_extras_index_token: None,
            next_extras_index_token: 1,
            extras,
            pending_skip_buckets_to_persist_after_create: None,
            connect_choice: BinaryChoice::new(true),
            connect_back_target: ConnectBackTarget::CopyExtrasDecision,
            success: None,
            success_notice: None,
        })
    }

    fn start_point_step(&self) -> Step {
        match self.start_point {
            Some(NewStartPoint::CurrentBranch) => Step::StartPointMode,
            Some(NewStartPoint::Branch(_)) => Step::BranchPicker,
            Some(NewStartPoint::Commit(_)) => Step::CommitPicker,
            None => Step::StartPointMode,
        }
    }

    fn review_selected_extras_count(&self) -> usize {
        if self.copy_extras_choice.yes_selected {
            self.extras.selected_for_copy().len()
        } else {
            0
        }
    }

    fn begin_extras_indexing(&mut self, ops: &dyn NewFlowOps) {
        let loaded = match ops.load_always_skip_buckets_for_indexing(&self.prepare.repo_root) {
            Ok(value) => value,
            Err(error) => {
                self.active_extras_index_token = None;
                self.extras_indexing = None;
                self.step = Step::ErrorScreen(NewFlowErrorState {
                    origin: NewFlowErrorOrigin::ExtrasIndexing,
                    message: format!("{error:#}"),
                });
                return;
            }
        };

        self.pending_skip_buckets_to_persist_after_create =
            loaded.registry_missing.then_some(loaded.buckets.clone());

        let token = self.next_extras_index_token;
        self.next_extras_index_token = self.next_extras_index_token.saturating_add(1);
        self.active_extras_index_token = Some(token);

        let collect_receiver = self.loader.spawn_collect_and_classify(
            self.prepare.repo_root.clone(),
            token,
            loaded.buckets.clone(),
        );

        self.extras_indexing = Some(ExtrasIndexingState {
            token,
            phase: ExtrasIndexingPhase::Collecting,
            loading: LoadingState::default(),
            collect_receiver: Some(collect_receiver),
            build_receiver: None,
            collect_candidates: None,
            collect_plan: None,
            skip_modal: None,
            persisted_skip_rules: loaded.buckets,
            configured_skip_rules: loaded.configured_buckets,
            defer_skip_persist_until_create: loaded.registry_missing,
        });
        self.step = Step::ExtrasIndexing;
    }

    fn invalidate_extras_indexing(&mut self) {
        self.active_extras_index_token = None;
        self.extras_indexing = None;
    }

    fn should_drain_loader_after_input(&self) -> bool {
        self.step == Step::ExtrasIndexing && self.extras_indexing.is_some()
    }

    fn fail_extras_indexing(&mut self, message: String) {
        self.success_notice = None;
        self.invalidate_extras_indexing();
        self.step = Step::ErrorScreen(NewFlowErrorState {
            origin: NewFlowErrorOrigin::ExtrasIndexing,
            message,
        });
    }

    fn start_build_with_candidates(
        &mut self,
        token: u64,
        candidates: Vec<PathBuf>,
        skipped_buckets: BTreeSet<String>,
    ) {
        if Some(token) != self.active_extras_index_token {
            return;
        }

        let filtered = seshmux_core::extras::filter_candidates_by_skipped_buckets(
            &candidates,
            &skipped_buckets,
        );
        let build_receiver = self.loader.spawn_build(filtered.clone(), token);

        if let Some(indexing) = &mut self.extras_indexing {
            indexing.phase = ExtrasIndexingPhase::Building {
                filtered_count: filtered.len(),
            };
            indexing.build_receiver = Some(build_receiver);
            indexing.collect_receiver = None;
            indexing.collect_candidates = None;
            indexing.collect_plan = None;
            indexing.skip_modal = None;
        }
    }

    fn apply_collect_event(&mut self, event: ExtrasLoadEvent) {
        let Some(indexing) = &mut self.extras_indexing else {
            return;
        };

        match event {
            ExtrasLoadEvent::Collecting => {
                indexing.phase = ExtrasIndexingPhase::Collecting;
            }
            ExtrasLoadEvent::Classifying { candidate_count } => {
                indexing.phase = ExtrasIndexingPhase::Classifying { candidate_count };
            }
            ExtrasLoadEvent::AwaitingSkipDecision {
                flagged_bucket_count,
            } => {
                indexing.phase = ExtrasIndexingPhase::AwaitingSkipDecision {
                    flagged_bucket_count,
                };
            }
            ExtrasLoadEvent::DoneCollect {
                token,
                candidates,
                plan,
            } => {
                if Some(token) != self.active_extras_index_token {
                    return;
                }

                indexing.collect_receiver = None;
                if plan.flagged_count() == 0 {
                    self.start_build_with_candidates(token, candidates, BTreeSet::new());
                    return;
                }

                indexing.phase = ExtrasIndexingPhase::AwaitingSkipDecision {
                    flagged_bucket_count: plan.flagged_count(),
                };
                indexing.collect_candidates = Some(candidates);
                indexing.collect_plan = Some(plan.clone());
                indexing.skip_modal = Some(SkipModalState::from_plan(
                    &plan,
                    &indexing.configured_skip_rules,
                ));
            }
            ExtrasLoadEvent::Done { token, result } => {
                if Some(token) != self.active_extras_index_token {
                    return;
                }

                if let Err(message) = result {
                    self.fail_extras_indexing(message);
                }
            }
            ExtrasLoadEvent::Building { .. } => {}
        }
    }

    fn apply_build_event(&mut self, event: ExtrasLoadEvent) {
        let Some(indexing) = &mut self.extras_indexing else {
            return;
        };

        match event {
            ExtrasLoadEvent::Building { filtered_count } => {
                indexing.phase = ExtrasIndexingPhase::Building { filtered_count };
            }
            ExtrasLoadEvent::Done { token, result } => {
                if Some(token) != self.active_extras_index_token {
                    return;
                }

                match result {
                    Ok(index) => {
                        self.extras = ExtrasState::from_index(index);
                        self.invalidate_extras_indexing();
                        self.step = Step::ExtrasPicker;
                    }
                    Err(message) => {
                        self.fail_extras_indexing(message);
                    }
                }
            }
            ExtrasLoadEvent::Collecting
            | ExtrasLoadEvent::Classifying { .. }
            | ExtrasLoadEvent::AwaitingSkipDecision { .. }
            | ExtrasLoadEvent::DoneCollect { .. } => {}
        }
    }

    fn on_tick(&mut self) {
        if self.step != Step::ExtrasIndexing {
            return;
        }

        let mut collect_events = Vec::<ExtrasLoadEvent>::new();
        let mut build_events = Vec::<ExtrasLoadEvent>::new();
        let mut collect_disconnected = false;
        let mut build_disconnected = false;

        if let Some(indexing) = &mut self.extras_indexing {
            indexing.loading.next_frame();

            if let Some(receiver) = &indexing.collect_receiver {
                loop {
                    match receiver.try_recv() {
                        Ok(event) => collect_events.push(event),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            collect_disconnected = true;
                            break;
                        }
                    }
                }
            }

            if let Some(receiver) = &indexing.build_receiver {
                loop {
                    match receiver.try_recv() {
                        Ok(event) => build_events.push(event),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            build_disconnected = true;
                            break;
                        }
                    }
                }
            }
        }

        for event in collect_events {
            self.apply_collect_event(event);
        }

        for event in build_events {
            self.apply_build_event(event);
        }

        let collect_receiver_still_expected = self
            .extras_indexing
            .as_ref()
            .and_then(|indexing| indexing.collect_receiver.as_ref())
            .is_some();
        let build_receiver_still_expected = self
            .extras_indexing
            .as_ref()
            .and_then(|indexing| indexing.build_receiver.as_ref())
            .is_some();

        if collect_disconnected
            && self.active_extras_index_token.is_some()
            && collect_receiver_still_expected
        {
            self.fail_extras_indexing("extras collect worker ended unexpectedly".to_string());
        }

        if build_disconnected
            && self.active_extras_index_token.is_some()
            && build_receiver_still_expected
        {
            self.fail_extras_indexing("extras build worker ended unexpectedly".to_string());
        }
    }

    fn skip_modal_open(&self) -> bool {
        self.extras_indexing
            .as_ref()
            .and_then(|indexing| indexing.skip_modal.as_ref())
            .is_some()
    }

    fn confirm_skip_modal_and_start_build(&mut self, ops: &dyn NewFlowOps) {
        let Some(indexing) = &self.extras_indexing else {
            return;
        };

        let token = indexing.token;
        if Some(token) != self.active_extras_index_token {
            return;
        }

        let Some(candidates) = indexing.collect_candidates.clone() else {
            return;
        };
        let Some(skip_modal) = indexing.skip_modal.clone() else {
            return;
        };
        let defer_skip_persist_until_create = indexing.defer_skip_persist_until_create;

        let selected_skips = skip_modal.skipped_buckets();
        if skip_modal.persist_always_skip {
            let mut persisted = indexing.persisted_skip_rules.clone();
            persisted.extend(selected_skips.iter().cloned());

            if defer_skip_persist_until_create {
                self.pending_skip_buckets_to_persist_after_create = Some(persisted.clone());
            } else if let Err(error) =
                ops.save_always_skip_buckets(&self.prepare.repo_root, &persisted)
            {
                self.fail_extras_indexing(format!("{error:#}"));
                return;
            }

            if let Some(active) = &mut self.extras_indexing {
                active.persisted_skip_rules = persisted;
            }
        }

        self.start_build_with_candidates(token, candidates, selected_skips);
    }

    fn cancel_skip_modal(&mut self) {
        self.pending_skip_buckets_to_persist_after_create = None;
        self.invalidate_extras_indexing();
        self.step = Step::CopyExtrasDecision;
    }

    fn skip_modal_toggle_current(&mut self) {
        if let Some(indexing) = &mut self.extras_indexing
            && let Some(modal) = &mut indexing.skip_modal
        {
            modal.toggle_current();
        }
    }

    fn skip_modal_move_up(&mut self) {
        if let Some(indexing) = &mut self.extras_indexing
            && let Some(modal) = &mut indexing.skip_modal
        {
            modal.move_up();
        }
    }

    fn skip_modal_move_down(&mut self) {
        if let Some(indexing) = &mut self.extras_indexing
            && let Some(modal) = &mut indexing.skip_modal
        {
            modal.move_down();
        }
    }

    fn skip_modal_toggle_persist(&mut self) {
        if let Some(indexing) = &mut self.extras_indexing
            && let Some(modal) = &mut indexing.skip_modal
        {
            modal.persist_always_skip = !modal.persist_always_skip;
        }
    }
}
#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use seshmux_app::{NewPrepare, NewRequest, NewResult};
    use seshmux_core::git::{BranchRef, BranchSource, CommitRef};

    use crate::ui::loading::{BucketPlan, ExtrasLoadEvent, ExtrasLoader, FlaggedBucket};

    use super::{FlowSignal, NewFlow, NewFlowErrorOrigin, NewFlowErrorState, NewFlowOps, Step};

    struct FakeOps {
        prepare: NewPrepare,
        branches: Vec<BranchRef>,
        latest_commits: Vec<CommitRef>,
        searched_commits: Vec<CommitRef>,
        always_skip_buckets: Mutex<BTreeSet<String>>,
        configured_skip_buckets: Mutex<BTreeSet<String>>,
        skip_registry_missing_for_indexing: bool,
        saved_skip_buckets: Mutex<Vec<BTreeSet<String>>>,
        save_skip_buckets_error: Option<String>,
        execute_calls: Mutex<Vec<NewRequest>>,
        execute_error: Option<String>,
    }

    impl FakeOps {
        fn new(repo_root: PathBuf) -> Self {
            Self {
                prepare: NewPrepare {
                    repo_root: repo_root.clone(),
                    worktrees_dir: repo_root.join("worktrees"),
                    gitignore_has_worktrees_entry: false,
                },
                branches: vec![BranchRef {
                    name: "main".to_string(),
                    source: BranchSource::Local,
                    display: "main [local]".to_string(),
                }],
                latest_commits: vec![CommitRef {
                    hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    short_hash: "aaaaaaa".to_string(),
                    subject: "first".to_string(),
                    display: "aaaaaaa first".to_string(),
                }],
                searched_commits: vec![CommitRef {
                    hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                    short_hash: "bbbbbbb".to_string(),
                    subject: "second".to_string(),
                    display: "bbbbbbb second".to_string(),
                }],
                always_skip_buckets: Mutex::new(BTreeSet::new()),
                configured_skip_buckets: Mutex::new(BTreeSet::new()),
                skip_registry_missing_for_indexing: false,
                saved_skip_buckets: Mutex::new(Vec::new()),
                save_skip_buckets_error: None,
                execute_calls: Mutex::new(Vec::new()),
                execute_error: None,
            }
        }

        fn saved_skip_buckets(&self) -> Vec<BTreeSet<String>> {
            self.saved_skip_buckets.lock().expect("saved lock").clone()
        }
    }

    impl NewFlowOps for FakeOps {
        fn prepare(&self, _cwd: &Path) -> Result<NewPrepare> {
            Ok(self.prepare.clone())
        }

        fn query_branches(&self, _repo_root: &Path, _query: &str) -> Result<Vec<BranchRef>> {
            Ok(self.branches.clone())
        }

        fn query_commits(
            &self,
            _repo_root: &Path,
            query: &str,
            _limit: usize,
        ) -> Result<Vec<CommitRef>> {
            if query.trim().is_empty() {
                Ok(self.latest_commits.clone())
            } else {
                Ok(self.searched_commits.clone())
            }
        }

        fn load_always_skip_buckets_for_indexing(
            &self,
            _repo_root: &Path,
        ) -> Result<seshmux_core::registry::AlwaysSkipBucketsLoad> {
            Ok(seshmux_core::registry::AlwaysSkipBucketsLoad {
                buckets: self.always_skip_buckets.lock().expect("rules lock").clone(),
                configured_buckets: self
                    .configured_skip_buckets
                    .lock()
                    .expect("configured rules lock")
                    .clone(),
                registry_missing: self.skip_registry_missing_for_indexing,
            })
        }

        fn save_always_skip_buckets(
            &self,
            _repo_root: &Path,
            buckets: &BTreeSet<String>,
        ) -> Result<()> {
            if let Some(message) = &self.save_skip_buckets_error {
                return Err(anyhow::anyhow!(message.clone()));
            }
            self.saved_skip_buckets
                .lock()
                .expect("saved lock")
                .push(buckets.clone());
            *self.always_skip_buckets.lock().expect("rules lock") = buckets.clone();
            *self
                .configured_skip_buckets
                .lock()
                .expect("configured rules lock") = buckets.clone();
            Ok(())
        }

        fn execute_new(&self, request: NewRequest) -> Result<NewResult> {
            if let Some(message) = &self.execute_error {
                return Err(anyhow::anyhow!(message.clone()));
            }
            self.execute_calls
                .lock()
                .expect("execute lock")
                .push(request.clone());
            Ok(NewResult {
                repo_root: request.cwd.clone(),
                worktrees_dir: request.cwd.join("worktrees"),
                worktree_path: request.cwd.join("worktrees").join(&request.worktree_name),
                branch_name: request.worktree_name.clone(),
                session_name: format!("repo/{}", request.worktree_name),
                attach_command: format!("tmux attach-session -t repo/{}", request.worktree_name),
                connected_now: request.connect_now,
            })
        }
    }

    #[derive(Default)]
    struct ScriptedLoader {
        collect_calls: Mutex<Vec<(PathBuf, u64, BTreeSet<String>)>>,
        build_calls: Mutex<Vec<(u64, Vec<PathBuf>)>>,
        collect_senders: Mutex<Vec<Sender<ExtrasLoadEvent>>>,
        build_senders: Mutex<Vec<Sender<ExtrasLoadEvent>>>,
    }

    impl ScriptedLoader {
        fn collect_call_count(&self) -> usize {
            self.collect_calls.lock().expect("collect calls lock").len()
        }

        fn build_call_count(&self) -> usize {
            self.build_calls.lock().expect("build calls lock").len()
        }

        fn last_build_candidates(&self) -> Vec<PathBuf> {
            self.build_calls
                .lock()
                .expect("build calls lock")
                .last()
                .map(|(_, candidates)| candidates.clone())
                .unwrap_or_default()
        }

        fn send_collect(&self, event: ExtrasLoadEvent) {
            let sender = self
                .collect_senders
                .lock()
                .expect("collect senders lock")
                .last()
                .cloned()
                .expect("collect sender should exist");
            sender.send(event).expect("send collect event");
        }

        fn send_build(&self, event: ExtrasLoadEvent) {
            let sender = self
                .build_senders
                .lock()
                .expect("build senders lock")
                .last()
                .cloned()
                .expect("build sender should exist");
            sender.send(event).expect("send build event");
        }

        fn close_collect_channel(&self) {
            let _ = self
                .collect_senders
                .lock()
                .expect("collect senders lock")
                .pop();
        }

        fn close_build_channel(&self) {
            let _ = self.build_senders.lock().expect("build senders lock").pop();
        }
    }

    impl ExtrasLoader for ScriptedLoader {
        fn spawn_collect_and_classify(
            &self,
            repo_root: PathBuf,
            token: u64,
            skip_rules: BTreeSet<String>,
        ) -> mpsc::Receiver<ExtrasLoadEvent> {
            self.collect_calls
                .lock()
                .expect("collect calls lock")
                .push((repo_root, token, skip_rules));
            let (sender, receiver) = mpsc::channel();
            self.collect_senders
                .lock()
                .expect("collect senders lock")
                .push(sender);
            receiver
        }

        fn spawn_build(
            &self,
            candidates: Vec<PathBuf>,
            token: u64,
        ) -> mpsc::Receiver<ExtrasLoadEvent> {
            self.build_calls
                .lock()
                .expect("build calls lock")
                .push((token, candidates));
            let (sender, receiver) = mpsc::channel();
            self.build_senders
                .lock()
                .expect("build senders lock")
                .push(sender);
            receiver
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn mouse(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn build_index(paths: &[PathBuf]) -> crate::new_flow::extras::ExtrasIndex {
        super::extras::build_extras_index_from_paths(paths).expect("index")
    }

    fn new_flow(ops: &FakeOps, loader: Arc<ScriptedLoader>, repo_root: &Path) -> NewFlow {
        NewFlow::new_with_loader(ops, repo_root, loader).expect("flow")
    }

    fn advance_to_copy_extras_decision(flow: &mut NewFlow, ops: &FakeOps, name: &str) {
        flow.on_key(key(KeyCode::Enter), ops).expect("gitignore");
        for character in name.chars() {
            flow.on_key(key(KeyCode::Char(character)), ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), ops).expect("name enter");
        flow.on_key(key(KeyCode::Enter), ops)
            .expect("start current branch");
        assert_eq!(flow.step, Step::CopyExtrasDecision);
    }

    fn confirm_copy_extras_yes(flow: &mut NewFlow, ops: &FakeOps) {
        flow.on_key(key(KeyCode::Char(' ')), ops)
            .expect("toggle copy extras to yes");
        flow.on_key(key(KeyCode::Enter), ops)
            .expect("confirm copy extras yes");
    }

    fn finish_indexing_without_modal(
        flow: &mut NewFlow,
        ops: &FakeOps,
        loader: &ScriptedLoader,
        paths: &[PathBuf],
    ) {
        let token = flow
            .active_extras_index_token
            .expect("indexing token should be active");

        loader.send_collect(ExtrasLoadEvent::Collecting);
        loader.send_collect(ExtrasLoadEvent::Classifying {
            candidate_count: paths.len(),
        });
        loader.send_collect(ExtrasLoadEvent::AwaitingSkipDecision {
            flagged_bucket_count: 0,
        });
        loader.send_collect(ExtrasLoadEvent::DoneCollect {
            token,
            candidates: paths.to_vec(),
            plan: BucketPlan::default(),
        });
        flow.on_tick();

        loader.send_build(ExtrasLoadEvent::Building {
            filtered_count: paths.len(),
        });
        loader.send_build(ExtrasLoadEvent::Done {
            token,
            result: Ok(build_index(paths)),
        });
        flow.on_tick();

        assert_eq!(flow.step, Step::ExtrasPicker);
        let _ = flow
            .on_key(key(KeyCode::Enter), ops)
            .expect("continue extras");
        assert_eq!(flow.step, Step::ConnectNow);
    }

    #[test]
    fn new_flow_requires_at_least_one_commit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.latest_commits.clear();

        assert!(NewFlow::new(&ops, &repo_root).is_err());
    }

    #[test]
    fn new_flow_opt_in_starts_async_collect_only_after_confirmation() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);

        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        assert_eq!(loader.collect_call_count(), 0);

        confirm_copy_extras_yes(&mut flow, &ops);

        assert_eq!(flow.step, Step::ExtrasIndexing);
        assert_eq!(loader.collect_call_count(), 1);
    }

    #[test]
    fn copy_extras_yes_transitions_without_waiting_for_listing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader, &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");

        let started = Instant::now();
        confirm_copy_extras_yes(&mut flow, &ops);
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(25),
            "copy extras confirmation should not block: {elapsed:?}"
        );
        assert_eq!(flow.step, Step::ExtrasIndexing);
    }

    #[test]
    fn extras_indexing_emits_collecting_classifying_and_building_phases() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let token = flow.active_extras_index_token.expect("active token");
        let candidates = vec![PathBuf::from("src/main.rs")];

        loader.send_collect(ExtrasLoadEvent::Collecting);
        flow.on_tick();
        assert!(matches!(
            flow.extras_indexing.as_ref().expect("indexing").phase,
            super::ExtrasIndexingPhase::Collecting
        ));

        loader.send_collect(ExtrasLoadEvent::Classifying { candidate_count: 1 });
        flow.on_tick();
        assert!(matches!(
            flow.extras_indexing.as_ref().expect("indexing").phase,
            super::ExtrasIndexingPhase::Classifying { candidate_count: 1 }
        ));

        loader.send_collect(ExtrasLoadEvent::AwaitingSkipDecision {
            flagged_bucket_count: 0,
        });
        loader.send_collect(ExtrasLoadEvent::DoneCollect {
            token,
            candidates: candidates.clone(),
            plan: BucketPlan::default(),
        });
        flow.on_tick();
        assert_eq!(loader.build_call_count(), 1);

        loader.send_build(ExtrasLoadEvent::Building { filtered_count: 1 });
        flow.on_tick();
        assert!(matches!(
            flow.extras_indexing.as_ref().expect("indexing").phase,
            super::ExtrasIndexingPhase::Building { filtered_count: 1 }
        ));

        loader.send_build(ExtrasLoadEvent::Done {
            token,
            result: Ok(build_index(&candidates)),
        });
        flow.on_tick();

        assert_eq!(flow.step, Step::ExtrasPicker);
    }

    fn open_skip_modal(flow: &mut NewFlow, loader: &ScriptedLoader, candidates: &[PathBuf]) -> u64 {
        open_skip_modal_for_bucket(flow, loader, candidates, "target")
    }

    fn open_skip_modal_for_bucket(
        flow: &mut NewFlow,
        loader: &ScriptedLoader,
        candidates: &[PathBuf],
        bucket: &str,
    ) -> u64 {
        let token = flow.active_extras_index_token.expect("token");
        let plan = BucketPlan {
            flagged: vec![FlaggedBucket {
                bucket: bucket.to_string(),
                count: 1,
            }],
        };

        loader.send_collect(ExtrasLoadEvent::Collecting);
        loader.send_collect(ExtrasLoadEvent::Classifying {
            candidate_count: candidates.len(),
        });
        loader.send_collect(ExtrasLoadEvent::AwaitingSkipDecision {
            flagged_bucket_count: 1,
        });
        loader.send_collect(ExtrasLoadEvent::DoneCollect {
            token,
            candidates: candidates.to_vec(),
            plan,
        });
        flow.on_tick();
        token
    }

    fn assert_error_screen(flow: &NewFlow, origin: NewFlowErrorOrigin, message_contains: &str) {
        match &flow.step {
            Step::ErrorScreen(error) => {
                assert_eq!(error.origin, origin);
                assert!(
                    error.message.contains(message_contains),
                    "expected error message to contain {message_contains:?}, got {:?}",
                    error.message
                );
            }
            other => panic!("expected ErrorScreen step, got {other:?}"),
        }
    }

    #[test]
    fn extras_skip_modal_blocks_build_until_confirmation() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![
            PathBuf::from("target/debug/a.o"),
            PathBuf::from("src/main.rs"),
        ];
        let _token = open_skip_modal(&mut flow, &loader, &candidates);

        assert_eq!(loader.build_call_count(), 0);
        assert!(flow.skip_modal_open());

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm modal");

        assert_eq!(loader.build_call_count(), 1);
        assert_eq!(
            loader.last_build_candidates(),
            vec![PathBuf::from("src/main.rs")]
        );
    }

    #[test]
    fn extras_skip_modal_esc_cancels_and_returns_to_copy_extras_decision() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![
            PathBuf::from("target/debug/a.o"),
            PathBuf::from("src/main.rs"),
        ];
        let _token = open_skip_modal(&mut flow, &loader, &candidates);

        flow.on_key(key(KeyCode::Esc), &ops).expect("cancel modal");

        assert_eq!(flow.step, Step::CopyExtrasDecision);
        assert!(flow.active_extras_index_token.is_none());
    }

    #[test]
    fn extras_skip_modal_bypasses_when_no_flagged_buckets() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let token = flow.active_extras_index_token.expect("token");
        let candidates = vec![PathBuf::from("src/main.rs")];
        loader.send_collect(ExtrasLoadEvent::DoneCollect {
            token,
            candidates: candidates.clone(),
            plan: BucketPlan::default(),
        });
        flow.on_tick();

        assert_eq!(loader.build_call_count(), 1);
        assert!(!flow.skip_modal_open());
    }

    #[test]
    fn extras_skip_modal_defaults_to_yes_and_persists_selection() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![
            PathBuf::from("target/debug/a.o"),
            PathBuf::from("src/main.rs"),
        ];
        let _token = open_skip_modal(&mut flow, &loader, &candidates);

        let modal = flow
            .extras_indexing
            .as_ref()
            .and_then(|state| state.skip_modal.as_ref())
            .expect("modal");
        assert!(modal.choices.first().expect("first bucket").skip);

        flow.on_key(key(KeyCode::Char('a')), &ops)
            .expect("toggle persist");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm modal");

        let saved = ops.saved_skip_buckets();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].contains("target"));
        assert_eq!(
            loader.last_build_candidates(),
            vec![PathBuf::from("src/main.rs")]
        );
    }

    #[test]
    fn extras_skip_modal_config_set_bucket_is_locked_on() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        *ops.always_skip_buckets.lock().expect("rules lock") =
            BTreeSet::from(["target".to_string()]);
        *ops.configured_skip_buckets
            .lock()
            .expect("configured rules lock") = BTreeSet::from(["target".to_string()]);

        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![
            PathBuf::from("target/debug/a.o"),
            PathBuf::from("src/main.rs"),
        ];
        let _token = open_skip_modal(&mut flow, &loader, &candidates);

        let modal_before = flow
            .extras_indexing
            .as_ref()
            .and_then(|state| state.skip_modal.as_ref())
            .expect("modal");
        assert!(modal_before.choices.first().expect("first choice").skip);
        assert!(
            modal_before
                .choices
                .first()
                .expect("first choice")
                .locked_in_config
        );

        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("attempt to unselect locked bucket");
        let modal_after = flow
            .extras_indexing
            .as_ref()
            .and_then(|state| state.skip_modal.as_ref())
            .expect("modal");
        assert!(modal_after.choices.first().expect("first choice").skip);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm modal");
        assert_eq!(
            loader.last_build_candidates(),
            vec![PathBuf::from("src/main.rs")]
        );
    }

    #[test]
    fn extras_skip_modal_config_set_rule_locks_nested_bucket() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        *ops.always_skip_buckets.lock().expect("rules lock") =
            BTreeSet::from(["target".to_string()]);
        *ops.configured_skip_buckets
            .lock()
            .expect("configured rules lock") = BTreeSet::from(["target".to_string()]);

        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![
            PathBuf::from("app/mobile/target/debug/a.o"),
            PathBuf::from("src/main.rs"),
        ];
        let _token =
            open_skip_modal_for_bucket(&mut flow, &loader, &candidates, "app/mobile/target");

        let modal_before = flow
            .extras_indexing
            .as_ref()
            .and_then(|state| state.skip_modal.as_ref())
            .expect("modal");
        assert!(modal_before.choices.first().expect("first choice").skip);
        assert!(
            modal_before
                .choices
                .first()
                .expect("first choice")
                .locked_in_config
        );

        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("attempt to unselect locked bucket");
        let modal_after = flow
            .extras_indexing
            .as_ref()
            .and_then(|state| state.skip_modal.as_ref())
            .expect("modal");
        assert!(modal_after.choices.first().expect("first choice").skip);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm modal");
        assert_eq!(
            loader.last_build_candidates(),
            vec![PathBuf::from("src/main.rs")]
        );
    }

    #[test]
    fn extras_skip_modal_seed_only_bucket_remains_toggleable() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.skip_registry_missing_for_indexing = true;
        *ops.always_skip_buckets.lock().expect("rules lock") =
            BTreeSet::from(["target".to_string()]);

        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![
            PathBuf::from("target/debug/a.o"),
            PathBuf::from("src/main.rs"),
        ];
        let _token = open_skip_modal(&mut flow, &loader, &candidates);

        let modal_before = flow
            .extras_indexing
            .as_ref()
            .and_then(|state| state.skip_modal.as_ref())
            .expect("modal");
        assert!(modal_before.choices.first().expect("first choice").skip);
        assert!(
            !modal_before
                .choices
                .first()
                .expect("first choice")
                .locked_in_config
        );

        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("unselect seed-only bucket");
        let modal_after = flow
            .extras_indexing
            .as_ref()
            .and_then(|state| state.skip_modal.as_ref())
            .expect("modal");
        assert!(!modal_after.choices.first().expect("first choice").skip);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm modal");
        assert_eq!(loader.last_build_candidates(), candidates);
    }

    #[test]
    fn extras_skip_persist_is_deferred_until_create_when_registry_missing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.skip_registry_missing_for_indexing = true;
        *ops.always_skip_buckets.lock().expect("rules lock") =
            BTreeSet::from(["target".to_string()]);

        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![
            PathBuf::from("target/debug/a.o"),
            PathBuf::from("src/main.rs"),
        ];
        let token = open_skip_modal(&mut flow, &loader, &candidates);
        flow.on_key(key(KeyCode::Char('a')), &ops)
            .expect("toggle persist");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm skip modal");

        assert!(ops.saved_skip_buckets().is_empty());

        loader.send_build(ExtrasLoadEvent::Building { filtered_count: 1 });
        loader.send_build(ExtrasLoadEvent::Done {
            token,
            result: Ok(build_index(&[PathBuf::from("src/main.rs")])),
        });
        flow.on_tick();

        assert_eq!(flow.step, Step::ExtrasPicker);
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("continue extras");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("continue connect");
        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");

        assert_eq!(flow.step, Step::Success);
        let saved = ops.saved_skip_buckets();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].contains("target"));
    }

    #[test]
    fn extras_skip_modal_keeps_loader_tick_polling_active_under_repeated_input() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let candidates = vec![PathBuf::from("target/debug/a.o")];
        let _token = open_skip_modal(&mut flow, &loader, &candidates);

        let before = flow
            .extras_indexing
            .as_ref()
            .expect("indexing")
            .loading
            .clone();
        for _ in 0..3 {
            flow.on_key(key(KeyCode::Down), &ops).expect("down");
            flow.on_key(key(KeyCode::Up), &ops).expect("up");
            flow.on_tick();
        }
        let after = flow
            .extras_indexing
            .as_ref()
            .expect("indexing")
            .loading
            .clone();

        assert_ne!(format!("{before:?}"), format!("{after:?}"));
    }

    #[test]
    fn collect_disconnect_after_donecollect_does_not_error() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let token = flow.active_extras_index_token.expect("token");
        loader.send_collect(ExtrasLoadEvent::DoneCollect {
            token,
            candidates: vec![PathBuf::from("target/debug/a.o")],
            plan: BucketPlan {
                flagged: vec![FlaggedBucket {
                    bucket: "target".to_string(),
                    count: 1,
                }],
            },
        });
        loader.close_collect_channel();

        flow.on_tick();

        assert_eq!(flow.step, Step::ExtrasIndexing);
        assert!(flow.skip_modal_open());
        assert!(!matches!(flow.step, Step::ErrorScreen(_)));
    }

    #[test]
    fn collect_disconnect_before_done_still_errors() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        loader.close_collect_channel();
        flow.on_tick();

        assert_error_screen(
            &flow,
            NewFlowErrorOrigin::ExtrasIndexing,
            "extras collect worker ended unexpectedly",
        );
    }

    #[test]
    fn build_disconnect_before_done_still_errors() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let token = flow.active_extras_index_token.expect("token");
        loader.send_collect(ExtrasLoadEvent::DoneCollect {
            token,
            candidates: vec![PathBuf::from("src/main.rs")],
            plan: BucketPlan::default(),
        });
        flow.on_tick();
        assert_eq!(loader.build_call_count(), 1);

        loader.close_build_channel();
        flow.on_tick();

        assert_error_screen(
            &flow,
            NewFlowErrorOrigin::ExtrasIndexing,
            "extras build worker ended unexpectedly",
        );
    }

    #[test]
    fn new_flow_loader_progress_not_starved_by_repeated_input() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let token = flow.active_extras_index_token.expect("token");
        loader.send_collect(ExtrasLoadEvent::Collecting);
        for _ in 0..20 {
            flow.on_key(key(KeyCode::Char('x')), &ops)
                .expect("noop input");
        }
        flow.on_tick();
        assert!(matches!(
            flow.extras_indexing.as_ref().expect("indexing").phase,
            super::ExtrasIndexingPhase::Collecting
        ));

        loader.send_collect(ExtrasLoadEvent::Classifying { candidate_count: 1 });
        for _ in 0..20 {
            flow.on_key(key(KeyCode::Char('x')), &ops)
                .expect("noop input");
        }
        flow.on_tick();
        assert!(matches!(
            flow.extras_indexing.as_ref().expect("indexing").phase,
            super::ExtrasIndexingPhase::Classifying { candidate_count: 1 }
        ));

        let candidates = vec![PathBuf::from("src/main.rs")];
        loader.send_collect(ExtrasLoadEvent::DoneCollect {
            token,
            candidates: candidates.clone(),
            plan: BucketPlan::default(),
        });
        for _ in 0..20 {
            flow.on_mouse(mouse(MouseEventKind::ScrollDown));
        }
        flow.on_tick();
        assert_eq!(loader.build_call_count(), 1);

        loader.send_build(ExtrasLoadEvent::Done {
            token,
            result: Ok(build_index(&candidates)),
        });
        flow.on_tick();
        assert_eq!(flow.step, Step::ExtrasPicker);
    }

    #[test]
    fn extras_indexing_back_invalidates_token_and_ignores_stale_result() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader, &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "alpha");
        confirm_copy_extras_yes(&mut flow, &ops);

        let stale_token = flow.active_extras_index_token.expect("token");
        flow.on_key(key(KeyCode::Esc), &ops)
            .expect("cancel indexing");
        assert_eq!(flow.step, Step::CopyExtrasDecision);

        flow.apply_build_event(ExtrasLoadEvent::Done {
            token: stale_token,
            result: Err("stale".to_string()),
        });

        assert_eq!(flow.step, Step::CopyExtrasDecision);
    }

    #[test]
    fn review_executes_with_collected_inputs() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "feature1");

        confirm_copy_extras_yes(&mut flow, &ops);
        finish_indexing_without_modal(
            &mut flow,
            &ops,
            &loader,
            &[PathBuf::from("dir/sub/one.txt")],
        );

        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("connect toggle to no");
        flow.on_key(key(KeyCode::Enter), &ops).expect("review");
        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");

        assert_eq!(flow.step, Step::Success);
        assert_eq!(
            ops.execute_calls.lock().expect("execute calls lock").len(),
            1
        );
    }

    #[test]
    fn review_count_excludes_selected_extras_when_copy_disabled() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "feature2");

        confirm_copy_extras_yes(&mut flow, &ops);
        finish_indexing_without_modal(
            &mut flow,
            &ops,
            &loader,
            &[PathBuf::from("dir/sub/one.txt")],
        );
        flow.extras.select_all();
        assert!(!flow.extras.selected_for_copy().is_empty());

        flow.on_key(key(KeyCode::Esc), &ops)
            .expect("back to extras picker");
        flow.on_key(key(KeyCode::Esc), &ops)
            .expect("back to copy extras decision");
        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("toggle copy extras to no");

        assert_eq!(flow.review_selected_extras_count(), 0);
    }

    #[test]
    fn create_success_with_skip_persist_failure_stays_on_success_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.skip_registry_missing_for_indexing = true;
        ops.save_skip_buckets_error = Some("persist failed".to_string());
        *ops.always_skip_buckets.lock().expect("rules lock") =
            BTreeSet::from(["target".to_string()]);

        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader.clone(), &repo_root);
        advance_to_copy_extras_decision(&mut flow, &ops, "feature3");
        confirm_copy_extras_yes(&mut flow, &ops);

        let token = open_skip_modal(
            &mut flow,
            &loader,
            &[
                PathBuf::from("target/debug/a.o"),
                PathBuf::from("src/main.rs"),
            ],
        );
        flow.on_key(key(KeyCode::Char('a')), &ops)
            .expect("toggle persist");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("confirm skip modal");

        loader.send_build(ExtrasLoadEvent::Done {
            token,
            result: Ok(build_index(&[PathBuf::from("src/main.rs")])),
        });
        flow.on_tick();

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("continue extras");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("continue connect");
        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");

        assert_eq!(flow.step, Step::Success);
        assert!(
            flow.success_notice
                .as_deref()
                .unwrap_or("")
                .contains("failed")
        );
        assert_eq!(
            ops.execute_calls.lock().expect("execute calls lock").len(),
            1
        );

        let signal = flow
            .on_key(key(KeyCode::Enter), &ops)
            .expect("success enter");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
        assert_eq!(
            ops.execute_calls.lock().expect("execute calls lock").len(),
            1
        );
    }

    #[test]
    fn error_origin_routes_back_to_expected_step() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let loader = Arc::new(ScriptedLoader::default());
        let mut flow = new_flow(&ops, loader, &repo_root);
        flow.step = Step::ErrorScreen(NewFlowErrorState {
            origin: NewFlowErrorOrigin::ExtrasIndexing,
            message: "indexing failed".to_string(),
        });
        flow.on_key(key(KeyCode::Enter), &ops).expect("enter");
        assert_eq!(flow.step, Step::CopyExtrasDecision);

        flow.step = Step::ErrorScreen(NewFlowErrorState {
            origin: NewFlowErrorOrigin::ReviewSubmit,
            message: "submit failed".to_string(),
        });
        flow.on_key(key(KeyCode::Esc), &ops).expect("esc");
        assert_eq!(flow.step, Step::Review);
    }

    #[test]
    fn esc_on_first_step_exits_flow() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());
        let loader = Arc::new(ScriptedLoader::default());

        let mut flow = new_flow(&ops, loader, &repo_root);
        let signal = flow.on_key(key(KeyCode::Esc), &ops).expect("signal");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
    }
}
