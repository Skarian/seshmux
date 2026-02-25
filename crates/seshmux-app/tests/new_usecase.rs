use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use anyhow::anyhow;
use seshmux_app::{App, NewRequest, NewStartPoint};
use seshmux_core::command_runner::{CommandOutput, CommandRunner};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
struct Call {
    program: String,
    args: Vec<String>,
}

#[derive(Default)]
struct QueueRunner {
    outputs: Mutex<VecDeque<anyhow::Result<CommandOutput>>>,
    calls: Mutex<Vec<Call>>,
}

impl QueueRunner {
    fn new(outputs: Vec<anyhow::Result<CommandOutput>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
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
        });

        self.outputs
            .lock()
            .expect("outputs lock")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow!("missing scripted output")))
    }

    fn run_interactive(
        &self,
        _program: &str,
        _args: &[&str],
        _cwd: Option<&Path>,
    ) -> anyhow::Result<i32> {
        Err(anyhow!("interactive command not expected in this test"))
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
"#,
    )
    .expect("write config");
}

#[test]
fn new_execute_returns_no_commits_error_for_current_branch_start() {
    let _guard = ENV_LOCK.lock().expect("env lock");

    let temp = tempfile::tempdir().expect("temp dir");
    write_valid_config(temp.path());
    unsafe {
        std::env::set_var("HOME", temp.path());
    }

    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("repo dir");

    let runner = QueueRunner::new(vec![
        output(&format!("{}\n", repo_root.display()), "", 0),
        output(
            "",
            "fatal: ambiguous argument 'HEAD': unknown revision or path not in the working tree.",
            128,
        ),
    ]);

    let app = App::new(&runner);
    let error = app
        .new_execute(NewRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            start_point: NewStartPoint::CurrentBranch,
            add_worktrees_gitignore_entry: false,
            selected_extras: Vec::new(),
            connect_now: false,
        })
        .expect_err("expected no commits error");

    assert!(error.to_string().contains(
        "repository has no commits yet; create an initial commit or choose a different start point"
    ));
}

#[test]
fn new_execute_rejects_duplicate_registry_name_before_second_mutation() {
    let _guard = ENV_LOCK.lock().expect("env lock");

    let temp = tempfile::tempdir().expect("temp dir");
    write_valid_config(temp.path());
    unsafe {
        std::env::set_var("HOME", temp.path());
    }

    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("repo dir");

    let runner = QueueRunner::new(vec![
        output(&format!("{}\n", repo_root.display()), "", 0),
        output("", "", 0),
        output("", "", 0),
        output(&format!("{}\n", repo_root.display()), "", 0),
    ]);

    let app = App::new(&runner);

    let first = app.new_execute(NewRequest {
        cwd: repo_root.clone(),
        worktree_name: "w1".to_string(),
        start_point: NewStartPoint::Commit("abc123".to_string()),
        add_worktrees_gitignore_entry: false,
        selected_extras: Vec::new(),
        connect_now: false,
    });
    assert!(first.is_ok());

    let second = app.new_execute(NewRequest {
        cwd: repo_root,
        worktree_name: "w1".to_string(),
        start_point: NewStartPoint::Commit("abc123".to_string()),
        add_worktrees_gitignore_entry: false,
        selected_extras: Vec::new(),
        connect_now: false,
    });

    assert!(second.is_err());
    let message = second.expect_err("second run should fail").to_string();
    assert!(message.contains("registry already has a conflicting worktree entry"));

    let calls = runner.calls();
    let worktree_add_calls = calls
        .iter()
        .filter(|call| {
            call.program == "git"
                && call
                    .args
                    .starts_with(&["worktree".to_string(), "add".to_string()])
        })
        .count();

    assert_eq!(worktree_add_calls, 1);
}
