use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::anyhow;
use seshmux_app::{App, AttachError, AttachRequest, DeleteRequest};
use seshmux_core::command_runner::{CommandOutput, CommandRunner};
use seshmux_core::registry::{RegistryEntry, insert_unique_entry, load_registry};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
struct Call {
    program: String,
    args: Vec<String>,
    interactive: bool,
}

#[derive(Default)]
struct QueueRunner {
    outputs: Mutex<VecDeque<anyhow::Result<CommandOutput>>>,
    interactive_statuses: Mutex<VecDeque<anyhow::Result<i32>>>,
    calls: Mutex<Vec<Call>>,
}

impl QueueRunner {
    fn new(
        outputs: Vec<anyhow::Result<CommandOutput>>,
        interactive_statuses: Vec<anyhow::Result<i32>>,
    ) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            interactive_statuses: Mutex::new(interactive_statuses.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl CommandRunner for QueueRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _cwd: Option<&Path>,
    ) -> anyhow::Result<CommandOutput> {
        self.calls.lock().expect("calls lock").push(Call {
            program: program.to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            interactive: false,
        });

        self.outputs
            .lock()
            .expect("outputs lock")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow!("missing scripted output")))
    }

    fn run_interactive(
        &self,
        program: &str,
        args: &[&str],
        _cwd: Option<&Path>,
    ) -> anyhow::Result<i32> {
        self.calls.lock().expect("calls lock").push(Call {
            program: program.to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            interactive: true,
        });

        self.interactive_statuses
            .lock()
            .expect("interactive lock")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow!("missing scripted interactive status")))
    }
}

fn output(stdout: &str, stderr: &str, status: i32) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput {
        status_code: status,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    })
}

fn write_valid_config(home: &Path) {
    let config_dir = home.join(".config").join("seshmux");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.toml"),
        r#"
version = 1

[[tmux.windows]]
name = "editor"
program = "nvim"
args = []

[[tmux.windows]]
name = "git"
program = "lazygit"
args = []
"#,
    )
    .expect("write config");
}

fn add_registry_entry(repo_root: &Path, name: &str, created_at: &str) -> PathBuf {
    let path = repo_root.join("worktrees").join(name);
    fs::create_dir_all(&path).expect("create worktree dir");

    insert_unique_entry(
        repo_root,
        RegistryEntry {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            created_at: created_at.to_string(),
        },
    )
    .expect("insert registry");

    path
}

#[test]
fn list_sorts_by_recency_and_includes_runtime_fields() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    let old_path = add_registry_entry(&repo_root, "old", "2026-02-24T10:00:00Z");
    let new_path = add_registry_entry(&repo_root, "new", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("old-branch\n", "", 0),
            output("", "missing session", 1),
            output("new-branch\n", "", 0),
            output("", "", 0),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let result = app.list(&repo_root).expect("list result");

    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0].name, "new");
    assert_eq!(result.rows[0].path, new_path);
    assert_eq!(result.rows[0].branch, "new-branch");
    assert!(result.rows[0].session_running);
    assert_eq!(result.rows[1].name, "old");
    assert_eq!(result.rows[1].path, old_path);
    assert_eq!(result.rows[1].branch, "old-branch");
    assert!(!result.rows[1].session_running);
}

#[test]
fn attach_returns_missing_session_error_when_create_is_false() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "missing session", 1),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let error = app
        .attach(AttachRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            create_if_missing: false,
        })
        .expect_err("missing session error");

    let typed = error
        .downcast_ref::<AttachError>()
        .expect("typed attach error");
    assert!(matches!(typed, AttachError::MissingSession { .. }));
}

#[test]
fn attach_creates_session_when_missing_and_connects() {
    let _guard = ENV_LOCK.lock().expect("env lock");

    let temp = tempfile::tempdir().expect("temp dir");
    write_valid_config(temp.path());
    unsafe {
        std::env::set_var("HOME", temp.path());
    }

    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "missing session", 1),
            output("", "", 0),
            output("", "", 0),
        ],
        vec![Ok(0)],
    );

    let app = App::new(&runner);
    let result = app
        .attach(AttachRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            create_if_missing: true,
        })
        .expect("attach result");

    assert!(result.created_session);
    assert_eq!(result.worktree_name, "w1");

    let calls = runner.calls();
    assert!(calls.iter().any(|call| {
        call.program == "tmux"
            && call.args.first().map(|value| value.as_str()) == Some("new-session")
            && !call.interactive
    }));
    assert!(calls.iter().any(|call| {
        call.program == "tmux"
            && matches!(
                call.args.first().map(|value| value.as_str()),
                Some("attach-session") | Some("switch-client")
            )
            && call.interactive
    }));
}

#[test]
fn delete_with_all_options_kills_session_removes_worktree_and_branch() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    let worktree_path = add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "", 0),
            output("", "", 0),
            output("", "", 0),
            output("", "", 0),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let result = app
        .delete(DeleteRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            kill_tmux_session: true,
            delete_branch: true,
        })
        .expect("delete result");

    assert_eq!(result.worktree_path, worktree_path);
    assert!(result.branch_deleted);
    assert!(load_registry(&repo_root).expect("registry load").is_empty());
}

#[test]
fn delete_keeps_branch_when_not_fully_merged() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "", 0),
            output("", "error: the branch 'w1' is not fully merged.", 1),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let result = app
        .delete(DeleteRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            kill_tmux_session: false,
            delete_branch: true,
        })
        .expect("delete should still succeed");

    assert!(!result.branch_deleted);
    assert!(load_registry(&repo_root).expect("registry load").is_empty());
}
