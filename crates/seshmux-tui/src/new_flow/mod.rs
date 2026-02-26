mod extras;
mod keys;
mod picker;
mod render;

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use crossterm::event::KeyEvent;
use seshmux_app::{App, NewPrepare, NewRequest, NewResult, NewStartPoint};
use seshmux_core::git::{BranchRef, CommitRef};
use tui_input::Input;

use crate::UiExit;
use crate::ui::binary_choice::BinaryChoice;

use self::extras::ExtrasState;
use self::picker::PickerState;

pub(crate) trait NewFlowOps {
    fn prepare(&self, cwd: &Path) -> Result<NewPrepare>;
    fn query_branches(&self, repo_root: &Path, query: &str) -> Result<Vec<BranchRef>>;
    fn query_commits(&self, repo_root: &Path, query: &str, limit: usize) -> Result<Vec<CommitRef>>;
    fn list_extras(&self, repo_root: &Path) -> Result<Vec<PathBuf>>;
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

    fn list_extras(&self, repo_root: &Path) -> Result<Vec<PathBuf>> {
        self.new_list_extras(repo_root)
    }

    fn execute_new(&self, request: NewRequest) -> Result<NewResult> {
        self.new_execute(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    GitignoreDecision,
    NameInput,
    StartPointMode,
    BranchPicker,
    BranchSearchInput,
    CommitPicker,
    CommitSearchInput,
    ExtrasPicker,
    ConnectNow,
    Review,
    Success,
    ErrorScreen,
}

#[derive(Debug)]
struct NewFlow {
    cwd: PathBuf,
    prepare: NewPrepare,
    step: Step,
    gitignore_choice: BinaryChoice,
    name_input: Input,
    name_error: Option<String>,
    start_mode_selected: usize,
    start_point: Option<NewStartPoint>,
    branch_picker: Option<PickerState<BranchRef>>,
    branch_search_input: Input,
    commit_picker: Option<PickerState<CommitRef>>,
    commit_search_input: Input,
    extras: ExtrasState,
    connect_choice: BinaryChoice,
    success: Option<NewResult>,
    error_message: Option<String>,
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
}

impl NewFlow {
    fn new(ops: &dyn NewFlowOps, cwd: &Path) -> Result<Self> {
        let prepare = ops.prepare(cwd)?;
        let commits = ops.query_commits(&prepare.repo_root, "", 1)?;
        if commits.is_empty() {
            return Err(anyhow!(
                "current branch/HEAD has no commits yet; create an initial commit on this branch before starting seshmux"
            ));
        }
        let candidates = ops.list_extras(&prepare.repo_root)?;
        let extras = ExtrasState::from_candidates(&prepare.repo_root, &candidates)?;

        let first_step = if prepare.gitignore_has_worktrees_entry {
            Step::NameInput
        } else {
            Step::GitignoreDecision
        };

        Ok(Self {
            cwd: cwd.to_path_buf(),
            prepare,
            step: first_step,
            gitignore_choice: BinaryChoice::new(true),
            name_input: Input::default(),
            name_error: None,
            start_mode_selected: 0,
            start_point: None,
            branch_picker: None,
            branch_search_input: Input::default(),
            commit_picker: None,
            commit_search_input: Input::default(),
            extras,
            connect_choice: BinaryChoice::new(true),
            success: None,
            error_message: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use seshmux_app::{NewPrepare, NewRequest, NewResult, NewStartPoint};
    use seshmux_core::git::{BranchRef, BranchSource, CommitRef};

    use super::{FlowSignal, NewFlow, NewFlowOps, Step};

    struct FakeOps {
        prepare: NewPrepare,
        branches: Vec<BranchRef>,
        latest_commits: Vec<CommitRef>,
        searched_commits: Vec<CommitRef>,
        extras: Vec<PathBuf>,
        execute_calls: RefCell<Vec<NewRequest>>,
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
                extras: Vec::new(),
                execute_calls: RefCell::new(Vec::new()),
            }
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

        fn list_extras(&self, _repo_root: &Path) -> Result<Vec<PathBuf>> {
            Ok(self.extras.clone())
        }

        fn execute_new(&self, request: NewRequest) -> Result<NewResult> {
            self.execute_calls.borrow_mut().push(request.clone());
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

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn render_output(flow: &NewFlow, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| flow.render(frame))
            .expect("render new flow");
        format!("{}", terminal.backend())
    }

    #[test]
    fn esc_on_first_step_exits_flow() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        let signal = flow.on_key(key(KeyCode::Esc), &ops).expect("signal");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
    }

    #[test]
    fn new_flow_requires_at_least_one_commit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.latest_commits.clear();

        let error = NewFlow::new(&ops, &repo_root).expect_err("flow should fail");
        assert!(
            error
                .to_string()
                .contains("current branch/HEAD has no commits yet")
        );
    }

    #[test]
    fn commit_picker_shows_latest_then_search_results() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "test".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Down), &ops)
            .expect("select branch mode");
        flow.on_key(key(KeyCode::Down), &ops)
            .expect("select commit mode");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("enter commit picker");

        assert_eq!(flow.step, Step::CommitPicker);
        assert_eq!(
            flow.commit_picker
                .as_ref()
                .expect("commit picker")
                .items
                .first()
                .expect("latest")
                .short_hash,
            "aaaaaaa"
        );

        flow.on_key(key(KeyCode::Enter), &ops).expect("open search");
        assert_eq!(flow.step, Step::CommitSearchInput);

        for character in "bbb".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("type search");
        }
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("apply search");
        assert_eq!(flow.step, Step::CommitPicker);
        assert_eq!(
            flow.commit_picker
                .as_ref()
                .expect("commit picker")
                .items
                .first()
                .expect("searched")
                .short_hash,
            "bbbbbbb"
        );
    }

    #[test]
    fn extras_filter_preserves_selection_state() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join("dir/sub")).expect("dirs");
        std::fs::write(repo_root.join("dir/sub/one.txt"), "one").expect("file one");
        std::fs::write(repo_root.join("dir/sub/two.txt"), "two").expect("file two");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.extras = vec![PathBuf::from("dir")];

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "abc".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("current mode");

        assert_eq!(flow.step, Step::ExtrasPicker);
        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("toggle root dir");
        let before = flow.extras.selected_for_copy();
        assert!(before.iter().any(|path| path == &PathBuf::from("dir")));

        flow.on_key(key(KeyCode::Char('/')), &ops)
            .expect("enter filter edit");
        for character in "one".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("type filter");
        }
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("finish filter");

        let after = flow.extras.selected_for_copy();
        assert_eq!(before, after);
    }

    #[test]
    fn branch_picker_no_results_does_not_advance_to_extras() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.branches.clear();

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "featurex".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Down), &ops)
            .expect("select branch mode");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("open branch picker");
        assert_eq!(flow.step, Step::BranchPicker);

        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("open branch search input");
        assert_eq!(flow.step, Step::BranchSearchInput);
        flow.on_key(key(KeyCode::Char('x')), &ops)
            .expect("search char");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("apply empty search");
        assert_eq!(flow.step, Step::BranchPicker);

        flow.on_key(key(KeyCode::Down), &ops)
            .expect("select show-all action");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("show-all action");
        assert_eq!(flow.step, Step::BranchPicker);
        assert!(flow.start_point.is_none());
    }

    #[test]
    fn extras_space_noops_when_filtered_view_is_empty() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join("dir/sub")).expect("dirs");
        std::fs::write(repo_root.join("dir/sub/one.txt"), "one").expect("file one");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.extras = vec![PathBuf::from("dir")];

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "abc".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("current mode");
        assert_eq!(flow.step, Step::ExtrasPicker);

        flow.on_key(key(KeyCode::Char('/')), &ops)
            .expect("enter filter edit");
        for character in "zzz".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("type unmatched filter");
        }
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("finish filter");
        assert!(flow.extras.visible.is_empty());

        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("space on empty extras view");
        assert_eq!(flow.step, Step::ExtrasPicker);
    }

    #[test]
    fn extras_tab_folds_directory_without_leaving_step() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join("dir/sub")).expect("dirs");
        std::fs::write(repo_root.join("dir/sub/one.txt"), "one").expect("file one");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.extras = vec![PathBuf::from("dir")];

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "abc".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("current mode");
        assert_eq!(flow.step, Step::ExtrasPicker);

        let before = flow.extras.visible.len();
        flow.on_key(key(KeyCode::Tab), &ops)
            .expect("fold current directory");
        let after = flow.extras.visible.len();

        assert_eq!(flow.step, Step::ExtrasPicker);
        assert!(after < before);
    }

    #[test]
    fn review_executes_with_collected_inputs() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "feature1".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("start current");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("extras continue");
        flow.on_key(key(KeyCode::Char(' ')), &ops)
            .expect("connect toggle to no");
        flow.on_key(key(KeyCode::Enter), &ops).expect("review");
        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");

        assert_eq!(flow.step, Step::Success);
        let calls = ops.execute_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].worktree_name, "feature1");
        assert_eq!(calls[0].start_point, NewStartPoint::CurrentBranch);
        assert!(!calls[0].connect_now);
    }

    #[test]
    fn error_modal_wraps_long_message() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.step = Step::ErrorScreen;
        flow.error_message = Some(
            "failed to create worktree because this is a deliberately long explanation that should wrap and retain TOKEN_WRAP_NEW in view".to_string(),
        );

        let output = render_output(&flow, 62, 20);
        assert!(output.contains("TOKEN_WRAP_NEW"));
    }
}
