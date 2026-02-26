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
        let normalized = normalize_extra_relative_path(&entry)?;
        set.insert(normalized);
    }

    Ok(set.into_iter().collect())
}

pub fn copy_selected_extras(
    repo_root: &Path,
    target_root: &Path,
    selected: &[PathBuf],
) -> Result<(), ExtrasError> {
    for relative in selected {
        let normalized = normalize_extra_relative_path(relative)?;

        let source = repo_root.join(&normalized);
        let target = target_root.join(&normalized);
        copy_existing_path(&source, &target)?;
    }

    Ok(())
}

pub fn normalize_extra_relative_path(path: &Path) -> Result<PathBuf, ExtrasError> {
    if path.is_absolute() {
        return Err(ExtrasError::InvalidPath(path.display().to_string()));
    }

    let mut clean = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => clean.push(value),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExtrasError::InvalidPath(path.display().to_string()));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(ExtrasError::InvalidPath(path.display().to_string()));
    }

    Ok(clean)
}

fn copy_directory_recursive(source: &Path, target: &Path) -> Result<(), ExtrasError> {
    create_dir_all(source, target, target)?;
    for entry in read_dir(source, source, target)? {
        let entry = entry.map_err(|error| copy_error(source, target, error))?;
        let child_source = entry.path();
        let child_target = target.join(entry.file_name());
        copy_existing_path(&child_source, &child_target)?;
    }

    Ok(())
}

fn copy_existing_path(source: &Path, target: &Path) -> Result<(), ExtrasError> {
    if source.is_dir() {
        return copy_directory_recursive(source, target);
    }

    if source.is_file() {
        return copy_file(source, target);
    }

    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> Result<(), ExtrasError> {
    if let Some(parent) = target.parent() {
        create_dir_all(source, target, parent)?;
    }

    fs::copy(source, target).map_err(|error| copy_error(source, target, error))?;
    Ok(())
}

fn create_dir_all(from: &Path, to: &Path, dir: &Path) -> Result<(), ExtrasError> {
    fs::create_dir_all(dir).map_err(|error| copy_error(from, to, error))
}

fn read_dir(dir: &Path, from: &Path, to: &Path) -> Result<fs::ReadDir, ExtrasError> {
    fs::read_dir(dir).map_err(|error| copy_error(from, to, error))
}

fn copy_error(from: &Path, to: &Path, error: std::io::Error) -> ExtrasError {
    ExtrasError::Copy {
        from: from.display().to_string(),
        to: to.display().to_string(),
        error,
    }
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

#[cfg(test)]
mod tests {
    use crate::test_support::{RecordingRunner, output};
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn list_extra_candidates_merges_and_deduplicates() {
        let runner = RecordingRunner::from_outputs(vec![
            output("a.txt\ncommon.txt\n", "", 0),
            output("common.txt\nb.txt\n", "", 0),
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

    #[test]
    fn normalize_extra_relative_path_cleans_curdir_segments() {
        let normalized =
            normalize_extra_relative_path(Path::new("./a/./b.txt")).expect("normalized path");
        assert_eq!(normalized, PathBuf::from("a/b.txt"));
    }

    #[test]
    fn normalize_extra_relative_path_rejects_empty_clean_path() {
        let error = normalize_extra_relative_path(Path::new(".")).expect_err("invalid path");
        assert!(matches!(error, ExtrasError::InvalidPath(_)));
    }
}
