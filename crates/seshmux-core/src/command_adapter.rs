use std::path::Path;

use crate::command_runner::{CommandOutput, CommandRunner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandFailure {
    pub(crate) command: String,
    pub(crate) status: i32,
    pub(crate) stderr: String,
}

pub(crate) fn run_program(
    runner: &dyn CommandRunner,
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<CommandOutput, String> {
    runner
        .run(program, args, cwd)
        .map_err(|error| error.to_string())
}

pub(crate) fn ensure_success(
    args: &[&str],
    output: CommandOutput,
) -> Result<CommandOutput, CommandFailure> {
    if output.status_code == 0 {
        return Ok(output);
    }

    Err(CommandFailure {
        command: args.join(" "),
        status: output.status_code,
        stderr: output.stderr.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use crate::command_runner::CommandOutput;

    use super::{CommandFailure, ensure_success};

    #[test]
    fn ensure_success_returns_failure_shape() {
        let result = ensure_success(
            &["worktree", "remove", "foo"],
            CommandOutput {
                status_code: 128,
                stdout: String::new(),
                stderr: "fatal: failed".to_string(),
            },
        );

        assert_eq!(
            result.expect_err("expected failure"),
            CommandFailure {
                command: "worktree remove foo".to_string(),
                status: 128,
                stderr: "fatal: failed".to_string(),
            }
        );
    }
}
