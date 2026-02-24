use std::env;
use std::fmt;
use std::path::Path;

use crate::command_runner::{CommandRunner, SystemCommandRunner};
use crate::config::{WindowSpec, load_config, resolve_config_path};

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

    if env::consts::OS == "macos" {
        checks.push(DoctorCheck {
            name: "os is macOS".to_string(),
            state: CheckState::Pass,
            details: "detected macOS".to_string(),
        });
    } else {
        checks.push(DoctorCheck {
            name: "os is macOS".to_string(),
            state: CheckState::Fail,
            details: format!("detected {}, expected macOS", env::consts::OS),
        });
    }

    if is_executable_in_path("git") {
        checks.push(DoctorCheck {
            name: "git is installed".to_string(),
            state: CheckState::Pass,
            details: "git executable found in PATH".to_string(),
        });
    } else {
        checks.push(DoctorCheck {
            name: "git is installed".to_string(),
            state: CheckState::Fail,
            details: "git executable not found in PATH".to_string(),
        });
    }

    checks.push(check_git_worktree_support(runner));
    checks.push(check_tmux_callable(runner));

    match resolve_config_path() {
        Ok(config_path) => {
            if config_path.exists() {
                checks.push(DoctorCheck {
                    name: "config file exists".to_string(),
                    state: CheckState::Pass,
                    details: format!("found at {}", config_path.display()),
                });

                match load_config(&config_path) {
                    Ok(config) => {
                        checks.push(DoctorCheck {
                            name: "config parses and validates".to_string(),
                            state: CheckState::Pass,
                            details: "config is valid".to_string(),
                        });

                        checks.push(check_window_targets(&config.tmux.windows));
                    }
                    Err(error) => {
                        checks.push(DoctorCheck {
                            name: "config parses and validates".to_string(),
                            state: CheckState::Fail,
                            details: error.to_string(),
                        });

                        checks.push(DoctorCheck {
                            name: "window launch targets executable".to_string(),
                            state: CheckState::Fail,
                            details: "skipped because config is invalid".to_string(),
                        });
                    }
                }
            } else {
                checks.push(DoctorCheck {
                    name: "config file exists".to_string(),
                    state: CheckState::Fail,
                    details: format!("expected at {}", config_path.display()),
                });

                checks.push(DoctorCheck {
                    name: "config parses and validates".to_string(),
                    state: CheckState::Fail,
                    details: "skipped because config file is missing".to_string(),
                });

                checks.push(DoctorCheck {
                    name: "window launch targets executable".to_string(),
                    state: CheckState::Fail,
                    details: "skipped because config file is missing".to_string(),
                });
            }
        }
        Err(error) => {
            checks.push(DoctorCheck {
                name: "config path resolves".to_string(),
                state: CheckState::Fail,
                details: error.to_string(),
            });

            checks.push(DoctorCheck {
                name: "config file exists".to_string(),
                state: CheckState::Fail,
                details: "skipped because config path could not be resolved".to_string(),
            });

            checks.push(DoctorCheck {
                name: "config parses and validates".to_string(),
                state: CheckState::Fail,
                details: "skipped because config path could not be resolved".to_string(),
            });

            checks.push(DoctorCheck {
                name: "window launch targets executable".to_string(),
                state: CheckState::Fail,
                details: "skipped because config path could not be resolved".to_string(),
            });
        }
    }

    DoctorReport { checks }
}

fn check_git_worktree_support(runner: &dyn CommandRunner) -> DoctorCheck {
    match runner.run("git", &["worktree", "-h"], None) {
        Ok(output) => {
            let combined = format!("{}\n{}", output.stdout, output.stderr);
            if combined.contains("usage: git worktree") {
                DoctorCheck {
                    name: "git worktree available".to_string(),
                    state: CheckState::Pass,
                    details: "git worktree command is available".to_string(),
                }
            } else {
                DoctorCheck {
                    name: "git worktree available".to_string(),
                    state: CheckState::Fail,
                    details: format!(
                        "git worktree help output did not match expected format (exit code {})",
                        output.status_code
                    ),
                }
            }
        }
        Err(error) => DoctorCheck {
            name: "git worktree available".to_string(),
            state: CheckState::Fail,
            details: format!("failed to execute git worktree check: {error}"),
        },
    }
}

fn check_tmux_callable(runner: &dyn CommandRunner) -> DoctorCheck {
    match runner.run("tmux", &["-V"], None) {
        Ok(output) if output.status_code == 0 => DoctorCheck {
            name: "tmux is installed".to_string(),
            state: CheckState::Pass,
            details: output.stdout.trim().to_string(),
        },
        Ok(output) => DoctorCheck {
            name: "tmux is installed".to_string(),
            state: CheckState::Fail,
            details: format!(
                "tmux returned exit code {} with output: {}",
                output.status_code,
                output.stderr.trim()
            ),
        },
        Err(error) => DoctorCheck {
            name: "tmux is installed".to_string(),
            state: CheckState::Fail,
            details: format!("failed to execute tmux check: {error}"),
        },
    }
}

fn check_window_targets(windows: &[WindowSpec]) -> DoctorCheck {
    let mut missing_targets = Vec::new();

    for window in windows {
        if let Some(program) = &window.program {
            if !is_executable_in_path(program) {
                missing_targets.push(format!("window '{}' program '{}'", window.name, program));
            }
        }

        if let Some(shell) = &window.shell {
            if let Some(executable) = shell.first() {
                if !is_executable_in_path(executable) {
                    missing_targets
                        .push(format!("window '{}' shell '{}'", window.name, executable));
                }
            }
        }
    }

    if missing_targets.is_empty() {
        DoctorCheck {
            name: "window launch targets executable".to_string(),
            state: CheckState::Pass,
            details: "all configured launch targets were found in PATH".to_string(),
        }
    } else {
        DoctorCheck {
            name: "window launch targets executable".to_string(),
            state: CheckState::Fail,
            details: format!("missing executables: {}", missing_targets.join(", ")),
        }
    }
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
