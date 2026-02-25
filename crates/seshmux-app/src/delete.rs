use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use thiserror::Error;

use crate::App;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRequest {
    pub cwd: PathBuf,
    pub worktree_name: String,
    pub kill_tmux_session: bool,
    pub delete_branch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteResult {
    pub worktree_name: String,
    pub worktree_path: PathBuf,
    pub session_name: String,
    pub branch_name: String,
    pub branch_deleted: bool,
}

#[derive(Debug, Error)]
pub enum DeleteError {
    #[error("worktree '{name}' was not found in worktree.toml")]
    UnknownWorktree { name: String },
    #[error("branch '{branch}' is not fully merged; confirm force delete to continue")]
    BranchRequiresForce { branch: String },
}

impl<'a> App<'a> {
    pub fn delete(&self, request: DeleteRequest) -> Result<DeleteResult> {
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
            .ok_or_else(|| DeleteError::UnknownWorktree {
                name: request.worktree_name.clone(),
            })?;

        let worktree_path = PathBuf::from(&entry.path);
        let repo_component = repo_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("repo");
        let session_name = seshmux_core::tmux::session_name(repo_component, &entry.name);

        if request.kill_tmux_session {
            let exists = seshmux_core::tmux::session_exists(&session_name, self.runner)
                .with_context(|| format!("failed to query tmux session '{session_name}'"))?;
            if exists {
                seshmux_core::tmux::kill_session(&session_name, self.runner)
                    .with_context(|| format!("failed to kill tmux session '{session_name}'"))?;
            }
        }

        seshmux_core::git::remove_worktree(&repo_root, &worktree_path, self.runner).with_context(
            || {
                format!(
                    "failed to remove worktree '{}' at {}",
                    request.worktree_name,
                    worktree_path.display()
                )
            },
        )?;

        let removed =
            seshmux_core::registry::remove_entry_by_name(&repo_root, &request.worktree_name)
                .with_context(|| {
                    format!(
                        "failed to update worktree registry at {}",
                        seshmux_core::registry::registry_path(&repo_root).display()
                    )
                })?;

        if removed.is_none() {
            return Err(DeleteError::UnknownWorktree {
                name: request.worktree_name.clone(),
            }
            .into());
        }

        let still_present =
            seshmux_core::registry::find_entry_by_name(&repo_root, &request.worktree_name)
                .with_context(|| {
                    format!(
                        "failed to re-check worktree registry at {}",
                        seshmux_core::registry::registry_path(&repo_root).display()
                    )
                })?
                .is_some();
        if still_present {
            bail!(
                "worktree '{}' still exists in registry after delete; aborting to avoid drift",
                request.worktree_name
            );
        }

        let branch_name = entry.name;
        let mut branch_deleted = false;

        if request.delete_branch {
            match seshmux_core::git::delete_branch(
                &repo_root,
                &branch_name,
                seshmux_core::git::BranchDeleteMode::Safe,
                self.runner,
            ) {
                Ok(()) => {
                    branch_deleted = true;
                }
                Err(seshmux_core::git::GitError::BranchNotFullyMerged { .. }) => {
                    return Err(DeleteError::BranchRequiresForce {
                        branch: branch_name,
                    }
                    .into());
                }
                Err(error) => return Err(error.into()),
            }
        }

        Ok(DeleteResult {
            worktree_name: request.worktree_name,
            worktree_path,
            session_name,
            branch_name,
            branch_deleted,
        })
    }

    pub fn force_delete_branch(&self, cwd: &Path, branch_name: &str) -> Result<()> {
        let repo_root = seshmux_core::git::repo_root(cwd, self.runner).with_context(|| {
            format!(
                "failed to resolve git repository root from {}",
                cwd.display()
            )
        })?;

        seshmux_core::git::delete_branch(
            &repo_root,
            branch_name,
            seshmux_core::git::BranchDeleteMode::Force,
            self.runner,
        )
        .with_context(|| format!("failed to force delete branch '{branch_name}'"))?;

        Ok(())
    }
}
