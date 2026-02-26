use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::anyhow;

use crate::command_runner::{CommandOutput, CommandRunner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub interactive: bool,
}

#[derive(Default)]
pub struct RecordingRunner {
    outputs: Mutex<VecDeque<anyhow::Result<CommandOutput>>>,
    interactive_statuses: Mutex<VecDeque<anyhow::Result<i32>>>,
    calls: Mutex<Vec<Call>>,
}

impl RecordingRunner {
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

    pub fn from_outputs(outputs: Vec<anyhow::Result<CommandOutput>>) -> Self {
        Self::new(outputs, Vec::new())
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl CommandRunner for RecordingRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> anyhow::Result<CommandOutput> {
        self.calls.lock().expect("calls lock").push(Call {
            program: program.to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            cwd: cwd.map(|value| value.to_path_buf()),
            interactive: false,
        });

        self.outputs
            .lock()
            .expect("outputs lock")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow!("missing output")))
    }

    fn run_interactive(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> anyhow::Result<i32> {
        self.calls.lock().expect("calls lock").push(Call {
            program: program.to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            cwd: cwd.map(|value| value.to_path_buf()),
            interactive: true,
        });

        self.interactive_statuses
            .lock()
            .expect("interactive lock")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow!("missing interactive status")))
    }
}

pub fn output(stdout: &str, stderr: &str, status_code: i32) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput {
        status_code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    })
}
