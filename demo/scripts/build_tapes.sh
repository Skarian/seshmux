#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEMO_ROOT="$REPO_ROOT/demo"
TAPES_ROOT="$DEMO_ROOT/tapes"
TIMING_FILE="$DEMO_ROOT/timing.env"

if [ ! -f "$TIMING_FILE" ]; then
  printf "missing timing config: %s\n" "$TIMING_FILE" >&2
  exit 1
fi

source "$TIMING_FILE"

require_var() {
  local name="$1"
  if [ -z "${!name:-}" ]; then
    printf "missing required timing variable: %s\n" "$name" >&2
    exit 1
  fi
}

required_vars=(
  TYPING_SPEED
  PLAYBACK_SPEED
  T_BOOTSTRAP_SETTLE_MS
  T_PRELAUNCH_PAUSE_MS
  T_APP_LAUNCH_MS
  T_MENU_MOVE_MS
  T_STANDARD_CONFIRM_MS
  T_NAME_INPUT_PAUSE_MS
  T_SCREEN_TRANSITION_MS
  T_MODAL_CONFIRM_MS
  T_TOGGLE_OPTION_MS
  T_TMUX_PREFIX_MS
  T_SESSION_HANDOFF_MS
  T_RESULT_HOLD_MS
)

for var in "${required_vars[@]}"; do
  require_var "$var"
done

mkdir -p "$TAPES_ROOT"

write_header() {
  local output_file="$1"
  local tape_path="$2"

  cat > "$tape_path" <<EOF
Output "$output_file"

Require bash
Require git
Require tmux

Set Shell "zsh"
Set Theme "Catppuccin Mocha"
Set Width 1280
Set Height 840
Set FontSize 22
Set FontFamily "VictorMono Nerd Font"
Set TypingSpeed $TYPING_SPEED
Set PlaybackSpeed $PLAYBACK_SPEED

EOF
}

build_tape_01() {
  local tape_path="$TAPES_ROOT/01-create-and-attach.tape"
  write_header "demo/output/01-create-and-attach.gif" "$tape_path"

  cat >> "$tape_path" <<EOF
Hide
Type "cd demo/tmp/repo-create"
Enter
Type "./demo-tmux new-session -s seshmux-demo-01 -c ."
Enter
Sleep ${T_BOOTSTRAP_SETTLE_MS}ms
Ctrl+b
Sleep ${T_TMUX_PREFIX_MS}ms
Type ":clear-history"
Enter
Sleep ${T_TMUX_PREFIX_MS}ms
Ctrl+l
Show

Sleep ${T_PRELAUNCH_PAUSE_MS}ms
Type "seshmux"
Enter
Sleep ${T_APP_LAUNCH_MS}ms

Enter
Sleep ${T_STANDARD_CONFIRM_MS}ms

Type "demo-feature"
Sleep ${T_NAME_INPUT_PAUSE_MS}ms
Enter
Sleep ${T_STANDARD_CONFIRM_MS}ms

Enter
Sleep ${T_STANDARD_CONFIRM_MS}ms

Enter
Sleep ${T_STANDARD_CONFIRM_MS}ms

Enter
Sleep ${T_STANDARD_CONFIRM_MS}ms

Enter
Sleep ${T_SESSION_HANDOFF_MS}ms
EOF
}

build_tape_02() {
  local tape_path="$TAPES_ROOT/02-attach-existing.tape"
  write_header "demo/output/02-attach-existing.gif" "$tape_path"

  cat >> "$tape_path" <<EOF
Hide
Type "cd demo/tmp/repo-attach"
Enter
Type "./demo-tmux new-session -s seshmux-demo-02 -c ."
Enter
Sleep ${T_BOOTSTRAP_SETTLE_MS}ms
Ctrl+b
Sleep ${T_TMUX_PREFIX_MS}ms
Type ":clear-history"
Enter
Sleep ${T_TMUX_PREFIX_MS}ms
Ctrl+l
Show

Sleep ${T_PRELAUNCH_PAUSE_MS}ms
Type "seshmux"
Enter
Sleep ${T_APP_LAUNCH_MS}ms

Down
Sleep ${T_MENU_MOVE_MS}ms
Down
Sleep ${T_MENU_MOVE_MS}ms
Enter
Sleep ${T_SCREEN_TRANSITION_MS}ms

Enter
Sleep ${T_MODAL_CONFIRM_MS}ms

Ctrl+b
Sleep ${T_TMUX_PREFIX_MS}ms
Type "d"
Sleep ${T_SESSION_HANDOFF_MS}ms
EOF
}

build_tape_03() {
  local tape_path="$TAPES_ROOT/03-delete-and-kill.tape"
  write_header "demo/output/03-delete-and-kill.gif" "$tape_path"

  cat >> "$tape_path" <<EOF
Hide
Type "cd demo/tmp/repo-delete"
Enter
Type "./demo-tmux new-session -s seshmux-demo-03 -c ."
Enter
Sleep ${T_BOOTSTRAP_SETTLE_MS}ms
Ctrl+b
Sleep ${T_TMUX_PREFIX_MS}ms
Type ":clear-history"
Enter
Sleep ${T_TMUX_PREFIX_MS}ms
Ctrl+l
Show

Sleep ${T_PRELAUNCH_PAUSE_MS}ms
Type "seshmux"
Enter
Sleep ${T_APP_LAUNCH_MS}ms

Down
Sleep ${T_MENU_MOVE_MS}ms
Down
Sleep ${T_MENU_MOVE_MS}ms
Down
Sleep ${T_MENU_MOVE_MS}ms
Enter
Sleep ${T_SCREEN_TRANSITION_MS}ms

Enter
Sleep ${T_MODAL_CONFIRM_MS}ms

Space
Sleep ${T_TOGGLE_OPTION_MS}ms
Down
Sleep ${T_TOGGLE_OPTION_MS}ms
Space
Sleep ${T_TOGGLE_OPTION_MS}ms
Enter
Sleep ${T_MODAL_CONFIRM_MS}ms

Space
Sleep ${T_TOGGLE_OPTION_MS}ms
Enter
Sleep ${T_RESULT_HOLD_MS}ms
EOF
}

build_tape_01
build_tape_02
build_tape_03

printf "Generated tapes:\n"
printf -- "- %s\n" "$TAPES_ROOT/01-create-and-attach.tape"
printf -- "- %s\n" "$TAPES_ROOT/02-attach-existing.tape"
printf -- "- %s\n" "$TAPES_ROOT/03-delete-and-kill.tape"
