use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::command_runner::{CommandOutput, CommandRunner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchSource {
    Local,
    Remote,
}

impl BranchSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    pub name: String,
    pub source: BranchSource,
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRef {
    pub hash: String,
    pub short_hash: String,
    pub subject: String,
    pub display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchDeleteMode {
    Safe,
    Force,
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git command failed: git {command} (exit {status}) {stderr}")]
    CommandFailed {
        command: String,
        status: i32,
        stderr: String,
    },
    #[error("failed to execute git command: {0}")]
    Execute(String),
    #[error("failed to parse git output: {0}")]
    Parse(String),
    #[error(
        "repository has no commits yet; create an initial commit or choose a different start point"
    )]
    NoCommits,
    #[error("branch '{branch}' is not fully merged; explicit force is required")]
    BranchNotFullyMerged { branch: String },
}

pub fn repo_root(cwd: &Path, runner: &dyn CommandRunner) -> Result<PathBuf, GitError> {
    let output = run_git_checked(runner, &["rev-parse", "--show-toplevel"], Some(cwd))?;
    let root = output
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| GitError::Parse("git rev-parse returned empty repo root".to_string()))?;

    Ok(PathBuf::from(root))
}

pub fn gitignore_contains_worktrees(repo_root: &Path) -> Result<bool, GitError> {
    let gitignore_path = repo_root.join(".gitignore");

    if !gitignore_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(&gitignore_path)
        .map_err(|error| GitError::Execute(error.to_string()))?;

    for line in content.lines().map(str::trim) {
        if line == "worktrees/" || line == "/worktrees/" {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn ensure_worktrees_gitignore_entry(repo_root: &Path) -> Result<(), GitError> {
    if gitignore_contains_worktrees(repo_root)? {
        return Ok(());
    }

    let gitignore_path = repo_root.join(".gitignore");
    let mut content = if gitignore_path.exists() {
        std::fs::read_to_string(&gitignore_path)
            .map_err(|error| GitError::Execute(error.to_string()))?
    } else {
        String::new()
    };

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }

    content.push_str("worktrees/\n");

    std::fs::write(&gitignore_path, content)
        .map_err(|error| GitError::Execute(error.to_string()))?;

    Ok(())
}

pub fn resolve_current_start_point(
    repo_root: &Path,
    runner: &dyn CommandRunner,
) -> Result<String, GitError> {
    let output = run_git(runner, &["rev-parse", "--verify", "HEAD"], Some(repo_root))?;

    if output.status_code != 0 {
        return Err(GitError::NoCommits);
    }

    Ok("HEAD".to_string())
}

pub fn create_worktree(
    repo_root: &Path,
    worktree_name: &str,
    target_path: &Path,
    start_point: &str,
    runner: &dyn CommandRunner,
) -> Result<(), GitError> {
    let target = target_path
        .to_str()
        .ok_or_else(|| GitError::Parse("worktree path is not valid UTF-8".to_string()))?;

    let args = ["worktree", "add", "-b", worktree_name, target, start_point];
    run_git_checked(runner, &args, Some(repo_root))?;

    Ok(())
}

pub fn current_branch(
    worktree_path: &Path,
    runner: &dyn CommandRunner,
) -> Result<String, GitError> {
    let output = run_git_checked(
        runner,
        &["rev-parse", "--abbrev-ref", "HEAD"],
        Some(worktree_path),
    )?;

    let branch = output
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| GitError::Parse("git rev-parse returned empty branch name".to_string()))?;

    Ok(branch.to_string())
}

pub fn remove_worktree(
    repo_root: &Path,
    target_path: &Path,
    runner: &dyn CommandRunner,
) -> Result<(), GitError> {
    let target = target_path
        .to_str()
        .ok_or_else(|| GitError::Parse("worktree path is not valid UTF-8".to_string()))?;

    run_git_checked(runner, &["worktree", "remove", target], Some(repo_root))?;
    Ok(())
}

pub fn delete_branch(
    repo_root: &Path,
    branch_name: &str,
    mode: BranchDeleteMode,
    runner: &dyn CommandRunner,
) -> Result<(), GitError> {
    let branch = branch_name.trim();
    if branch.is_empty() {
        return Err(GitError::Parse("branch name cannot be empty".to_string()));
    }

    let flag = match mode {
        BranchDeleteMode::Safe => "-d",
        BranchDeleteMode::Force => "-D",
    };

    let output = run_git(runner, &["branch", flag, branch], Some(repo_root))?;
    if output.status_code == 0 {
        return Ok(());
    }

    if matches!(mode, BranchDeleteMode::Safe) && looks_like_branch_not_fully_merged(&output.stderr)
    {
        return Err(GitError::BranchNotFullyMerged {
            branch: branch.to_string(),
        });
    }

    Err(GitError::CommandFailed {
        command: format!("branch {flag} {branch}"),
        status: output.status_code,
        stderr: output.stderr.trim().to_string(),
    })
}

pub fn query_branches(
    repo_root: &Path,
    query: &str,
    runner: &dyn CommandRunner,
) -> Result<Vec<BranchRef>, GitError> {
    let mut branches = Vec::new();

    let local_output = run_git_checked(
        runner,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        Some(repo_root),
    )?;
    branches.extend(parse_branch_lines(
        &local_output.stdout,
        BranchSource::Local,
    ));

    let remote_output = run_git_checked(
        runner,
        &["for-each-ref", "--format=%(refname:short)", "refs/remotes"],
        Some(repo_root),
    )?;
    branches.extend(parse_branch_lines(
        &remote_output.stdout,
        BranchSource::Remote,
    ));

    let query_normalized = query.trim().to_lowercase();

    let mut filtered: Vec<BranchRef> = branches
        .into_iter()
        .filter(|branch| {
            if branch.name.ends_with("/HEAD") {
                return false;
            }

            if query_normalized.is_empty() {
                true
            } else {
                branch.name.to_lowercase().contains(&query_normalized)
            }
        })
        .collect();

    filtered.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(filtered)
}

pub fn query_commits(
    repo_root: &Path,
    query: &str,
    limit: usize,
    runner: &dyn CommandRunner,
) -> Result<Vec<CommitRef>, GitError> {
    let effective_limit = if limit == 0 { 50 } else { limit };
    let trimmed = query.trim();

    let output = if trimmed.is_empty() {
        let limit_value = effective_limit.to_string();
        let args = ["log", "--format=%H%x1f%h%x1f%s", "-n", limit_value.as_str()];
        run_git(runner, &args, Some(repo_root))?
    } else {
        run_git(
            runner,
            &["log", "--all", "--format=%H%x1f%h%x1f%s"],
            Some(repo_root),
        )?
    };

    if output.status_code != 0 {
        if looks_like_empty_history(&output.stderr) {
            return Ok(Vec::new());
        }

        return Err(GitError::CommandFailed {
            command: if trimmed.is_empty() {
                format!("log --format=%H%x1f%h%x1f%s -n {effective_limit}")
            } else {
                "log --all --format=%H%x1f%h%x1f%s".to_string()
            },
            status: output.status_code,
            stderr: output.stderr.trim().to_string(),
        });
    }

    let mut commits = parse_commit_lines(&output.stdout)?;

    if !trimmed.is_empty() {
        let normalized = trimmed.to_lowercase();
        commits.retain(|commit| commit.hash.to_lowercase().contains(&normalized));
        commits.truncate(effective_limit);
    }

    Ok(commits)
}

fn parse_branch_lines(raw: &str, source: BranchSource) -> Vec<BranchRef> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|name| BranchRef {
            name: name.to_string(),
            display: format!("{} [{}]", name, source.as_str()),
            source: source.clone(),
        })
        .collect()
}

fn parse_commit_lines(raw: &str) -> Result<Vec<CommitRef>, GitError> {
    let mut commits = Vec::new();

    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let mut parts = line.splitn(3, '\u{1f}');

        let hash = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| GitError::Parse("missing commit hash in git log output".to_string()))?;

        let short_hash = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| GitError::Parse("missing short hash in git log output".to_string()))?;

        let subject = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| GitError::Parse("missing subject in git log output".to_string()))?;

        commits.push(CommitRef {
            hash: hash.to_string(),
            short_hash: short_hash.to_string(),
            subject: subject.to_string(),
            display: format!("{short_hash} {subject}"),
        });
    }

    Ok(commits)
}

fn looks_like_empty_history(stderr: &str) -> bool {
    let normalized = stderr.to_lowercase();
    normalized.contains("does not have any commits yet")
        || normalized.contains("your current branch")
            && normalized.contains("does not have any commits yet")
        || normalized.contains("fatal: ambiguous argument 'head'")
        || normalized.contains("unknown revision or path not in the working tree")
}

fn looks_like_branch_not_fully_merged(stderr: &str) -> bool {
    let normalized = stderr.to_lowercase();
    normalized.contains("not fully merged")
}

fn run_git_checked(
    runner: &dyn CommandRunner,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<CommandOutput, GitError> {
    let output = run_git(runner, args, cwd)?;

    if output.status_code != 0 {
        return Err(GitError::CommandFailed {
            command: args.join(" "),
            status: output.status_code,
            stderr: output.stderr.trim().to_string(),
        });
    }

    Ok(output)
}

fn run_git(
    runner: &dyn CommandRunner,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<CommandOutput, GitError> {
    runner
        .run("git", args, cwd)
        .map_err(|error| GitError::Execute(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
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
    fn query_branches_applies_source_labels() {
        let runner = QueueRunner::new(vec![
            output("main\nfeature\n", "", 0),
            output("origin/main\norigin/feature\n", "", 0),
        ]);

        let branches = query_branches(Path::new("."), "", &runner).expect("branches");

        assert_eq!(branches.len(), 4);
        assert_eq!(branches[0].display, "feature [local]");
        assert_eq!(branches[3].display, "origin/main [remote]");
    }

    #[test]
    fn query_commits_returns_latest_list() {
        let runner = QueueRunner::new(vec![output(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{1f}aaaaaaa\u{1f}first\n",
            "",
            0,
        )]);

        let commits = query_commits(Path::new("."), "", 50, &runner).expect("commits");

        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].display, "aaaaaaa first");
    }

    #[test]
    fn query_commits_filters_search_results_and_limits_output() {
        let runner = QueueRunner::new(vec![output(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{1f}aaaaaaa\u{1f}first\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\u{1f}bbbbbbb\u{1f}second\naaaabbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\u{1f}aaaabbb\u{1f}third\n",
            "",
            0,
        )]);

        let commits = query_commits(Path::new("."), "aaaa", 2, &runner).expect("commits");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].short_hash, "aaaaaaa");
        assert_eq!(commits[1].short_hash, "aaaabbb");
    }

    #[test]
    fn query_commits_returns_empty_when_history_is_empty() {
        let runner = QueueRunner::new(vec![output(
            "",
            "fatal: your current branch 'main' does not have any commits yet",
            128,
        )]);

        let commits = query_commits(Path::new("."), "", 50, &runner).expect("no commits");
        assert!(commits.is_empty());
    }

    #[test]
    fn resolve_current_start_point_returns_no_commits_error() {
        let runner = QueueRunner::new(vec![output(
            "",
            "fatal: ambiguous argument 'HEAD': unknown revision",
            128,
        )]);

        let error = resolve_current_start_point(Path::new("."), &runner).expect_err("should fail");
        assert!(matches!(error, GitError::NoCommits));
    }

    #[test]
    fn current_branch_reads_head_name() {
        let runner = QueueRunner::new(vec![output("feature-1\n", "", 0)]);
        let branch = current_branch(Path::new("."), &runner).expect("branch");
        assert_eq!(branch, "feature-1");
    }

    #[test]
    fn delete_branch_safe_reports_not_fully_merged() {
        let runner = QueueRunner::new(vec![output(
            "",
            "error: the branch 'feature-1' is not fully merged.",
            1,
        )]);

        let error = delete_branch(Path::new("."), "feature-1", BranchDeleteMode::Safe, &runner)
            .expect_err("branch should require force");

        assert!(matches!(error, GitError::BranchNotFullyMerged { .. }));
    }

    #[test]
    fn delete_branch_force_succeeds() {
        let runner = QueueRunner::new(vec![output("", "", 0)]);
        let result = delete_branch(
            Path::new("."),
            "feature-1",
            BranchDeleteMode::Force,
            &runner,
        );
        assert!(result.is_ok());
    }
}
