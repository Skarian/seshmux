use std::path::Path;

use thiserror::Error;

use crate::command_runner::CommandRunner;
use crate::config::WindowSpec;
use crate::names::sanitize_repo_component;

#[derive(Debug, Error)]
pub enum TmuxError {
    #[error("failed to execute tmux command: {0}")]
    Execute(String),
    #[error("tmux command failed: tmux {command} (exit {status}) {stderr}")]
    CommandFailed {
        command: String,
        status: i32,
        stderr: String,
    },
    #[error("tmux window '{window}' has invalid launch mode")]
    InvalidWindowMode { window: String },
    #[error("worktree path is not valid UTF-8")]
    InvalidPath,
}

pub fn session_name(repo_name: &str, worktree_name: &str) -> String {
    format!("{}/{}", sanitize_repo_component(repo_name), worktree_name)
}

pub fn create_session_and_windows(
    session: &str,
    cwd: &Path,
    windows: &[WindowSpec],
    runner: &dyn CommandRunner,
) -> Result<(), TmuxError> {
    if windows.is_empty() {
        return Err(TmuxError::InvalidWindowMode {
            window: "<missing>".to_string(),
        });
    }

    let cwd_value = cwd.to_str().ok_or(TmuxError::InvalidPath)?;

    let first = &windows[0];
    let first_launch = build_window_launch(first)?;

    let mut create_args = vec![
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        session.to_string(),
        "-c".to_string(),
        cwd_value.to_string(),
        "-n".to_string(),
        first.name.clone(),
    ];
    create_args.extend(first_launch);

    run_tmux_checked(runner, &create_args, None)?;

    for window in windows.iter().skip(1) {
        let launch = build_window_launch(window)?;
        let mut args = vec![
            "new-window".to_string(),
            "-t".to_string(),
            session.to_string(),
            "-c".to_string(),
            cwd_value.to_string(),
            "-n".to_string(),
            window.name.clone(),
        ];
        args.extend(launch);

        run_tmux_checked(runner, &args, None)?;
    }

    Ok(())
}

pub fn session_exists(session: &str, runner: &dyn CommandRunner) -> Result<bool, TmuxError> {
    let args = ["has-session", "-t", session];
    let output = run_tmux(runner, &args, None)?;

    Ok(output.status_code == 0)
}

pub fn connect_session(
    session: &str,
    inside_tmux: bool,
    runner: &dyn CommandRunner,
) -> Result<(), TmuxError> {
    let args = if inside_tmux {
        vec!["switch-client", "-t", session]
    } else {
        vec!["attach-session", "-t", session]
    };

    let status = runner
        .run_interactive("tmux", &args, None)
        .map_err(|error| TmuxError::Execute(error.to_string()))?;

    if status != 0 {
        return Err(TmuxError::CommandFailed {
            command: args.join(" "),
            status,
            stderr: String::new(),
        });
    }

    Ok(())
}

pub fn kill_session(session: &str, runner: &dyn CommandRunner) -> Result<(), TmuxError> {
    run_tmux_checked(runner, &["kill-session", "-t", session], None)?;
    Ok(())
}

fn build_window_launch(window: &WindowSpec) -> Result<Vec<String>, TmuxError> {
    let direct_mode = window.program.is_some() || window.args.is_some();
    let shell_mode = window.shell.is_some() || window.command.is_some();

    if direct_mode && shell_mode {
        return Err(TmuxError::InvalidWindowMode {
            window: window.name.clone(),
        });
    }

    if !direct_mode && !shell_mode {
        return Err(TmuxError::InvalidWindowMode {
            window: window.name.clone(),
        });
    }

    if direct_mode {
        let program = window
            .program
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| TmuxError::InvalidWindowMode {
                window: window.name.clone(),
            })?;

        let mut parts = vec![program];
        if let Some(args) = &window.args {
            parts.extend(args.clone());
        }
        return Ok(parts);
    }

    let shell = window
        .shell
        .clone()
        .filter(|values| !values.is_empty())
        .ok_or_else(|| TmuxError::InvalidWindowMode {
            window: window.name.clone(),
        })?;

    if shell[0].trim().is_empty() {
        return Err(TmuxError::InvalidWindowMode {
            window: window.name.clone(),
        });
    }

    let command = window
        .command
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| TmuxError::InvalidWindowMode {
            window: window.name.clone(),
        })?;

    let mut parts = shell;
    parts.push(command);

    Ok(parts)
}

fn run_tmux_checked(
    runner: &dyn CommandRunner,
    args: &[impl AsRef<str>],
    cwd: Option<&Path>,
) -> Result<(), TmuxError> {
    let arg_refs: Vec<&str> = args.iter().map(|value| value.as_ref()).collect();
    let output = run_tmux(runner, &arg_refs, cwd)?;

    if output.status_code != 0 {
        return Err(TmuxError::CommandFailed {
            command: arg_refs.join(" "),
            status: output.status_code,
            stderr: output.stderr.trim().to_string(),
        });
    }

    Ok(())
}

fn run_tmux(
    runner: &dyn CommandRunner,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<crate::command_runner::CommandOutput, TmuxError> {
    runner
        .run("tmux", args, cwd)
        .map_err(|error| TmuxError::Execute(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use anyhow::anyhow;

    use crate::command_runner::{CommandOutput, CommandRunner};
    use crate::config::WindowSpec;

    use super::*;

    #[derive(Debug, Clone)]
    struct Call {
        program: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        interactive: bool,
    }

    #[derive(Default)]
    struct RecordingRunner {
        outputs: Mutex<VecDeque<anyhow::Result<CommandOutput>>>,
        interactive_statuses: Mutex<VecDeque<anyhow::Result<i32>>>,
        calls: Mutex<Vec<Call>>,
    }

    impl RecordingRunner {
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
            self.calls.lock().expect("lock calls").clone()
        }
    }

    impl CommandRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            cwd: Option<&Path>,
        ) -> anyhow::Result<CommandOutput> {
            self.calls.lock().expect("lock calls").push(Call {
                program: program.to_string(),
                args: args.iter().map(|value| (*value).to_string()).collect(),
                cwd: cwd.map(|value| value.to_path_buf()),
                interactive: false,
            });

            self.outputs
                .lock()
                .expect("lock outputs")
                .pop_front()
                .unwrap_or_else(|| Err(anyhow!("missing output")))
        }

        fn run_interactive(
            &self,
            program: &str,
            args: &[&str],
            cwd: Option<&Path>,
        ) -> anyhow::Result<i32> {
            self.calls.lock().expect("lock calls").push(Call {
                program: program.to_string(),
                args: args.iter().map(|value| (*value).to_string()).collect(),
                cwd: cwd.map(|value| value.to_path_buf()),
                interactive: true,
            });

            self.interactive_statuses
                .lock()
                .expect("lock statuses")
                .pop_front()
                .unwrap_or_else(|| Err(anyhow!("missing interactive status")))
        }
    }

    fn output(status_code: i32) -> anyhow::Result<CommandOutput> {
        Ok(CommandOutput {
            status_code,
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn direct_window() -> WindowSpec {
        WindowSpec {
            name: "editor".to_string(),
            program: Some("nvim".to_string()),
            args: Some(vec![".".to_string()]),
            shell: None,
            command: None,
        }
    }

    fn shell_window() -> WindowSpec {
        WindowSpec {
            name: "ops".to_string(),
            program: None,
            args: None,
            shell: Some(vec!["/bin/zsh".to_string(), "-lc".to_string()]),
            command: Some("echo ready".to_string()),
        }
    }

    #[test]
    fn session_name_uses_repo_and_worktree_format() {
        assert_eq!(session_name("My Repo", "feature-a"), "my-repo/feature-a");
    }

    #[test]
    fn create_session_and_windows_builds_direct_and_shell_commands() {
        let runner = RecordingRunner::new(vec![output(0), output(0)], Vec::new());
        let cwd = PathBuf::from("/tmp/project/worktrees/w1");

        create_session_and_windows(
            "project/w1",
            &cwd,
            &[direct_window(), shell_window()],
            &runner,
        )
        .expect("create session");

        let calls = runner.calls();
        assert_eq!(calls.len(), 2);

        assert_eq!(calls[0].program, "tmux");
        assert!(calls[0].args.starts_with(&[
            "new-session".to_string(),
            "-d".to_string(),
            "-s".to_string(),
            "project/w1".to_string()
        ]));
        assert_eq!(calls[0].cwd, None);
        assert!(calls[0].args.contains(&"nvim".to_string()));
        assert!(calls[0].args.contains(&".".to_string()));
        assert!(!calls[0].interactive);

        assert!(calls[1].args.starts_with(&[
            "new-window".to_string(),
            "-t".to_string(),
            "project/w1".to_string()
        ]));
        assert!(calls[1].args.contains(&"/bin/zsh".to_string()));
        assert!(calls[1].args.contains(&"-lc".to_string()));
        assert!(calls[1].args.contains(&"echo ready".to_string()));
        assert!(!calls[1].interactive);
    }

    #[test]
    fn connect_session_uses_interactive_runner() {
        let runner = RecordingRunner::new(Vec::new(), vec![Ok(0)]);
        connect_session("repo/w1", false, &runner).expect("connect");

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].interactive);
        assert_eq!(
            calls[0].args,
            vec![
                "attach-session".to_string(),
                "-t".to_string(),
                "repo/w1".to_string()
            ]
        );
    }

    #[test]
    fn kill_session_invokes_tmux_kill_session() {
        let runner = RecordingRunner::new(vec![output(0)], Vec::new());
        kill_session("repo/w1", &runner).expect("kill");

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].args,
            vec![
                "kill-session".to_string(),
                "-t".to_string(),
                "repo/w1".to_string()
            ]
        );
    }
}
