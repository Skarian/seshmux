use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::anyhow;
use seshmux_core::command_runner::{CommandOutput, CommandRunner};

pub static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct Call {
    pub program: String,
    pub args: Vec<String>,
    #[allow(dead_code)]
    pub interactive: bool,
}

#[derive(Default)]
pub struct QueueRunner {
    outputs: Mutex<VecDeque<anyhow::Result<CommandOutput>>>,
    interactive_statuses: Mutex<VecDeque<anyhow::Result<i32>>>,
    calls: Mutex<Vec<Call>>,
}

impl QueueRunner {
    pub fn new(
        outputs: Vec<anyhow::Result<CommandOutput>>,
        interactive_statuses: Vec<anyhow::Result<i32>>,
    ) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            interactive_statuses: Mutex::new(interactive_statuses.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<Call> {
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

pub fn output(stdout: &str, stderr: &str, status: i32) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput {
        status_code: status,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    })
}

pub fn write_valid_config(home: &Path, include_git_window: bool) {
    let config_dir = home.join(".config").join("seshmux");
    fs::create_dir_all(&config_dir).expect("create config dir");

    let mut config = r#"
version = 1

[[tmux.windows]]
name = "editor"
program = "nvim"
args = []
"#
    .to_string();

    if include_git_window {
        config.push_str(
            r#"

[[tmux.windows]]
name = "git"
program = "lazygit"
args = []
"#,
        );
    }

    fs::write(config_dir.join("config.toml"), config).expect("write config");
}

#[allow(dead_code)]
pub fn add_registry_entry(repo_root: &Path, name: &str, created_at: &str) -> PathBuf {
    let path = repo_root.join("worktrees").join(name);
    fs::create_dir_all(&path).expect("create worktree dir");

    seshmux_core::registry::insert_unique_entry(
        repo_root,
        seshmux_core::registry::RegistryEntry {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            created_at: created_at.to_string(),
        },
    )
    .expect("insert registry");

    path
}
