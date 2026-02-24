use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SeshmuxConfig {
    pub version: u32,
    pub tmux: TmuxConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TmuxConfig {
    pub windows: Vec<WindowSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowSpec {
    pub name: String,
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    pub shell: Option<Vec<String>>,
    pub command: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not resolve home directory for config path")]
    HomeDirectoryUnavailable,
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid config: {message}")]
    Validation { message: String },
}

pub fn resolve_config_path() -> anyhow::Result<PathBuf> {
    let base_dirs = BaseDirs::new().ok_or(ConfigError::HomeDirectoryUnavailable)?;
    Ok(base_dirs
        .home_dir()
        .join(".config")
        .join("seshmux")
        .join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<SeshmuxConfig, ConfigError> {
    let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    let parsed: SeshmuxConfig = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;

    validate_config(&parsed)?;
    Ok(parsed)
}

pub fn validate_config(config: &SeshmuxConfig) -> Result<(), ConfigError> {
    if config.version != 1 {
        return Err(ConfigError::Validation {
            message: "version must be 1".to_string(),
        });
    }

    if config.tmux.windows.is_empty() {
        return Err(ConfigError::Validation {
            message: "at least one tmux window must be configured".to_string(),
        });
    }

    for (index, window) in config.tmux.windows.iter().enumerate() {
        if window.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                message: format!("window[{index}] name must be non-empty"),
            });
        }

        let direct_mode_selected = window.program.is_some() || window.args.is_some();
        let shell_mode_selected = window.shell.is_some() || window.command.is_some();

        if direct_mode_selected && shell_mode_selected {
            return Err(ConfigError::Validation {
                message: format!(
                    "window[{index}] must use exactly one launch mode (direct or shell), not both"
                ),
            });
        }

        if !direct_mode_selected && !shell_mode_selected {
            return Err(ConfigError::Validation {
                message: format!(
                    "window[{index}] must define either direct mode (program/args) or shell mode (shell/command)"
                ),
            });
        }

        if direct_mode_selected {
            if window
                .program
                .as_ref()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
            {
                return Err(ConfigError::Validation {
                    message: format!("window[{index}] direct mode requires non-empty program"),
                });
            }

            if window.shell.is_some() || window.command.is_some() {
                return Err(ConfigError::Validation {
                    message: format!(
                        "window[{index}] direct mode cannot include shell or command fields"
                    ),
                });
            }
        }

        if shell_mode_selected {
            let shell = window
                .shell
                .as_ref()
                .ok_or_else(|| ConfigError::Validation {
                    message: format!("window[{index}] shell mode requires shell field"),
                })?;

            if shell.is_empty() || shell[0].trim().is_empty() {
                return Err(ConfigError::Validation {
                    message: format!("window[{index}] shell mode requires shell[0] executable"),
                });
            }

            if window
                .command
                .as_ref()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
            {
                return Err(ConfigError::Validation {
                    message: format!("window[{index}] shell mode requires non-empty command"),
                });
            }

            if window.program.is_some() || window.args.is_some() {
                return Err(ConfigError::Validation {
                    message: format!(
                        "window[{index}] shell mode cannot include program or args fields"
                    ),
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_config_from_toml(raw: &str) -> Result<SeshmuxConfig, ConfigError> {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        fs::write(file.path(), raw).expect("write temp config");
        load_config(file.path())
    }

    #[test]
    fn accepts_valid_direct_mode_window() {
        let raw = r#"
version = 1

[[tmux.windows]]
name = "editor"
program = "nvim"
args = ["."]
"#;

        let config = load_config_from_toml(raw).expect("valid config");
        assert_eq!(config.tmux.windows.len(), 1);
    }

    #[test]
    fn accepts_valid_shell_mode_window() {
        let raw = r#"
version = 1

[[tmux.windows]]
name = "ops"
shell = ["/bin/zsh", "-lc"]
command = "echo ready"
"#;

        let config = load_config_from_toml(raw).expect("valid config");
        assert_eq!(config.tmux.windows.len(), 1);
    }

    #[test]
    fn rejects_window_with_both_launch_modes() {
        let raw = r#"
version = 1

[[tmux.windows]]
name = "bad"
program = "nvim"
shell = ["/bin/zsh", "-lc"]
command = "echo"
"#;

        let error = load_config_from_toml(raw).expect_err("config should fail");
        assert!(error.to_string().contains("exactly one launch mode"));
    }

    #[test]
    fn rejects_shell_mode_without_command() {
        let raw = r#"
version = 1

[[tmux.windows]]
name = "bad"
shell = ["/bin/zsh", "-lc"]
"#;

        let error = load_config_from_toml(raw).expect_err("config should fail");
        assert!(error.to_string().contains("requires non-empty command"));
    }

    #[test]
    fn rejects_direct_mode_without_program() {
        let raw = r#"
version = 1

[[tmux.windows]]
name = "bad"
args = ["--verbose"]
"#;

        let error = load_config_from_toml(raw).expect_err("config should fail");
        assert!(error.to_string().contains("requires non-empty program"));
    }

    #[test]
    fn rejects_config_with_no_windows() {
        let raw = r#"
version = 1

[tmux]
windows = []
"#;

        let error = load_config_from_toml(raw).expect_err("config should fail");
        assert!(error.to_string().contains("at least one tmux window"));
    }
}
