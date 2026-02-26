#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEMO_ROOT="$REPO_ROOT/demo"
OUTPUT_ROOT="$DEMO_ROOT/output"
ENV_FILE="$DEMO_ROOT/tmp/env.sh"
GENERATED_CONFIG="$DEMO_ROOT/tmp/home/.config/seshmux/config.toml"
USER_CONFIG_DIR="$HOME/.config/seshmux"
USER_CONFIG_PATH="$USER_CONFIG_DIR/config.toml"
USER_CONFIG_BACKUP_PATH="$USER_CONFIG_DIR/config.toml.bak"
USER_CONFIG_SWAP_STATE="none"

cd "$REPO_ROOT"

cleanup_server_sessions() {
  local tmux_tmpdir="${1:-}"
  local sessions

  if [ -n "$tmux_tmpdir" ]; then
    sessions="$(TMUX="" TMUX_TMPDIR="$tmux_tmpdir" tmux list-sessions -F '#{session_name}' 2>/dev/null || true)"
  else
    sessions="$(tmux list-sessions -F '#{session_name}' 2>/dev/null || true)"
  fi

  if [ -z "$sessions" ]; then
    return
  fi

  while IFS= read -r session_name; do
    case "$session_name" in
      seshmux-demo-*|repo-create/*|repo-attach/*|repo-delete/*)
        if [ -n "$tmux_tmpdir" ]; then
          TMUX="" TMUX_TMPDIR="$tmux_tmpdir" tmux kill-session -t "$session_name" >/dev/null 2>&1 || true
        else
          tmux kill-session -t "$session_name" >/dev/null 2>&1 || true
        fi
        ;;
    esac
  done <<< "$sessions"
}

cleanup_demo_sessions() {
  cleanup_server_sessions

  if [ -f "$ENV_FILE" ]; then
    source "$ENV_FILE"
    if [ -n "${SESHMUX_DEMO_TMUX_TMPDIR:-}" ]; then
      cleanup_server_sessions "$SESHMUX_DEMO_TMUX_TMPDIR"
    fi
  fi
}

install_demo_user_config() {
  if [ -f "$USER_CONFIG_BACKUP_PATH" ]; then
    printf "refusing to run: backup already exists at %s\n" "$USER_CONFIG_BACKUP_PATH" >&2
    exit 1
  fi

  mkdir -p "$USER_CONFIG_DIR"

  if [ -f "$USER_CONFIG_PATH" ]; then
    mv "$USER_CONFIG_PATH" "$USER_CONFIG_BACKUP_PATH"
    USER_CONFIG_SWAP_STATE="moved"
  fi

  cp "$GENERATED_CONFIG" "$USER_CONFIG_PATH"
  USER_CONFIG_SWAP_STATE="installed"
}

restore_user_config() {
  if [ "$USER_CONFIG_SWAP_STATE" = "none" ]; then
    return
  fi

  if [ "$USER_CONFIG_SWAP_STATE" = "installed" ]; then
    rm -f "$USER_CONFIG_PATH"
  fi

  if [ -f "$USER_CONFIG_BACKUP_PATH" ]; then
    mv "$USER_CONFIG_BACKUP_PATH" "$USER_CONFIG_PATH"
  fi
}

cleanup_all() {
  restore_user_config
  cleanup_demo_sessions
}

cleanup_demo_sessions
trap cleanup_all EXIT

"$DEMO_ROOT/scripts/setup.sh"
install_demo_user_config
"$DEMO_ROOT/scripts/build_tapes.sh"
cargo build -p seshmux-cli

mkdir -p "$OUTPUT_ROOT"
rm -f \
  "$OUTPUT_ROOT/01-create-and-attach.gif" \
  "$OUTPUT_ROOT/02-attach-existing.gif" \
  "$OUTPUT_ROOT/03-delete-and-kill.gif"

vhs "$DEMO_ROOT/tapes/01-create-and-attach.tape"
vhs "$DEMO_ROOT/tapes/02-attach-existing.tape"
vhs "$DEMO_ROOT/tapes/03-delete-and-kill.tape"

printf "Generated demo GIFs:\n"
printf -- "- %s\n" "$OUTPUT_ROOT/01-create-and-attach.gif"
printf -- "- %s\n" "$OUTPUT_ROOT/02-attach-existing.gif"
printf -- "- %s\n" "$OUTPUT_ROOT/03-delete-and-kill.gif"
