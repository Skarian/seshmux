use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

pub fn new_command_with_temp_home() -> (Command, tempfile::TempDir) {
    let temp_home = tempfile::tempdir().expect("temp home");
    let binary = assert_cmd::cargo::cargo_bin!("seshmux");
    let mut command = Command::new(binary);
    command.env("HOME", temp_home.path());
    command.env("XDG_CONFIG_HOME", temp_home.path().join(".config"));
    (command, temp_home)
}

pub fn write_valid_config(home: &Path) {
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

pub fn init_git_repo(path: &Path) {
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

pub fn init_git_repo_with_commit(path: &Path) {
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

pub fn assert_timestamp_log_names(entries: &[std::fs::DirEntry]) {
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
