use std::path::Path;

use anyhow::Result;
use comfy_table::{Cell, ContentArrangement, Table};
use seshmux_app::App;
use seshmux_core::doctor::{CheckState, DoctorReport};

use crate::cli::{Cli, Command};

pub fn run_with_deps(cli: Cli, app: &App<'_>, cwd: &Path) -> Result<()> {
    match cli.command {
        Some(Command::Doctor) => run_doctor_command(app),
        None => run_root_command(app, cwd),
    }
}

fn run_root_command(app: &App<'_>, cwd: &Path) -> Result<()> {
    app.ensure_config_ready()?;
    app.ensure_runtime_repo_ready(cwd)?;

    let _ = seshmux_tui::run_root(app, cwd)?;

    Ok(())
}

fn run_doctor_command(app: &App<'_>) -> Result<()> {
    let report = app.doctor()?;
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
