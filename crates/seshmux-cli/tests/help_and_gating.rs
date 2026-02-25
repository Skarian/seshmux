mod support;

use predicates::prelude::*;
use std::fs;

use support::{
    assert_timestamp_log_names, init_git_repo, new_command_with_temp_home, write_valid_config,
};

#[test]
fn root_help_runs_without_config() {
    let (mut command, _temp_home) = new_command_with_temp_home();
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: seshmux"))
        .stdout(predicate::str::contains("--diagnostics"))
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("new").not())
        .stdout(predicate::str::contains("list").not())
        .stdout(predicate::str::contains("attach").not())
        .stdout(predicate::str::contains("delete").not());
}

#[test]
fn doctor_help_runs_without_config() {
    let (mut command, _temp_home) = new_command_with_temp_home();
    command
        .args(["doctor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Run environment and configuration checks",
        ));
}

#[test]
fn doctor_runs_without_config() {
    let (mut command, _temp_home) = new_command_with_temp_home();
    command
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("config file exists"))
        .stdout(predicate::str::contains(".config/seshmux/config.toml"));
}

#[test]
fn legacy_subcommands_are_rejected() {
    for subcommand in ["new", "list", "attach", "delete"] {
        let (mut command, _temp_home) = new_command_with_temp_home();
        command
            .arg(subcommand)
            .assert()
            .failure()
            .stderr(predicate::str::contains("unrecognized subcommand"));
    }
}

#[test]
fn root_command_is_gated_without_config() {
    let (mut command, _temp_home) = new_command_with_temp_home();
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing config at"))
        .stderr(predicate::str::contains(".config/seshmux/config.toml"))
        .stderr(predicate::str::contains("README.md"));
}

#[test]
fn root_command_fails_outside_git_repo_before_tui() {
    let (mut command, temp_home) = new_command_with_temp_home();
    write_valid_config(temp_home.path());

    command
        .current_dir(temp_home.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "failed to resolve git repository root",
        ));
}

#[test]
fn root_command_fails_in_repo_with_no_commits_before_tui() {
    let (mut command, temp_home) = new_command_with_temp_home();
    write_valid_config(temp_home.path());
    let repo_dir = temp_home.path().join("empty-repo");
    init_git_repo(&repo_dir);

    command
        .current_dir(&repo_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository has no commits yet"));
}

#[test]
fn doctor_with_diagnostics_creates_log_file() {
    let (mut command, temp_home) = new_command_with_temp_home();
    command
        .args(["--diagnostics", "doctor"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Diagnostics enabled:"));

    let diagnostics_dir = temp_home.path().join(".config/seshmux/diagnostics");
    let logs: Vec<_> = fs::read_dir(&diagnostics_dir)
        .expect("diagnostics dir")
        .filter_map(Result::ok)
        .collect();
    assert_timestamp_log_names(&logs);
}
