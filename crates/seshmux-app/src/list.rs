use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::App;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListResult {
    pub repo_root: PathBuf,
    pub rows: Vec<WorktreeRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRow {
    pub name: String,
    pub path: PathBuf,
    pub created_at: String,
    pub branch: String,
    pub session_name: String,
    pub session_running: bool,
}

impl<'a> App<'a> {
    pub fn list(&self, cwd: &Path) -> Result<ListResult> {
        let repo_root = seshmux_core::git::repo_root(cwd, self.runner).with_context(|| {
            format!(
                "failed to resolve git repository root from {}",
                cwd.display()
            )
        })?;

        let mut rows = Vec::<WorktreeRow>::new();
        let entries = seshmux_core::registry::load_registry(&repo_root).with_context(|| {
            format!(
                "failed to load worktree registry at {}",
                seshmux_core::registry::registry_path(&repo_root).display()
            )
        })?;

        let repo_component = repo_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("repo");

        for entry in entries {
            let path = PathBuf::from(entry.path.clone());

            let branch = if path.exists() {
                seshmux_core::git::current_branch(&path, self.runner).with_context(|| {
                    format!(
                        "failed to resolve current branch for worktree '{}' at {}",
                        entry.name,
                        path.display()
                    )
                })?
            } else {
                "MISSING".to_string()
            };

            let session_name = seshmux_core::tmux::session_name(repo_component, &entry.name);
            let session_running = seshmux_core::tmux::session_exists(&session_name, self.runner)
                .with_context(|| format!("failed to check tmux session '{session_name}'"))?;

            rows.push(WorktreeRow {
                name: entry.name,
                path,
                created_at: entry.created_at,
                branch,
                session_name,
                session_running,
            });
        }

        rows.sort_by(|left, right| right.created_at.cmp(&left.created_at));

        Ok(ListResult { repo_root, rows })
    }
}
