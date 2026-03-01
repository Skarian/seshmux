#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEMO_ROOT="$REPO_ROOT/demo"
TMP_ROOT="$DEMO_ROOT/tmp"
HOME_ROOT="$TMP_ROOT/home"
TMUX_TMPDIR_ROOT="$TMP_ROOT/tmux"
CREATE_REPO="$TMP_ROOT/repo-create"
ATTACH_REPO="$TMP_ROOT/repo-attach"
DELETE_REPO="$TMP_ROOT/repo-delete"
ENV_FILE="$TMP_ROOT/env.sh"
REAL_HOME="${HOME:?}"

if [ -d "$TMUX_TMPDIR_ROOT" ]; then
  TMUX="" TMUX_TMPDIR="$TMUX_TMPDIR_ROOT" tmux kill-server >/dev/null 2>&1 || true
fi

rm -rf "$TMP_ROOT"
mkdir -p \
  "$HOME_ROOT/.config/seshmux" \
  "$HOME_ROOT/bin" \
  "$TMUX_TMPDIR_ROOT" \
  "$CREATE_REPO" \
  "$ATTACH_REPO" \
  "$DELETE_REPO"

VIM_BIN="$(command -v vim || true)"
LAZYGIT_BIN="$(command -v lazygit || true)"
CODEX_BIN="$(command -v codex || true)"
GIT_LOG_COMMAND='while true; do clear; git --no-pager log --oneline --decorate -n 12; sleep 2; done'

cat > "$HOME_ROOT/.config/seshmux/config.toml" <<TOML
version = 1
TOML

if [ -n "$LAZYGIT_BIN" ]; then
  cat >> "$HOME_ROOT/.config/seshmux/config.toml" <<TOML
[[tmux.windows]]
name = "lazygit"
program = "$LAZYGIT_BIN"
TOML
else
  cat >> "$HOME_ROOT/.config/seshmux/config.toml" <<'TOML'
[[tmux.windows]]
name = "git-log"
shell = ["/bin/zsh", "-lc"]
command = "while true; do clear; git --no-pager log --oneline --decorate -n 12; sleep 2; done"
TOML
fi

if [ -n "$CODEX_BIN" ]; then
  cat >> "$HOME_ROOT/.config/seshmux/config.toml" <<TOML
[[tmux.windows]]
name = "codex"
program = "$CODEX_BIN"
TOML
else
  cat >> "$HOME_ROOT/.config/seshmux/config.toml" <<'TOML'
[[tmux.windows]]
name = "codex"
program = "/bin/zsh"
args = ["-lc", "printf 'codex not found in PATH\n'; exec /bin/zsh"]
TOML
fi

if [ -n "$VIM_BIN" ]; then
  cat >> "$HOME_ROOT/.config/seshmux/config.toml" <<TOML
[[tmux.windows]]
name = "vim"
program = "$VIM_BIN"
args = ["-R", "-n", "README.md"]
TOML
else
  cat >> "$HOME_ROOT/.config/seshmux/config.toml" <<'TOML'
[[tmux.windows]]
name = "vim"
program = "/bin/zsh"
TOML
fi

cat > "$HOME_ROOT/bin/seshmux" <<SCRIPT
#!/usr/bin/env bash
set -euo pipefail

exec "$REPO_ROOT/target/debug/seshmux"
SCRIPT

chmod +x "$HOME_ROOT/bin/seshmux"

init_repo() {
  local repo_dir="$1"
  local title="$2"

  git -C "$repo_dir" init -b main >/dev/null
  git -C "$repo_dir" config user.name "Seshmux Demo"
  git -C "$repo_dir" config user.email "demo@seshmux.local"
  git -C "$repo_dir" config commit.gpgsign false

  cat > "$repo_dir/README.md" <<MARKDOWN
# $title

If you are seeing this you created a new worktree and tmux session!
MARKDOWN

  cat > "$repo_dir/.gitignore" <<'GITIGNORE'
worktrees/
GITIGNORE

  git -C "$repo_dir" add README.md .gitignore
  git -C "$repo_dir" commit -m "Initial demo commit" >/dev/null
}

write_launcher() {
  local repo_dir="$1"

  cat > "$repo_dir/demo-tmux" <<SCRIPT
#!/usr/bin/env bash
set -euo pipefail

HOME="$REAL_HOME" TMUX_TMPDIR="$TMUX_TMPDIR_ROOT" PATH="$HOME_ROOT/bin:\$PATH" exec tmux "\$@"
SCRIPT

  chmod +x "$repo_dir/demo-tmux"
}

write_registry() {
  local repo_dir="$1"
  local worktree_name="$2"
  local worktree_path="$3"
  local created_at="$4"

  mkdir -p "$repo_dir/worktrees"

  cat > "$repo_dir/worktrees/worktree.toml" <<TOML
version = 1

[settings.extras]
always_skip_buckets = ["target", "node_modules"]

[[worktree]]
name = "$worktree_name"
path = "$worktree_path"
created_at = "$created_at"
TOML
}

run_demo_tmux() {
  HOME="$REAL_HOME" TMUX_TMPDIR="$TMUX_TMPDIR_ROOT" PATH="$HOME_ROOT/bin:$PATH" tmux "$@"
}

create_demo_session() {
  local session_name="$1"
  local session_cwd="$2"

  if [ -n "$LAZYGIT_BIN" ]; then
    run_demo_tmux new-session \
      -d \
      -s "$session_name" \
      -c "$session_cwd" \
      -n "lazygit" \
      "$LAZYGIT_BIN"
  else
    run_demo_tmux new-session \
      -d \
      -s "$session_name" \
      -c "$session_cwd" \
      -n "git-log" \
      /bin/zsh \
      -lc \
      "$GIT_LOG_COMMAND"
  fi

  if [ -n "$CODEX_BIN" ]; then
    run_demo_tmux new-window \
      -d \
      -t "$session_name" \
      -c "$session_cwd" \
      -n "codex" \
      "$CODEX_BIN"
  else
    run_demo_tmux new-window \
      -d \
      -t "$session_name" \
      -c "$session_cwd" \
      -n "codex" \
      /bin/zsh \
      -lc \
      "printf 'codex not found in PATH\n'; exec /bin/zsh"
  fi

  if [ -n "$VIM_BIN" ]; then
    run_demo_tmux new-window \
      -d \
      -t "$session_name" \
      -c "$session_cwd" \
      -n "vim" \
      "$VIM_BIN" \
      -R \
      -n \
      README.md
  else
    run_demo_tmux new-window \
      -d \
      -t "$session_name" \
      -c "$session_cwd" \
      -n "vim" \
      /bin/zsh
  fi

  run_demo_tmux select-window -t "$session_name:vim"
}

init_repo "$CREATE_REPO" "Create Demo Repo"
init_repo "$ATTACH_REPO" "Attach Demo Repo"
init_repo "$DELETE_REPO" "Delete Demo Repo"

cat > "$ATTACH_REPO/README.md" <<'MARKDOWN'
# Attach Demo Repo

If you are seeing this you attached to a tmux session for a work-tree
MARKDOWN
git -C "$ATTACH_REPO" add README.md
git -C "$ATTACH_REPO" commit -m "Customize attach demo message" >/dev/null

write_launcher "$CREATE_REPO"
write_launcher "$ATTACH_REPO"
write_launcher "$DELETE_REPO"

ATTACH_WORKTREE_PATH="$ATTACH_REPO/worktrees/attach-me"
DELETE_WORKTREE_PATH="$DELETE_REPO/worktrees/delete-me"

git -C "$ATTACH_REPO" worktree add "$ATTACH_WORKTREE_PATH" -b attach-me >/dev/null
git -C "$DELETE_REPO" worktree add "$DELETE_WORKTREE_PATH" -b delete-me >/dev/null

write_registry \
  "$ATTACH_REPO" \
  "attach-me" \
  "$ATTACH_WORKTREE_PATH" \
  "2026-02-26T00:00:00Z"

write_registry \
  "$DELETE_REPO" \
  "delete-me" \
  "$DELETE_WORKTREE_PATH" \
  "2026-02-26T00:00:01Z"

create_demo_session "repo-attach/attach-me" "$ATTACH_WORKTREE_PATH"
create_demo_session "repo-delete/delete-me" "$DELETE_WORKTREE_PATH"

cat > "$ENV_FILE" <<ENV
export SESHMUX_DEMO_HOME="$HOME_ROOT"
export SESHMUX_DEMO_TMUX_TMPDIR="$TMUX_TMPDIR_ROOT"
export SESHMUX_DEMO_CREATE_REPO="$CREATE_REPO"
export SESHMUX_DEMO_ATTACH_REPO="$ATTACH_REPO"
export SESHMUX_DEMO_DELETE_REPO="$DELETE_REPO"
export SESHMUX_DEMO_BIN="$REPO_ROOT/target/debug/seshmux"
ENV

printf "Demo environment ready.\n"
printf -- "- Env file: %s\n" "$ENV_FILE"
printf -- "- Create repo: %s\n" "$CREATE_REPO"
printf -- "- Attach repo: %s\n" "$ATTACH_REPO"
printf -- "- Delete repo: %s\n" "$DELETE_REPO"
