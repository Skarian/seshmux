use anyhow::{Context, Result, anyhow, bail};
use comfy_table::{Cell, ContentArrangement, Table};
use seshmux_core::command_runner::CommandRunner;
use seshmux_core::config::{load_config, resolve_config_path};
use seshmux_core::doctor::{CheckState, DoctorReport, run_doctor_with_runner};

use crate::cli::{Cli, Command};
use crate::prompt::PromptDriver;

pub fn run_with_deps(
    cli: Cli,
    prompt: &mut dyn PromptDriver,
    command_runner: &dyn CommandRunner,
) -> Result<()> {
    match cli.command {
        Command::Doctor => run_doctor_command(command_runner),
        Command::New => {
            enforce_config_gating()?;
            run_runtime_stub("new", prompt, command_runner)
        }
        Command::List => {
            enforce_config_gating()?;
            run_runtime_stub("list", prompt, command_runner)
        }
        Command::Attach => {
            enforce_config_gating()?;
            run_runtime_stub("attach", prompt, command_runner)
        }
        Command::Delete => {
            enforce_config_gating()?;
            run_runtime_stub("delete", prompt, command_runner)
        }
    }
}

fn enforce_config_gating() -> Result<()> {
    let config_path = resolve_config_path().context("failed to resolve config path")?;

    if !config_path.exists() {
        bail!(
            "missing config at {}\nCreate ~/.config/seshmux/config.toml and see README.md for setup instructions.",
            config_path.display()
        );
    }

    load_config(&config_path).map_err(|error| {
        anyhow!(
            "invalid config at {}: {error}\nFix the config and retry. See README.md for setup instructions.",
            config_path.display()
        )
    })?;

    Ok(())
}

fn run_runtime_stub(
    command_name: &str,
    _prompt: &mut dyn PromptDriver,
    _command_runner: &dyn CommandRunner,
) -> Result<()> {
    bail!("{command_name} is not implemented in this milestone")
}

fn run_doctor_command(command_runner: &dyn CommandRunner) -> Result<()> {
    let report = run_doctor_with_runner(command_runner);
    print_doctor_report(&report);
    Ok(())
}

fn print_doctor_report(report: &DoctorReport) {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec!["Check", "Status", "Details"]);

    for check in &report.checks {
        let status = match check.state {
            CheckState::Pass => "PASS",
            CheckState::Fail => "FAIL",
        };

        table.add_row(vec![
            Cell::new(check.name.as_str()),
            Cell::new(status),
            Cell::new(check.details.as_str()),
        ]);
    }

    println!("{table}");
    println!("{}", report.summary());
}
