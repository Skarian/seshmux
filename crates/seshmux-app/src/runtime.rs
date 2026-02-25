use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::App;

pub(crate) fn resolve_repo_root(app: &App<'_>, cwd: &Path) -> Result<PathBuf> {
    seshmux_core::git::repo_root(cwd, app.runner).with_context(|| {
        format!(
            "failed to resolve git repository root from {}",
            cwd.display()
        )
    })
}

pub(crate) fn repo_component(repo_root: &Path) -> &str {
    repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repo")
}

pub(crate) fn session_name_for(repo_root: &Path, worktree_name: &str) -> String {
    seshmux_core::tmux::session_name(repo_component(repo_root), worktree_name)
}

pub(crate) fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{repo_component, session_name_for};

    #[test]
    fn repo_component_defaults_when_missing() {
        assert_eq!(repo_component(Path::new("/")), "repo");
        assert_eq!(repo_component(Path::new("/tmp/repo-name")), "repo-name");
    }

    #[test]
    fn session_name_for_uses_repo_and_worktree() {
        let repo_root = Path::new("/tmp/My Repo");
        assert_eq!(session_name_for(repo_root, "w1"), "my-repo/w1");
    }
}
