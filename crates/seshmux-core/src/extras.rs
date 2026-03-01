use std::collections::{BTreeMap, BTreeSet};
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
    let raw = collect_git_extra_paths_nul_two_pass(repo_root, runner)?;
    filter_safe_extra_paths(repo_root, raw)
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

pub fn classify_flagged_buckets(
    candidates: &[PathBuf],
    skip_rules: &BTreeSet<String>,
) -> BTreeMap<String, usize> {
    let mut buckets = BTreeMap::<String, usize>::new();
    if skip_rules.is_empty() {
        return buckets;
    }
    let rule_patterns = compiled_rule_patterns(skip_rules);
    if rule_patterns.is_empty() {
        return buckets;
    }

    for candidate in candidates {
        let Some(bucket) = matched_bucket_for_rules(candidate, &rule_patterns) else {
            continue;
        };
        *buckets.entry(bucket).or_insert(0) += 1;
    }

    buckets
}

pub fn filter_candidates_by_skipped_buckets(
    candidates: &[PathBuf],
    skipped_buckets: &BTreeSet<String>,
) -> Vec<PathBuf> {
    if skipped_buckets.is_empty() {
        return candidates.to_vec();
    }
    let skipped_bucket_components = compiled_bucket_components(skipped_buckets);
    if skipped_bucket_components.is_empty() {
        return candidates.to_vec();
    }

    candidates
        .iter()
        .filter(|candidate| {
            !candidate_matches_any_skipped_bucket(candidate, &skipped_bucket_components)
        })
        .cloned()
        .collect()
}

pub fn depth_two_bucket_key(path: &Path) -> Option<String> {
    let components = normalized_components(path)?;
    let first = components.first()?;
    let second = components.get(1);

    Some(match second {
        Some(second) => format!("{first}/{second}"),
        None => first.to_string(),
    })
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
        return Ok(());
    }

    if metadata.is_file() {
        return copy_file(source, target);
    }

    Ok(())
}

#[derive(Debug)]
struct RulePattern {
    components: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct RuleMatch {
    start: usize,
    len: usize,
    end: usize,
}

fn matched_bucket_for_rules(path: &Path, rule_patterns: &[RulePattern]) -> Option<String> {
    let components = normalized_components(path)?;
    if components.len() < 2 {
        return None;
    }
    let directory_components = &components[..components.len() - 1];

    let mut best_match: Option<RuleMatch> = None;
    for rule in rule_patterns {
        let Some(matched) = earliest_match_for_rule(directory_components, &rule.components) else {
            continue;
        };
        let should_replace = best_match
            .map(|existing| {
                matched.start < existing.start
                    || (matched.start == existing.start && matched.len > existing.len)
            })
            .unwrap_or(true);
        if should_replace {
            best_match = Some(matched);
        }
    }

    best_match.map(|matched| directory_components[..matched.end].join("/"))
}

fn earliest_match_for_rule(
    directory_components: &[String],
    rule_components: &[String],
) -> Option<RuleMatch> {
    if rule_components.is_empty() || rule_components.len() > directory_components.len() {
        return None;
    }

    for start in 0..=(directory_components.len() - rule_components.len()) {
        let end = start + rule_components.len();
        if directory_components[start..end] == *rule_components {
            return Some(RuleMatch {
                start,
                len: rule_components.len(),
                end,
            });
        }
    }

    None
}

fn compiled_rule_patterns(skip_rules: &BTreeSet<String>) -> Vec<RulePattern> {
    skip_rules
        .iter()
        .filter_map(|rule| {
            normalize_rule_components(rule).map(|components| RulePattern { components })
        })
        .collect()
}

fn compiled_bucket_components(skipped_buckets: &BTreeSet<String>) -> Vec<Vec<String>> {
    skipped_buckets
        .iter()
        .filter_map(|bucket| normalize_rule_components(bucket))
        .collect()
}

fn normalize_rule_components(value: &str) -> Option<Vec<String>> {
    normalized_components(Path::new(value))
}

fn candidate_matches_any_skipped_bucket(
    candidate: &Path,
    skipped_bucket_components: &[Vec<String>],
) -> bool {
    let Some(candidate_components) = normalized_components(candidate) else {
        return false;
    };

    skipped_bucket_components.iter().any(|bucket_components| {
        candidate_components.len() >= bucket_components.len()
            && candidate_components[..bucket_components.len()] == **bucket_components
    })
}

fn normalized_components(path: &Path) -> Option<Vec<String>> {
    let normalized = normalize_extra_relative_path(path).ok()?;
    let mut components = Vec::<String>::new();
    for component in normalized.components() {
        let Component::Normal(value) = component else {
            return None;
        };
        components.push(value.to_string_lossy().to_string());
    }

    if components.is_empty() {
        None
    } else {
        Some(components)
    }
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

fn copy_error(from: &Path, to: &Path, error: std::io::Error) -> ExtrasError {
    ExtrasError::Copy {
        from: from.display().to_string(),
        to: to.display().to_string(),
        error,
    }
}

pub fn collect_git_extra_paths_nul_two_pass(
    repo_root: &Path,
    runner: &dyn CommandRunner,
) -> Result<Vec<PathBuf>, ExtrasError> {
    let untracked = run_git_stdout(
        runner,
        repo_root,
        &["ls-files", "-o", "-z", "--exclude-standard", "--", "."],
    )?;
    let ignored = run_git_stdout(
        runner,
        repo_root,
        &[
            "ls-files",
            "-o",
            "-i",
            "-z",
            "--exclude-standard",
            "--",
            ".",
        ],
    )?;

    let mut merged = BTreeSet::new();
    for path in parse_nul_paths(&untracked)
        .into_iter()
        .chain(parse_nul_paths(&ignored))
    {
        merged.insert(path);
    }

    Ok(merged.into_iter().collect())
}

pub fn parse_nul_paths(stdout: &str) -> Vec<PathBuf> {
    stdout
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn filter_safe_extra_paths(
    repo_root: &Path,
    raw: Vec<PathBuf>,
) -> Result<Vec<PathBuf>, ExtrasError> {
    let mut filtered = BTreeSet::new();

    for entry in raw {
        let normalized = normalize_extra_relative_path(&entry)?;
        if is_worktrees_relative_path(&normalized) {
            continue;
        }
        if is_symlink_candidate(repo_root, &normalized) {
            continue;
        }
        filtered.insert(normalized);
    }

    Ok(filtered.into_iter().collect())
}

fn run_git_stdout(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    args: &[&str],
) -> Result<String, ExtrasError> {
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

    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use crate::test_support::{RecordingRunner, output};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::time::Instant;

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
            output("a.txt\0common.txt\0", "", 0),
            output("common.txt\0b.txt\0", "", 0),
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
            output("worktrees/cache/state.txt\0a.txt\0", "", 0),
            output("worktrees/cache/more.txt\0b.txt\0", "", 0),
        ]);

        let entries = list_extra_candidates(&repo_root, &runner).expect("entries");
        assert_eq!(
            entries,
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]
        );
    }

    #[test]
    fn collect_two_pass_paths_uses_expected_git_invocations() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&repo_root).expect("repo");

        let runner = RecordingRunner::from_outputs(vec![
            output("one.txt\0", "", 0),
            output("two.txt\0", "", 0),
        ]);
        let _entries = collect_git_extra_paths_nul_two_pass(&repo_root, &runner).expect("entries");
        let calls = runner.calls();

        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].args,
            vec!["ls-files", "-o", "-z", "--exclude-standard", "--", "."]
        );
        assert_eq!(
            calls[1].args,
            vec![
                "ls-files",
                "-o",
                "-i",
                "-z",
                "--exclude-standard",
                "--",
                "."
            ]
        );
    }

    #[test]
    fn parse_nul_paths_ignores_empty_segments() {
        let parsed = parse_nul_paths("one.txt\0\0nested/two.txt\0");
        assert_eq!(
            parsed,
            vec![PathBuf::from("one.txt"), PathBuf::from("nested/two.txt")]
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
    fn copy_selected_extras_skips_symlink_leaf_input() {
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

        copy_selected_extras(
            &repo_root,
            &target_root,
            &[
                PathBuf::from("assets/keep.txt"),
                PathBuf::from("assets/link.txt"),
            ],
        )
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
    fn copy_selected_extras_directory_input_is_ignored_and_copies_nothing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        let target_root = temp.path().join("target");

        fs::create_dir_all(repo_root.join("crates/seshmux-tui/src/ui")).expect("extras path");
        fs::create_dir_all(repo_root.join("crates/seshmux-tui/src/new_flow")).expect("sibling");
        fs::write(repo_root.join("crates/.DS_Store"), "meta").expect("meta");
        fs::write(
            repo_root.join("crates/seshmux-tui/src/ui/loading.rs"),
            "loading",
        )
        .expect("loading");
        fs::write(
            repo_root.join("crates/seshmux-tui/src/new_flow/mod.rs"),
            "tracked-sibling",
        )
        .expect("sibling file");

        let candidate_leaves = [
            PathBuf::from("crates/.DS_Store"),
            PathBuf::from("crates/seshmux-tui/src/ui/loading.rs"),
        ];
        assert!(
            !candidate_leaves
                .iter()
                .any(|path| path == &PathBuf::from("crates/seshmux-tui/src/new_flow/mod.rs"))
        );

        copy_selected_extras(&repo_root, &target_root, &[PathBuf::from("crates")]).expect("copy");

        assert!(!target_root.join("crates/.DS_Store").exists());
        assert!(
            !target_root
                .join("crates/seshmux-tui/src/ui/loading.rs")
                .exists()
        );
        assert!(
            !target_root
                .join("crates/seshmux-tui/src/new_flow/mod.rs")
                .exists()
        );
    }

    #[test]
    fn classify_flagged_buckets_matches_rules_at_any_depth() {
        let candidates = vec![
            PathBuf::from("target/debug/deps/a.o"),
            PathBuf::from("target/.rustc_info.json"),
            PathBuf::from("app/mobile/target/build/b.o"),
            PathBuf::from("pkg/.cache/a.bin"),
            PathBuf::from("app/mobile/vendor/bundle/gem.rb"),
            PathBuf::from("app/mobile/vendor/cache/tmp.bin"),
            PathBuf::from("src/main.rs"),
        ];
        let skip_rules = BTreeSet::from([
            "target".to_string(),
            ".cache".to_string(),
            "vendor".to_string(),
            "vendor/bundle".to_string(),
        ]);

        let flagged = classify_flagged_buckets(&candidates, &skip_rules);

        assert_eq!(flagged.get("target"), Some(&2));
        assert_eq!(flagged.get("app/mobile/target"), Some(&1));
        assert_eq!(flagged.get("pkg/.cache"), Some(&1));
        assert_eq!(flagged.get("app/mobile/vendor/bundle"), Some(&1));
        assert_eq!(flagged.get("app/mobile/vendor"), Some(&1));
        assert!(!flagged.contains_key("target/debug"));
        assert!(!flagged.contains_key("src/main.rs"));
    }

    #[test]
    fn classify_flagged_buckets_prefers_closest_to_root_then_longest_at_same_start() {
        let candidates = vec![
            PathBuf::from("target/debug/build/rustix/out/file.txt"),
            PathBuf::from("app/mobile/target/debug/build/rustix/out/file.txt"),
            PathBuf::from("app/mobile/vendor/bundle/gem.rb"),
        ];
        let skip_rules = BTreeSet::from([
            "target".to_string(),
            "build".to_string(),
            "out".to_string(),
            "vendor".to_string(),
            "vendor/bundle".to_string(),
        ]);

        let flagged = classify_flagged_buckets(&candidates, &skip_rules);

        assert_eq!(flagged.get("target"), Some(&1));
        assert_eq!(flagged.get("app/mobile/target"), Some(&1));
        assert_eq!(flagged.get("app/mobile/vendor/bundle"), Some(&1));
        assert!(!flagged.contains_key("target/debug/build/rustix/out"));
        assert!(!flagged.contains_key("app/mobile/target/debug/build/rustix/out"));
        assert!(!flagged.contains_key("app/mobile/vendor"));
    }

    #[test]
    fn filter_candidates_by_skipped_buckets_omits_matching_directory_prefixes() {
        let candidates = vec![
            PathBuf::from("target/debug/deps/a.o"),
            PathBuf::from("targeted/release/b.o"),
            PathBuf::from("app/mobile/target/build/c.o"),
            PathBuf::from("app/mobile/targeted/build/d.o"),
            PathBuf::from("app/mobile/vendor/bundle/gem.rb"),
            PathBuf::from("app/mobile/vendor/other/file.txt"),
            PathBuf::from("src/main.rs"),
        ];
        let skipped = BTreeSet::from([
            "target".to_string(),
            "app/mobile/target".to_string(),
            "app/mobile/vendor/bundle".to_string(),
        ]);

        let filtered = filter_candidates_by_skipped_buckets(&candidates, &skipped);

        assert_eq!(
            filtered,
            vec![
                PathBuf::from("targeted/release/b.o"),
                PathBuf::from("app/mobile/targeted/build/d.o"),
                PathBuf::from("app/mobile/vendor/other/file.txt"),
                PathBuf::from("src/main.rs"),
            ]
        );
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

    #[test]
    #[ignore]
    fn timing_receipt_collect_git_extra_paths_large_synthetic_input() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&repo_root).expect("repo");

        let mut untracked = String::new();
        for index in 0..20_000 {
            untracked.push_str(&format!("scratch/file-{index}.txt\0"));
        }

        let mut ignored = String::new();
        for index in 0..90_000 {
            ignored.push_str(&format!("target/debug/deps/object-{index}.o\0"));
        }

        let runner =
            RecordingRunner::from_outputs(vec![output(&untracked, "", 0), output(&ignored, "", 0)]);

        let started = Instant::now();
        let paths = collect_git_extra_paths_nul_two_pass(&repo_root, &runner).expect("paths");
        let elapsed = started.elapsed();

        println!(
            "TIMING collect_git_extra_paths_nul_two_pass entries={} elapsed_ms={}",
            paths.len(),
            elapsed.as_millis()
        );
        assert_eq!(paths.len(), 110_000);
    }
}
