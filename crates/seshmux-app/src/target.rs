use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::App;
use crate::catalog::WorktreeCatalog;
use crate::runtime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTarget {
    pub(crate) repo_root: PathBuf,
    pub(crate) worktree_name: String,
    pub(crate) worktree_path: PathBuf,
    pub(crate) session_name: String,
}

pub(crate) fn resolve_target(
    app: &App<'_>,
    cwd: &Path,
    worktree_name: &str,
) -> Result<Option<ResolvedTarget>> {
    let catalog = WorktreeCatalog::load(app, cwd)?;
    let Some(entry) = catalog.find(worktree_name) else {
        return Ok(None);
    };

    let repo_root = catalog.repo_root().to_path_buf();
    let resolved_name = entry.name.clone();
    let worktree_path = PathBuf::from(&entry.path);
    let session_name = runtime::session_name_for(&repo_root, &resolved_name);

    Ok(Some(ResolvedTarget {
        repo_root,
        worktree_name: resolved_name,
        worktree_path,
        session_name,
    }))
}
