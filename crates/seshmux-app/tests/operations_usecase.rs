mod support;

use std::fs;

use seshmux_app::{App, AttachError, AttachRequest, DeleteRequest};
use seshmux_core::registry::load_registry;

use support::{ENV_LOCK, QueueRunner, add_registry_entry, output, write_valid_config};

#[test]
fn list_sorts_by_recency_and_includes_runtime_fields() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    let old_path = add_registry_entry(&repo_root, "old", "2026-02-24T10:00:00Z");
    let new_path = add_registry_entry(&repo_root, "new", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("old-branch\n", "", 0),
            output("", "missing session", 1),
            output("new-branch\n", "", 0),
            output("", "", 0),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let result = app.list(&repo_root).expect("list result");

    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0].name, "new");
    assert_eq!(result.rows[0].path, new_path);
    assert_eq!(result.rows[0].branch, "new-branch");
    assert!(result.rows[0].session_running);
    assert_eq!(result.rows[1].name, "old");
    assert_eq!(result.rows[1].path, old_path);
    assert_eq!(result.rows[1].branch, "old-branch");
    assert!(!result.rows[1].session_running);
}

#[test]
fn attach_returns_missing_session_error_when_create_is_false() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "missing session", 1),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let error = app
        .attach(AttachRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            create_if_missing: false,
        })
        .expect_err("missing session error");

    let typed = error
        .downcast_ref::<AttachError>()
        .expect("typed attach error");
    assert!(matches!(typed, AttachError::MissingSession { .. }));
}

#[test]
fn attach_creates_session_when_missing_and_connects() {
    let _guard = ENV_LOCK.lock().expect("env lock");

    let temp = tempfile::tempdir().expect("temp dir");
    write_valid_config(temp.path(), true);
    unsafe {
        std::env::set_var("HOME", temp.path());
    }

    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "missing session", 1),
            output("", "", 0),
            output("", "", 0),
        ],
        vec![Ok(0)],
    );

    let app = App::new(&runner);
    let result = app
        .attach(AttachRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            create_if_missing: true,
        })
        .expect("attach result");

    assert!(result.created_session);
    assert_eq!(result.worktree_name, "w1");

    let calls = runner.calls();
    assert!(calls.iter().any(|call| {
        call.program == "tmux"
            && call.args.first().map(|value| value.as_str()) == Some("new-session")
            && !call.interactive
    }));
    assert!(calls.iter().any(|call| {
        call.program == "tmux"
            && matches!(
                call.args.first().map(|value| value.as_str()),
                Some("attach-session") | Some("switch-client")
            )
            && call.interactive
    }));
}

#[test]
fn delete_with_all_options_kills_session_removes_worktree_and_branch() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    let worktree_path = add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "", 0),
            output("", "", 0),
            output("", "", 0),
            output("", "", 0),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let result = app
        .delete(DeleteRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            kill_tmux_session: true,
            delete_branch: true,
            force_worktree: false,
        })
        .expect("delete result");

    assert_eq!(result.worktree_path, worktree_path);
    assert!(result.branch_deleted);
    assert!(result.branch_delete_error.is_none());
    assert!(load_registry(&repo_root).expect("registry load").is_empty());
}

#[test]
fn delete_keeps_branch_when_not_fully_merged() {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees");
    add_registry_entry(&repo_root, "w1", "2026-02-25T10:00:00Z");

    let runner = QueueRunner::new(
        vec![
            output(&format!("{}\n", repo_root.display()), "", 0),
            output("", "", 0),
            output("", "error: the branch 'w1' is not fully merged.", 1),
        ],
        Vec::new(),
    );

    let app = App::new(&runner);
    let result = app
        .delete(DeleteRequest {
            cwd: repo_root.clone(),
            worktree_name: "w1".to_string(),
            kill_tmux_session: false,
            delete_branch: true,
            force_worktree: false,
        })
        .expect("delete should still succeed");

    assert!(!result.branch_deleted);
    assert!(
        result
            .branch_delete_error
            .as_deref()
            .unwrap_or("")
            .contains("not fully merged")
    );
    assert!(load_registry(&repo_root).expect("registry load").is_empty());
}
