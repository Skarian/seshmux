use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::command_runner::CommandRunner;

#[derive(Debug, Error)]
pub enum ExtrasError {
    #[error("failed to execute git command: {0}")]
    Execute(String),
    #[error("git command failed: git {command} (exit {status}) {stderr}")]
    CommandFailed {
        command: String,
        status: i32,
        stderr: String,
    },
    #[error("extra path must be relative and stay inside repository: {0}")]
    InvalidPath(String),
    #[error("failed to copy extra from {from} to {to}: {error}")]
    Copy {
        from: String,
        to: String,
        error: std::io::Error,
    },
}

pub fn list_extra_candidates(
    repo_root: &Path,
    runner: &dyn CommandRunner,
) -> Result<Vec<PathBuf>, ExtrasError> {
    let untracked = run_git_lines(
        runner,
        repo_root,
        &["ls-files", "--others", "--exclude-standard", "--directory"],
    )?;

    let ignored = run_git_lines(
        runner,
        repo_root,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ],
    )?;

    let mut set = BTreeSet::new();

    for entry in untracked.into_iter().chain(ignored.into_iter()) {
        validate_relative_path(&entry)?;
        set.insert(entry);
    }

    Ok(set.into_iter().collect())
}

pub fn copy_selected_extras(
    repo_root: &Path,
    target_root: &Path,
    selected: &[PathBuf],
) -> Result<(), ExtrasError> {
    for relative in selected {
        validate_relative_path(relative)?;

        let source = repo_root.join(relative);
        let target = target_root.join(relative);

        if source.is_dir() {
            copy_directory_recursive(&source, &target)?;
            continue;
        }

        if source.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| ExtrasError::Copy {
                    from: source.display().to_string(),
                    to: target.display().to_string(),
                    error,
                })?;
            }

            fs::copy(&source, &target).map_err(|error| ExtrasError::Copy {
                from: source.display().to_string(),
                to: target.display().to_string(),
                error,
            })?;
        }
    }

    Ok(())
}

fn copy_directory_recursive(source: &Path, target: &Path) -> Result<(), ExtrasError> {
    fs::create_dir_all(target).map_err(|error| ExtrasError::Copy {
        from: source.display().to_string(),
        to: target.display().to_string(),
        error,
    })?;

    for entry in fs::read_dir(source).map_err(|error| ExtrasError::Copy {
        from: source.display().to_string(),
        to: target.display().to_string(),
        error,
    })? {
        let entry = entry.map_err(|error| ExtrasError::Copy {
            from: source.display().to_string(),
            to: target.display().to_string(),
            error,
        })?;

        let child_source = entry.path();
        let child_target = target.join(entry.file_name());

        if child_source.is_dir() {
            copy_directory_recursive(&child_source, &child_target)?;
        } else if child_source.is_file() {
            if let Some(parent) = child_target.parent() {
                fs::create_dir_all(parent).map_err(|error| ExtrasError::Copy {
                    from: child_source.display().to_string(),
                    to: child_target.display().to_string(),
                    error,
                })?;
            }

            fs::copy(&child_source, &child_target).map_err(|error| ExtrasError::Copy {
                from: child_source.display().to_string(),
                to: child_target.display().to_string(),
                error,
            })?;
        }
    }

    Ok(())
}

fn run_git_lines(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    args: &[&str],
) -> Result<Vec<PathBuf>, ExtrasError> {
    let output = runner
        .run("git", args, Some(repo_root))
        .map_err(|error| ExtrasError::Execute(error.to_string()))?;

    if output.status_code != 0 {
        return Err(ExtrasError::CommandFailed {
            command: args.join(" "),
            status: output.status_code,
            stderr: output.stderr.trim().to_string(),
        });
    }

    Ok(output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn validate_relative_path(path: &Path) -> Result<(), ExtrasError> {
    if path.is_absolute() {
        return Err(ExtrasError::InvalidPath(path.display().to_string()));
    }

    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(ExtrasError::InvalidPath(path.display().to_string()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;

    use anyhow::anyhow;

    use crate::command_runner::{CommandOutput, CommandRunner};

    use super::*;

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
    }

    fn output(stdout: &str) -> anyhow::Result<CommandOutput> {
        Ok(CommandOutput {
            status_code: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    #[test]
    fn list_extra_candidates_merges_and_deduplicates() {
        let runner = QueueRunner::new(vec![
            output("a.txt\ncommon.txt\n"),
            output("common.txt\nb.txt\n"),
        ]);
        let entries = list_extra_candidates(Path::new("/tmp/repo"), &runner).expect("entries");
        assert_eq!(
            entries,
            vec![
                PathBuf::from("a.txt"),
                PathBuf::from("b.txt"),
                PathBuf::from("common.txt")
            ]
        );
    }

    #[test]
    fn copy_selected_extras_preserves_relative_paths() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        let target_root = temp.path().join("target");

        fs::create_dir_all(repo_root.join("nested")).expect("nested dir");
        fs::write(repo_root.join("nested").join("file.txt"), "hello").expect("file");

        copy_selected_extras(
            &repo_root,
            &target_root,
            &[PathBuf::from("nested/file.txt")],
        )
        .expect("copy extras");

        let copied = target_root.join("nested").join("file.txt");
        assert!(copied.exists());
        assert_eq!(fs::read_to_string(copied).expect("read copied"), "hello");
    }

    #[test]
    fn copy_selected_extras_rejects_parent_traversal() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        let target_root = temp.path().join("target");

        fs::create_dir_all(&repo_root).expect("repo dir");

        let error = copy_selected_extras(&repo_root, &target_root, &[PathBuf::from("../secret")])
            .expect_err("expected path validation error");

        assert!(matches!(error, ExtrasError::InvalidPath(_)));
    }
}
