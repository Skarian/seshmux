pub mod cli;
pub mod dispatch;

use anyhow::{Context, Result};
use clap::Parser;
use seshmux_app::App;
use seshmux_core::command_runner::SystemCommandRunner;

use crate::cli::Cli;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let command_runner = SystemCommandRunner::new();
    let app = App::new(&command_runner);
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    dispatch::run_with_deps(cli, &app, &cwd)
}
