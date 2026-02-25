use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::App;
use crate::runtime;

pub(crate) struct WorktreeCatalog {
    repo_root: PathBuf,
    entries: Vec<seshmux_core::registry::RegistryEntry>,
}

impl WorktreeCatalog {
    pub(crate) fn load(app: &App<'_>, cwd: &Path) -> Result<Self> {
        let repo_root = runtime::resolve_repo_root(app, cwd)?;
        let entries = seshmux_core::registry::load_registry(&repo_root).with_context(|| {
            format!(
                "failed to load worktree registry at {}",
                seshmux_core::registry::registry_path(&repo_root).display()
            )
        })?;

        Ok(Self { repo_root, entries })
    }

    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub(crate) fn find(&self, name: &str) -> Option<&seshmux_core::registry::RegistryEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    pub(crate) fn list_rows(&self, app: &App<'_>) -> Result<Vec<crate::list::WorktreeRow>> {
        let mut rows = Vec::new();

        for entry in &self.entries {
            let path = PathBuf::from(entry.path.clone());
            let branch = if path.exists() {
                seshmux_core::git::current_branch(&path, app.runner).with_context(|| {
                    format!(
                        "failed to resolve current branch for worktree '{}' at {}",
                        entry.name,
                        path.display()
                    )
                })?
            } else {
                "MISSING".to_string()
            };

            let session_name = runtime::session_name_for(&self.repo_root, &entry.name);
            let session_running = seshmux_core::tmux::session_exists(&session_name, app.runner)
                .with_context(|| format!("failed to check tmux session '{session_name}'"))?;

            rows.push(crate::list::WorktreeRow {
                name: entry.name.clone(),
                path,
                created_at: entry.created_at.clone(),
                branch,
                session_name,
                session_running,
            });
        }

        rows.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::Mutex;

    use anyhow::anyhow;

    use crate::App;
    use crate::runtime;
    use seshmux_core::command_runner::{CommandOutput, CommandRunner};
    use seshmux_core::registry::{RegistryEntry, insert_unique_entry};

    use super::WorktreeCatalog;

    #[derive(Default)]
    struct QueueRunner {
        outputs: Mutex<VecDeque<anyhow::Result<CommandOutput>>>,
    }

    impl QueueRunner {
        fn new(outputs: Vec<anyhow::Result<CommandOutput>>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
            }
        }
    }

    impl CommandRunner for QueueRunner {
        fn run(
            &self,
            _program: &str,
            _args: &[&str],
            _cwd: Option<&Path>,
        ) -> anyhow::Result<CommandOutput> {
            self.outputs
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or_else(|| Err(anyhow!("missing output")))
        }

        fn run_interactive(
            &self,
            _program: &str,
            _args: &[&str],
            _cwd: Option<&Path>,
        ) -> anyhow::Result<i32> {
            Err(anyhow!("interactive command not expected in this test"))
        }
    }

    fn output(stdout: &str, stderr: &str, status_code: i32) -> anyhow::Result<CommandOutput> {
        Ok(CommandOutput {
            status_code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        })
    }

    #[test]
    fn list_rows_sorts_by_recency() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");

        let old_path = repo_root.join("worktrees").join("old");
        let new_path = repo_root.join("worktrees").join("new");
        std::fs::create_dir_all(&old_path).expect("old path");
        std::fs::create_dir_all(&new_path).expect("new path");

        insert_unique_entry(
            &repo_root,
            RegistryEntry {
                name: "old".to_string(),
                path: old_path.to_string_lossy().to_string(),
                created_at: "2026-02-24T10:00:00Z".to_string(),
            },
        )
        .expect("insert old");
        insert_unique_entry(
            &repo_root,
            RegistryEntry {
                name: "new".to_string(),
                path: new_path.to_string_lossy().to_string(),
                created_at: "2026-02-25T10:00:00Z".to_string(),
            },
        )
        .expect("insert new");

        let runner = QueueRunner::new(vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("old\n", "", 0),
            output("", "missing", 1),
            output("new\n", "", 0),
            output("", "", 0),
        ]);
        let app = App::new(&runner);

        let catalog = WorktreeCatalog::load(&app, &repo_root).expect("catalog");
        let rows = catalog.list_rows(&app).expect("rows");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "new");
        assert_eq!(rows[1].name, "old");
        assert_eq!(
            rows[0].session_name,
            runtime::session_name_for(&repo_root, "new")
        );
        assert!(rows[0].session_running);
        assert!(!rows[1].session_running);
    }
}
