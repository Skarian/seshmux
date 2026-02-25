mod new;

pub use new::{NewPrepare, NewRequest, NewResult, NewStartPoint};

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
}
