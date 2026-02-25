use std::fs;
use std::path::Path;
use std::process::Command;

use seshmux_app::{App, DeleteRequest};
use seshmux_core::command_runner::SystemCommandRunner;
use seshmux_core::registry::{RegistryEntry, find_entry_by_name, insert_unique_entry};

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command should execute");

    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn delete_removes_worktree_folder_and_registry_entry_in_real_repo() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("repo dir");

    run_git(&repo_root, &["init"]);
    fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
    run_git(&repo_root, &["add", "."]);
    run_git(
        &repo_root,
        &[
            "-c",
            "user.name=seshmux-test",
            "-c",
            "user.email=seshmux-test@example.com",
            "commit",
            "-m",
            "initial",
        ],
    );

    let worktrees_dir = repo_root.join("worktrees");
    fs::create_dir_all(&worktrees_dir).expect("worktrees dir");
    let worktree_path = worktrees_dir.join("w1");
    run_git(
        &repo_root,
        &[
            "worktree",
            "add",
            "-b",
            "w1",
            worktree_path.to_str().expect("path utf8"),
            "HEAD",
        ],
    );

    insert_unique_entry(
        &repo_root,
        RegistryEntry {
            name: "w1".to_string(),
            path: worktree_path.to_string_lossy().to_string(),
            created_at: "2026-02-25T10:00:00Z".to_string(),
        },
    )
    .expect("insert registry entry");

    let runner = SystemCommandRunner::new();
    let app = App::new(&runner);
    let result = app
        .delete(DeleteRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            kill_tmux_session: false,
            delete_branch: false,
        })
        .expect("delete should succeed");

    assert_eq!(result.worktree_name, "w1");
    assert!(!result.branch_deleted);
    assert!(!worktree_path.exists());
    assert!(
        find_entry_by_name(&repo_root, "w1")
            .expect("load registry")
            .is_none()
    );
}
