use std::env;
use std::fmt;
use std::path::Path;

use crate::command_runner::{CommandRunner, SystemCommandRunner};
use crate::config::{WindowSpec, load_config, parse_window_launch, resolve_config_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Pass,
    Fail,
}

impl fmt::Display for CheckState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(f, "PASS"),
            Self::Fail => write!(f, "FAIL"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorCheck {
    pub name: String,
    pub state: CheckState,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.state == CheckState::Fail)
    }

    pub fn summary(&self) -> String {
        let passed = self
            .checks
            .iter()
            .filter(|check| check.state == CheckState::Pass)
            .count();
        let failed = self.checks.len().saturating_sub(passed);
        format!("{passed} passed, {failed} failed")
    }
}

pub fn run_doctor() -> DoctorReport {
    let runner = SystemCommandRunner::new();
    run_doctor_with_runner(&runner)
}

pub fn run_doctor_with_runner(runner: &dyn CommandRunner) -> DoctorReport {
    let mut checks = Vec::new();

    checks.push(match env::consts::OS {
        "macos" => pass_check("os is supported", "detected macOS"),
        "linux" => pass_check("os is supported", "detected Linux"),
        detected => fail_check(
            "os is supported",
            format!("detected {detected}, expected macOS or Linux"),
        ),
    });

    checks.push(if is_executable_in_path("git") {
        pass_check("git is installed", "git executable found in PATH")
    } else {
        fail_check("git is installed", "git executable not found in PATH")
    });

    checks.push(check_git_worktree_support(runner));
    checks.push(check_tmux_callable(runner));

    match resolve_config_path() {
        Ok(config_path) => {
            if config_path.exists() {
                checks.push(pass_check(
                    "config file exists",
                    format!("found at {}", config_path.display()),
                ));

                match load_config(&config_path) {
                    Ok(config) => {
                        checks.push(pass_check("config parses and validates", "config is valid"));
                        checks.push(check_window_targets(&config.tmux.windows));
                    }
                    Err(error) => {
                        checks.push(fail_check("config parses and validates", error.to_string()));
                        checks.push(skipped_check(
                            "window launch targets executable",
                            "config is invalid",
                        ));
                    }
                }
            } else {
                checks.push(fail_check(
                    "config file exists",
                    format!("expected at {}", config_path.display()),
                ));
                push_skipped_checks(
                    &mut checks,
                    &[
                        "config parses and validates",
                        "window launch targets executable",
                    ],
                    "config file is missing",
                );
            }
        }
        Err(error) => {
            checks.push(fail_check("config path resolves", error.to_string()));
            push_skipped_checks(
                &mut checks,
                &[
                    "config file exists",
                    "config parses and validates",
                    "window launch targets executable",
                ],
                "config path could not be resolved",
            );
        }
    }

    DoctorReport { checks }
}

fn check_git_worktree_support(runner: &dyn CommandRunner) -> DoctorCheck {
    match runner.run("git", &["worktree", "-h"], None) {
        Ok(output) => {
            let combined = format!("{}\n{}", output.stdout, output.stderr);
            if combined.contains("usage: git worktree") {
                pass_check(
                    "git worktree available",
                    "git worktree command is available",
                )
            } else {
                fail_check(
                    "git worktree available",
                    format!(
                        "git worktree help output did not match expected format (exit code {})",
                        output.status_code
                    ),
                )
            }
        }
        Err(error) => fail_check(
            "git worktree available",
            format!("failed to execute git worktree check: {error}"),
        ),
    }
}

fn check_tmux_callable(runner: &dyn CommandRunner) -> DoctorCheck {
    match runner.run("tmux", &["-V"], None) {
        Ok(output) if output.status_code == 0 => {
            pass_check("tmux is installed", output.stdout.trim().to_string())
        }
        Ok(output) => fail_check(
            "tmux is installed",
            format!(
                "tmux returned exit code {} with output: {}",
                output.status_code,
                output.stderr.trim()
            ),
        ),
        Err(error) => fail_check(
            "tmux is installed",
            format!("failed to execute tmux check: {error}"),
        ),
    }
}

fn check_window_targets(windows: &[WindowSpec]) -> DoctorCheck {
    let mut missing_targets = Vec::new();

    for window in windows {
        let launch = match parse_window_launch(window) {
            Ok(launch) => launch,
            Err(_) => {
                missing_targets.push(format!("window '{}' has invalid launch mode", window.name));
                continue;
            }
        };

        let executable = launch.executable();
        if !is_executable_in_path(executable) {
            missing_targets.push(format!(
                "window '{}' {} '{}'",
                window.name,
                launch.executable_label(),
                executable
            ));
        }
    }

    if missing_targets.is_empty() {
        pass_check(
            "window launch targets executable",
            "all configured launch targets were found in PATH",
        )
    } else {
        fail_check(
            "window launch targets executable",
            format!("missing executables: {}", missing_targets.join(", ")),
        )
    }
}

fn pass_check(name: &str, details: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        state: CheckState::Pass,
        details: details.into(),
    }
}

fn fail_check(name: &str, details: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        state: CheckState::Fail,
        details: details.into(),
    }
}

fn skipped_check(name: &str, reason: &str) -> DoctorCheck {
    fail_check(name, format!("skipped because {reason}"))
}

fn push_skipped_checks(checks: &mut Vec<DoctorCheck>, names: &[&str], reason: &str) {
    checks.extend(
        names
            .iter()
            .copied()
            .map(|name| skipped_check(name, reason)),
    );
}

fn is_executable_in_path(program: &str) -> bool {
    let program_path = Path::new(program);

    if program_path.is_absolute() || program.contains('/') {
        return is_executable_file(program_path);
    }

    let path_value = match env::var_os("PATH") {
        Some(value) => value,
        None => return false,
    };

    env::split_paths(&path_value)
        .map(|directory| directory.join(program))
        .any(|candidate| is_executable_file(&candidate))
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match path.metadata() {
            Ok(metadata) => metadata.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_state_display_is_uppercase_label() {
        assert_eq!(CheckState::Pass.to_string(), "PASS");
        assert_eq!(CheckState::Fail.to_string(), "FAIL");
    }

    #[test]
    fn doctor_summary_counts_pass_and_fail() {
        let report = DoctorReport {
            checks: vec![
                DoctorCheck {
                    name: "a".to_string(),
                    state: CheckState::Pass,
                    details: "ok".to_string(),
                },
                DoctorCheck {
                    name: "b".to_string(),
                    state: CheckState::Fail,
                    details: "no".to_string(),
                },
                DoctorCheck {
                    name: "c".to_string(),
                    state: CheckState::Pass,
                    details: "ok".to_string(),
                },
            ],
        };

        assert_eq!(report.summary(), "2 passed, 1 failed");
        assert!(report.has_failures());
    }
}
