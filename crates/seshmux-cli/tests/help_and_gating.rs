use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

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

fn init_git_repo(path: &Path) {
    fs::create_dir_all(path).expect("create repo dir");
    let output = StdCommand::new("git")
        .arg("init")
        .current_dir(path)
        .output()
        .expect("git init");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git_repo_with_commit(path: &Path) {
    init_git_repo(path);
    fs::write(path.join("README.md"), "hello\n").expect("write readme");

    let add_output = StdCommand::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output()
        .expect("git add");
    assert!(
        add_output.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    let commit_output = StdCommand::new("git")
        .args([
            "-c",
            "user.name=seshmux-test",
            "-c",
            "user.email=seshmux-test@example.com",
            "commit",
            "-m",
            "initial",
        ])
        .current_dir(path)
        .output()
        .expect("git commit");
    assert!(
        commit_output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit_output.stderr)
    );
}

fn assert_timestamp_log_names(entries: &[std::fs::DirEntry]) {
    assert!(!entries.is_empty(), "expected at least one diagnostics log");

    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .expect("diagnostics filename utf8");
        assert!(
            name.ends_with(".log"),
            "diagnostics file should end with .log: {name}"
        );
        let stem = name
            .strip_suffix(".log")
            .expect("diagnostics filename .log suffix");
        assert!(
            !stem.is_empty() && stem.chars().all(|character| character.is_ascii_digit()),
            "diagnostics filename must be <timestamp>.log, got: {name}"
        );
    }
}

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
fn root_command_runs_when_config_exists() {
    let (mut command, temp_home) = new_command_with_temp_home();
    write_valid_config(temp_home.path());
    let repo_dir = temp_home.path().join("repo");
    init_git_repo_with_commit(&repo_dir);

    command
        .current_dir(&repo_dir)
        .env("SESHMUX_TUI_TEST_EXIT", "completed")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

#[test]
fn root_command_prints_cancel_message() {
    let (mut command, temp_home) = new_command_with_temp_home();
    write_valid_config(temp_home.path());
    let repo_dir = temp_home.path().join("repo");
    init_git_repo_with_commit(&repo_dir);

    command
        .current_dir(&repo_dir)
        .env("SESHMUX_TUI_TEST_EXIT", "canceled")
        .assert()
        .success()
        .stdout(predicate::str::contains("Canceled."));
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
fn root_command_with_diagnostics_creates_log_file() {
    let (mut command, temp_home) = new_command_with_temp_home();
    write_valid_config(temp_home.path());
    let repo_dir = temp_home.path().join("repo");
    init_git_repo_with_commit(&repo_dir);

    command
        .arg("--diagnostics")
        .current_dir(&repo_dir)
        .env("SESHMUX_TUI_TEST_EXIT", "completed")
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
