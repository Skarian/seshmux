use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use thiserror::Error;

use crate::App;

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
        let repo_root =
            seshmux_core::git::repo_root(&request.cwd, self.runner).with_context(|| {
                format!(
                    "failed to resolve git repository root from {}",
                    request.cwd.display()
                )
            })?;

        let entry = seshmux_core::registry::find_entry_by_name(&repo_root, &request.worktree_name)
            .with_context(|| {
                format!(
                    "failed to inspect worktree registry at {}",
                    seshmux_core::registry::registry_path(&repo_root).display()
                )
            })?
            .ok_or_else(|| AttachError::UnknownWorktree {
                name: request.worktree_name.clone(),
            })?;

        let worktree_path = PathBuf::from(&entry.path);
        if !worktree_path.exists() {
            bail!(
                "worktree path does not exist on disk: {}",
                worktree_path.display()
            );
        }

        let repo_component = repo_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("repo");
        let session_name = seshmux_core::tmux::session_name(repo_component, &entry.name);

        let session_exists = seshmux_core::tmux::session_exists(&session_name, self.runner)
            .with_context(|| format!("failed to query tmux session '{session_name}'"))?;

        if session_exists {
            seshmux_core::tmux::connect_session(&session_name, is_inside_tmux(), self.runner)
                .with_context(|| format!("failed to connect to tmux session '{session_name}'"))?;

            return Ok(AttachResult {
                worktree_name: entry.name,
                worktree_path,
                session_name,
                created_session: false,
            });
        }

        if !request.create_if_missing {
            return Err(AttachError::MissingSession {
                worktree_name: entry.name,
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

        seshmux_core::tmux::connect_session(&session_name, is_inside_tmux(), self.runner)
            .with_context(|| format!("failed to connect to tmux session '{session_name}'"))?;

        Ok(AttachResult {
            worktree_name: entry.name,
            worktree_path,
            session_name,
            created_session: true,
        })
    }
}

fn is_inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}
