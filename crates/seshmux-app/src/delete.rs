use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use thiserror::Error;

use crate::App;
use crate::target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRequest {
    pub cwd: PathBuf,
    pub worktree_name: String,
    pub kill_tmux_session: bool,
    pub delete_branch: bool,
    pub force_worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteResult {
    pub worktree_name: String,
    pub repo_root: PathBuf,
    pub worktree_path: PathBuf,
    pub session_name: String,
    pub branch_name: String,
    pub branch_deleted: bool,
    pub branch_delete_error: Option<String>,
}

#[derive(Debug, Error)]
pub enum DeleteError {
    #[error("worktree '{name}' was not found in worktree.toml")]
    UnknownWorktree { name: String },
    #[error("worktree deletion failed: {message}")]
    WorktreeDeleteFailed { message: String },
}

impl<'a> App<'a> {
    pub fn delete(&self, request: DeleteRequest) -> Result<DeleteResult> {
        let worktree_name = request.worktree_name.clone();
        let target =
            target::resolve_target(self, &request.cwd, &worktree_name)?.ok_or_else(|| {
                DeleteError::UnknownWorktree {
                    name: worktree_name.clone(),
                }
            })?;

        let repo_root = target.repo_root.clone();
        let worktree_path = target.worktree_path.clone();
        let session_name = target.session_name.clone();

        if request.kill_tmux_session {
            let exists = seshmux_core::tmux::session_exists(&session_name, self.runner)
                .with_context(|| format!("failed to query tmux session '{session_name}'"))?;
            if exists {
                seshmux_core::tmux::kill_session(&session_name, self.runner)
                    .with_context(|| format!("failed to kill tmux session '{session_name}'"))?;
            }
        }

        let remove_result = if request.force_worktree {
            seshmux_core::git::force_remove_worktree(&repo_root, &worktree_path, self.runner)
        } else {
            seshmux_core::git::remove_worktree(&repo_root, &worktree_path, self.runner)
        };
        if let Err(error) = remove_result {
            return Err(DeleteError::WorktreeDeleteFailed {
                message: format!("{error:#}"),
            }
            .into());
        }

        let removed = seshmux_core::registry::remove_entry_by_name(&repo_root, &worktree_name)
            .with_context(|| {
                format!(
                    "failed to update worktree registry at {}",
                    seshmux_core::registry::registry_path(&repo_root).display()
                )
            })?;

        if removed.is_none() {
            return Err(DeleteError::UnknownWorktree {
                name: worktree_name.clone(),
            }
            .into());
        }

        let still_present = seshmux_core::registry::find_entry_by_name(&repo_root, &worktree_name)
            .with_context(|| {
                format!(
                    "failed to re-check worktree registry at {}",
                    seshmux_core::registry::registry_path(&repo_root).display()
                )
            })?
            .is_some();
        if still_present {
            bail!(
                "worktree '{worktree_name}' still exists in registry after delete; aborting to avoid drift"
            );
        }

        let branch_name = target.worktree_name;
        let mut branch_deleted = false;
        let mut branch_delete_error = None;

        if request.delete_branch {
            match seshmux_core::git::delete_branch(&repo_root, &branch_name, self.runner) {
                Ok(()) => {
                    branch_deleted = true;
                }
                Err(error) => {
                    branch_deleted = false;
                    branch_delete_error = Some(format!("{error:#}"));
                }
            }
        }

        Ok(DeleteResult {
            worktree_name,
            repo_root,
            worktree_path,
            session_name,
            branch_name,
            branch_deleted,
            branch_delete_error,
        })
    }

    pub fn force_delete_branch(&self, repo_root: PathBuf, branch_name: String) -> Result<()> {
        seshmux_core::git::force_delete_branch(&repo_root, &branch_name, self.runner)
            .with_context(|| format!("failed to force delete branch '{branch_name}'"))?;
        Ok(())
    }
}
