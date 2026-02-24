use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn new_command_with_temp_home() -> (Command, tempfile::TempDir) {
    let temp_home = tempfile::tempdir().expect("temp home");
    let binary = assert_cmd::cargo::cargo_bin!("seshmux");
    let mut command = Command::new(binary);
    command.env("HOME", temp_home.path());
    command.env("XDG_CONFIG_HOME", temp_home.path().join(".config"));
    (command, temp_home)
}

fn write_valid_config(home: &Path) {
    let config_dir = home.join(".config").join("seshmux");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.toml"),
        r#"
version = 1

[[tmux.windows]]
name = "editor"
program = "nvim"
args = []
"#,
    )
    .expect("write config");
}

#[test]
fn root_help_runs_without_config() {
    let (mut command, _temp_home) = new_command_with_temp_home();
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: seshmux"));
}

#[test]
fn subcommand_help_runs_without_config() {
    let (mut command, _temp_home) = new_command_with_temp_home();
    command
        .args(["list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("List registered worktrees"));
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
fn runtime_commands_are_gated_without_config() {
    for subcommand in ["new", "list", "attach", "delete"] {
        let (mut command, _temp_home) = new_command_with_temp_home();
        command
            .arg(subcommand)
            .assert()
            .failure()
            .stderr(predicate::str::contains("missing config at"))
            .stderr(predicate::str::contains(".config/seshmux/config.toml"))
            .stderr(predicate::str::contains("README.md"));
    }
}

#[test]
fn runtime_command_reaches_stub_with_valid_config() {
    let (mut command, temp_home) = new_command_with_temp_home();
    write_valid_config(temp_home.path());

    command
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("list is not implemented in this milestone"))
        .stderr(predicate::str::contains("missing config at").not());
}
