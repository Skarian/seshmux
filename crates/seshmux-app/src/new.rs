use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::App;
use crate::runtime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPrepare {
    pub repo_root: PathBuf,
    pub worktrees_dir: PathBuf,
    pub gitignore_has_worktrees_entry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewStartPoint {
    CurrentBranch,
    Branch(String),
    Commit(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRequest {
    pub cwd: PathBuf,
    pub worktree_name: String,
    pub start_point: NewStartPoint,
    pub add_worktrees_gitignore_entry: bool,
    pub selected_extras: Vec<PathBuf>,
    pub connect_now: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewResult {
    pub repo_root: PathBuf,
    pub worktrees_dir: PathBuf,
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub session_name: String,
    pub attach_command: String,
    pub connected_now: bool,
}

impl<'a> App<'a> {
    pub fn new_prepare(&self, cwd: &Path) -> Result<NewPrepare> {
        let repo_root = runtime::resolve_repo_root(self, cwd)?;
        let worktrees_dir = repo_root.join("worktrees");
        let gitignore_has_worktrees_entry =
            seshmux_core::git::gitignore_contains_worktrees(&repo_root).with_context(|| {
                format!("failed to inspect .gitignore in {}", repo_root.display())
            })?;

        Ok(NewPrepare {
            repo_root,
            worktrees_dir,
            gitignore_has_worktrees_entry,
        })
    }

    pub fn new_query_branches(
        &self,
        repo_root: &Path,
        query: &str,
    ) -> Result<Vec<seshmux_core::git::BranchRef>> {
        seshmux_core::git::query_branches(repo_root, query, self.runner)
            .with_context(|| format!("failed to query branches in {}", repo_root.display()))
    }

    pub fn new_query_commits(
        &self,
        repo_root: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Vec<seshmux_core::git::CommitRef>> {
        seshmux_core::git::query_commits(repo_root, query, limit, self.runner)
            .with_context(|| format!("failed to query commits in {}", repo_root.display()))
    }

    pub fn new_list_extras(&self, repo_root: &Path) -> Result<Vec<PathBuf>> {
        seshmux_core::extras::list_extra_candidates(repo_root, self.runner).with_context(|| {
            format!(
                "failed to list extra copy candidates in {}",
                repo_root.display()
            )
        })
    }

    pub fn new_load_always_skip_buckets_for_indexing(
        &self,
        repo_root: &Path,
    ) -> Result<seshmux_core::registry::AlwaysSkipBucketsLoad> {
        seshmux_core::registry::load_always_skip_buckets_for_indexing(repo_root).with_context(
            || {
                format!(
                    "failed to load extras skip settings in {}",
                    repo_root.display()
                )
            },
        )
    }

    pub fn new_save_always_skip_buckets(
        &self,
        repo_root: &Path,
        buckets: &BTreeSet<String>,
    ) -> Result<()> {
        seshmux_core::registry::save_always_skip_buckets(repo_root, buckets).with_context(|| {
            format!(
                "failed to persist extras skip settings in {}",
                repo_root.display()
            )
        })
    }

    pub fn new_execute(&self, request: NewRequest) -> Result<NewResult> {
        let config = self.ensure_config_ready()?;

        seshmux_core::names::validate_worktree_name(&request.worktree_name)
            .with_context(|| format!("invalid worktree name '{}'", request.worktree_name))?;

        let repo_root = runtime::resolve_repo_root(self, &request.cwd)?;

        let worktrees_dir = repo_root.join("worktrees");
        std::fs::create_dir_all(&worktrees_dir)
            .with_context(|| format!("failed to create {}", worktrees_dir.display()))?;

        let worktree_path = worktrees_dir.join(&request.worktree_name);

        seshmux_core::registry::ensure_entry_available(
            &repo_root,
            &request.worktree_name,
            &worktree_path,
        )
        .with_context(|| "registry already has a conflicting worktree entry".to_string())?;

        if request.add_worktrees_gitignore_entry {
            seshmux_core::git::ensure_worktrees_gitignore_entry(&repo_root).with_context(|| {
                format!("failed to update .gitignore in {}", repo_root.display())
            })?;
        }

        let start_point = match &request.start_point {
            NewStartPoint::CurrentBranch => {
                seshmux_core::git::resolve_current_start_point(&repo_root, self.runner)?
            }
            NewStartPoint::Branch(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    bail!("start branch cannot be empty");
                }
                trimmed.to_string()
            }
            NewStartPoint::Commit(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    bail!("start commit cannot be empty");
                }
                trimmed.to_string()
            }
        };

        seshmux_core::git::create_worktree(
            &repo_root,
            &request.worktree_name,
            &worktree_path,
            &start_point,
            self.runner,
        )
        .with_context(|| {
            format!(
                "failed to create worktree '{}' at {}",
                request.worktree_name,
                worktree_path.display()
            )
        })?;

        seshmux_core::extras::copy_selected_extras(
            &repo_root,
            &worktree_path,
            &request.selected_extras,
        )
        .with_context(|| {
            format!(
                "failed to copy selected extras into {}",
                worktree_path.display()
            )
        })?;

        let created_at = seshmux_core::time::now_utc_rfc3339()
            .map_err(|error| anyhow!("failed to format timestamp: {error}"))?;

        seshmux_core::registry::insert_unique_entry(
            &repo_root,
            seshmux_core::registry::RegistryEntry {
                name: request.worktree_name.clone(),
                path: worktree_path.to_string_lossy().to_string(),
                created_at,
            },
        )
        .with_context(|| {
            format!(
                "failed to register worktree '{}' in {}",
                request.worktree_name,
                worktrees_dir.join("worktree.toml").display()
            )
        })?;

        let session_name = runtime::session_name_for(&repo_root, &request.worktree_name);
        let attach_command = format!("tmux attach-session -t {session_name}");

        seshmux_core::tmux::create_session_and_windows(
            &session_name,
            &worktree_path,
            &config.tmux.windows,
            self.runner,
        )
        .with_context(|| {
            format!(
                "failed to create tmux session '{session_name}'; attach manually with '{attach_command}' after resolving tmux errors"
            )
        })?;

        let mut connected_now = false;
        if request.connect_now {
            seshmux_core::tmux::connect_session(
                &session_name,
                runtime::inside_tmux(),
                self.runner,
            )
            .with_context(|| {
                format!(
                    "failed to connect to tmux session '{session_name}'; attach manually with '{attach_command}'"
                )
            })?;
            connected_now = true;
        }

        Ok(NewResult {
            repo_root,
            worktrees_dir,
            worktree_path,
            branch_name: request.worktree_name,
            session_name,
            attach_command,
            connected_now,
        })
    }
}
