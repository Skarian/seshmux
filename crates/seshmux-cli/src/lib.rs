pub mod cli;
pub mod dispatch;
pub mod prompt;

use anyhow::Result;
use clap::Parser;
use prompt::InquirePromptDriver;
use seshmux_core::command_runner::SystemCommandRunner;

use crate::cli::Cli;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut prompt_driver = InquirePromptDriver::new();
    let command_runner = SystemCommandRunner::new();
    dispatch::run_with_deps(cli, &mut prompt_driver, &command_runner)
}
