mod attach;
mod delete;
mod list;
mod new;

pub use attach::{AttachError, AttachRequest, AttachResult};
pub use delete::{DeleteError, DeleteRequest, DeleteResult};
pub use list::{ListResult, WorktreeRow};
pub use new::{NewPrepare, NewRequest, NewResult, NewStartPoint};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use seshmux_core::command_runner::CommandRunner;
use seshmux_core::config::{SeshmuxConfig, load_config, resolve_config_path};
use seshmux_core::doctor::{DoctorReport, run_doctor_with_runner};

pub struct App<'a> {
    pub runner: &'a dyn CommandRunner,
}

impl<'a> App<'a> {
    pub fn new(runner: &'a dyn CommandRunner) -> Self {
        Self { runner }
    }

    pub fn doctor(&self) -> Result<DoctorReport> {
        Ok(run_doctor_with_runner(self.runner))
    }

    pub fn ensure_config_ready(&self) -> Result<SeshmuxConfig> {
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
        })
    }

    pub fn ensure_runtime_repo_ready(&self, cwd: &Path) -> Result<PathBuf> {
        let repo_root = seshmux_core::git::repo_root(cwd, self.runner).with_context(|| {
            format!(
                "failed to resolve git repository root from {}",
                cwd.display()
            )
        })?;

        let commits = seshmux_core::git::query_commits(&repo_root, "", 1, self.runner)
            .with_context(|| {
                format!(
                    "failed to inspect commit history in {}",
                    repo_root.display()
                )
            })?;

        if commits.is_empty() {
            bail!(
                "repository has no commits yet; create an initial commit before starting seshmux"
            );
        }

        Ok(repo_root)
    }
}
