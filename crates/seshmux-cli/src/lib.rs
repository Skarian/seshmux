pub mod cli;
pub mod diagnostics;
pub mod dispatch;

use anyhow::{Context, Result};
use clap::Parser;
use seshmux_app::App;
use seshmux_core::command_runner::SystemCommandRunner;

use crate::cli::Cli;
use crate::diagnostics::DiagnosticsSession;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let diagnostics = DiagnosticsSession::initialize(cli.diagnostics)?;
    diagnostics.record(format!("cli parsed: command={:?}", cli.command));
    if let Some(path) = diagnostics.path() {
        eprintln!("Diagnostics enabled: {}", path.display());
    }

    let command_runner = SystemCommandRunner::new();
    diagnostics.record("command runner initialized");
    let app = App::new(&command_runner);
    diagnostics.record("app initialized");
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    diagnostics.record(format!("cwd={}", cwd.display()));

    let result = dispatch::run_with_deps(cli, &app, &cwd);
    match &result {
        Ok(()) => diagnostics.record("command completed successfully"),
        Err(error) => diagnostics.record(format!("command failed: {error:#}")),
    }

    result
}
