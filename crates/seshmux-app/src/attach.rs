use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use thiserror::Error;

use crate::App;
use crate::runtime;
use crate::target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    pub cwd: PathBuf,
    pub worktree_name: String,
    pub create_if_missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachResult {
    pub worktree_name: String,
    pub worktree_path: PathBuf,
    pub session_name: String,
    pub created_session: bool,
}

#[derive(Debug, Error)]
pub enum AttachError {
    #[error("worktree '{name}' was not found in worktree.toml")]
    UnknownWorktree { name: String },
    #[error("no tmux session found for '{worktree_name}' ({session_name})")]
    MissingSession {
        worktree_name: String,
        session_name: String,
    },
}

impl<'a> App<'a> {
    pub fn attach(&self, request: AttachRequest) -> Result<AttachResult> {
        let target = target::resolve_target(self, &request.cwd, &request.worktree_name)?
            .ok_or_else(|| AttachError::UnknownWorktree {
                name: request.worktree_name.clone(),
            })?;

        let worktree_path = target.worktree_path.clone();
        if !worktree_path.exists() {
            bail!(
                "worktree path does not exist on disk: {}",
                worktree_path.display()
            );
        }

        let session_name = target.session_name.clone();

        let session_exists = seshmux_core::tmux::session_exists(&session_name, self.runner)
            .with_context(|| format!("failed to query tmux session '{session_name}'"))?;

        if session_exists {
            seshmux_core::tmux::connect_session(&session_name, runtime::inside_tmux(), self.runner)
                .with_context(|| format!("failed to connect to tmux session '{session_name}'"))?;

            return Ok(AttachResult {
                worktree_name: target.worktree_name,
                worktree_path,
                session_name,
                created_session: false,
            });
        }

        if !request.create_if_missing {
            return Err(AttachError::MissingSession {
                worktree_name: target.worktree_name,
                session_name,
            }
            .into());
        }

        let config = self.ensure_config_ready()?;

        seshmux_core::tmux::create_session_and_windows(
            &session_name,
            &worktree_path,
            &config.tmux.windows,
            self.runner,
        )
        .with_context(|| format!("failed to create tmux session '{session_name}'"))?;

        seshmux_core::tmux::connect_session(&session_name, runtime::inside_tmux(), self.runner)
            .with_context(|| format!("failed to connect to tmux session '{session_name}'"))?;

        Ok(AttachResult {
            worktree_name: target.worktree_name,
            worktree_path,
            session_name,
            created_session: true,
        })
    }
}
