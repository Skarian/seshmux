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
        if is_worktrees_relative_path(&normalized) {
            continue;
        }
        if is_symlink_candidate(repo_root, &normalized) {
            continue;
        }
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
        if is_worktrees_relative_path(&normalized) {
            continue;
        }

        let source = repo_root.join(&normalized);
        let target = target_root.join(&normalized);
        copy_existing_path(repo_root, &source, &target)?;
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

fn copy_directory_recursive(
    repo_root: &Path,
    source: &Path,
    target: &Path,
) -> Result<(), ExtrasError> {
    create_dir_all(source, target, target)?;
    for entry in read_dir(source, source, target)? {
        let entry = entry.map_err(|error| copy_error(source, target, error))?;
        let child_source = entry.path();
        let child_target = target.join(entry.file_name());
        copy_existing_path(repo_root, &child_source, &child_target)?;
    }

    Ok(())
}

fn copy_existing_path(repo_root: &Path, source: &Path, target: &Path) -> Result<(), ExtrasError> {
    if is_worktrees_source_path(repo_root, source) {
        return Ok(());
    }

    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(copy_error(source, target, error)),
    };

    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    if metadata.is_dir() {
        return copy_directory_recursive(repo_root, source, target);
    }

    if metadata.is_file() {
        return copy_file(source, target);
    }

    Ok(())
}

fn is_symlink_candidate(repo_root: &Path, relative: &Path) -> bool {
    match fs::symlink_metadata(repo_root.join(relative)) {
        Ok(metadata) => metadata.file_type().is_symlink(),
        Err(_) => true,
    }
}

fn is_worktrees_source_path(repo_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(repo_root) else {
        return false;
    };
    is_worktrees_relative_path(relative)
}

fn is_worktrees_relative_path(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Normal(value)) if value == "worktrees"
    )
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
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use super::*;

    #[test]
    fn list_extra_candidates_merges_and_deduplicates() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&repo_root).expect("repo");
        fs::write(repo_root.join("a.txt"), "a").expect("a");
        fs::write(repo_root.join("b.txt"), "b").expect("b");
        fs::write(repo_root.join("common.txt"), "common").expect("common");

        let runner = RecordingRunner::from_outputs(vec![
            output("a.txt\ncommon.txt\n", "", 0),
            output("common.txt\nb.txt\n", "", 0),
        ]);
        let entries = list_extra_candidates(&repo_root, &runner).expect("entries");
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
    fn list_extra_candidates_skips_worktrees_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(repo_root.join("worktrees/cache")).expect("worktrees dir");
        fs::write(repo_root.join("a.txt"), "a").expect("file a");
        fs::write(repo_root.join("b.txt"), "b").expect("file b");

        let runner = RecordingRunner::from_outputs(vec![
            output("worktrees/\na.txt\n", "", 0),
            output("worktrees/cache/\nb.txt\n", "", 0),
        ]);

        let entries = list_extra_candidates(&repo_root, &runner).expect("entries");
        assert_eq!(
            entries,
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]
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
    fn copy_selected_extras_skips_worktrees_directory_paths() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        let target_root = temp.path().join("target");

        fs::create_dir_all(repo_root.join("worktrees/cache")).expect("worktrees");
        fs::write(repo_root.join("worktrees/cache/state.txt"), "state").expect("state");
        fs::write(repo_root.join("keep.txt"), "keep").expect("keep");

        copy_selected_extras(
            &repo_root,
            &target_root,
            &[
                PathBuf::from("worktrees"),
                PathBuf::from("worktrees/cache/state.txt"),
                PathBuf::from("keep.txt"),
            ],
        )
        .expect("copy extras");

        assert!(target_root.join("keep.txt").exists());
        assert!(!target_root.join("worktrees").exists());
    }

    #[cfg(unix)]
    #[test]
    fn copy_selected_extras_skips_symlink_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        let target_root = temp.path().join("target");
        let outside_root = temp.path().join("outside");

        fs::create_dir_all(&repo_root).expect("repo");
        fs::create_dir_all(&outside_root).expect("outside");
        fs::write(outside_root.join("secret.txt"), "TOPSECRET").expect("secret");
        symlink(outside_root.join("secret.txt"), repo_root.join("link.txt")).expect("symlink");

        copy_selected_extras(&repo_root, &target_root, &[PathBuf::from("link.txt")])
            .expect("copy extras");

        assert!(!target_root.join("link.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn copy_selected_extras_skips_symlink_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        let target_root = temp.path().join("target");
        let outside_root = temp.path().join("outside");

        fs::create_dir_all(&repo_root).expect("repo");
        fs::create_dir_all(outside_root.join("secrets")).expect("outside");
        fs::write(outside_root.join("secrets/file.txt"), "TOPSECRET").expect("secret");
        symlink(outside_root.join("secrets"), repo_root.join("linkdir")).expect("symlink");

        copy_selected_extras(&repo_root, &target_root, &[PathBuf::from("linkdir")])
            .expect("copy extras");

        assert!(!target_root.join("linkdir").exists());
    }

    #[cfg(unix)]
    #[test]
    fn copy_selected_extras_skips_symlink_nested_in_selected_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        let target_root = temp.path().join("target");
        let outside_root = temp.path().join("outside");

        fs::create_dir_all(repo_root.join("assets")).expect("assets");
        fs::create_dir_all(&outside_root).expect("outside");
        fs::write(repo_root.join("assets/keep.txt"), "keep").expect("keep");
        fs::write(outside_root.join("secret.txt"), "TOPSECRET").expect("secret");
        symlink(
            outside_root.join("secret.txt"),
            repo_root.join("assets/link.txt"),
        )
        .expect("symlink");

        copy_selected_extras(&repo_root, &target_root, &[PathBuf::from("assets")])
            .expect("copy extras");

        assert_eq!(
            fs::read_to_string(target_root.join("assets/keep.txt")).expect("copied keep"),
            "keep"
        );
        assert!(!target_root.join("assets/link.txt").exists());
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
