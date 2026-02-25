# seshmux

`seshmux` is a TUI-first Rust tool for managing git worktrees paired with tmux sessions.

Runtime expectations:

- Run `seshmux` from inside a git repository.
- The repository must have at least one commit before runtime flows start.
- Runtime TUI requires a valid config file.

## Configuration

`seshmux` requires a config file before the interactive runtime (`seshmux`) will run.

Create `~/.config/seshmux/config.toml`:

```toml
version = 1

[[tmux.windows]]
name = "codex"
program = "codex"
args = []

[[tmux.windows]]
name = "editor"
program = "nvim"
args = []

[[tmux.windows]]
name = "git"
program = "lazygit"
args = []

[[tmux.windows]]
name = "ops"
shell = ["/bin/zsh", "-lc"]
command = "echo ready && pwd"
```

Launch-mode rules:

- direct mode uses `program` with optional `args`.
- shell mode uses `shell` and `command`.
- each window must use exactly one mode.

## Commands

- `seshmux`
- `seshmux doctor`
- `seshmux --diagnostics`
- `seshmux --diagnostics doctor`
- `seshmux --help`
- `seshmux doctor --help`

## Diagnostics

- `--diagnostics` writes a runtime log to `~/.config/seshmux/diagnostics/<timestamp>.log`.
- If an internal panic occurs, seshmux prints a fatal message and references the diagnostics log path when available.
- Without `--diagnostics`, seshmux still prints a fatal message and tells you to rerun with diagnostics.

## Runtime Flows

The root TUI menu includes:

- `New worktree`
- `List worktrees`
- `Attach session`
- `Delete worktree`

## Key Semantics

- `Esc`: back one step. On first step of a flow, exits that flow.
- `Ctrl+C`: cancel the active command flow.
