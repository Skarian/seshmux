# seshmux

`seshmux` is an interactive Rust CLI for managing git worktrees paired with tmux sessions.

## Configuration

`seshmux` requires a config file before runtime commands (`new`, `list`, `attach`, `delete`) will run.

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

- `seshmux doctor`
- `seshmux new`
- `seshmux list`
- `seshmux attach`
- `seshmux delete`
- `seshmux --help`
