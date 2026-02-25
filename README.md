# seshmux

`seshmux` is a TUI-first Rust tool for managing git worktrees paired with tmux sessions.

Runtime expectations:

- Run `seshmux` from inside a git repository.
- The repository must have at least one commit before runtime flows start.

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
- `seshmux --help`
- `seshmux doctor --help`
