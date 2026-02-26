# Demo GIF workflow

This folder contains a deterministic local workflow for regenerating README demo GIFs with `vhs`.

## Output artifacts

Running the workflow overwrites these files:

- `demo/output/01-create-and-attach.gif`
- `demo/output/02-attach-existing.gif`
- `demo/output/03-delete-and-kill.gif`

## Prerequisites

- `cargo`
- `just`
- `tmux`
- `vhs`

## Run

From repo root:

```bash
just demo
```

`just demo` now regenerates tapes from `demo/timing.env` via `demo/scripts/build_tapes.sh` before rendering GIFs.

## How isolation works

`demo/scripts/setup.sh` recreates `demo/tmp` on each run and writes `demo/tmp/env.sh`.

Demo repositories are recreated under `demo/tmp`:

- `demo/tmp/repo-create`
- `demo/tmp/repo-attach`
- `demo/tmp/repo-delete`

During `just demo`, `demo/scripts/render.sh` temporarily swaps your local seshmux config:

1. move `~/.config/seshmux/config.toml` to `~/.config/seshmux/config.toml.bak` (if present)
2. copy generated demo config into `~/.config/seshmux/config.toml`
3. restore your original file during cleanup and remove the demo one

tmux runs with your real `HOME` and shell config, while demo tmux sessions remain isolated via `TMUX_TMPDIR=demo/tmp/tmux`.

## Troubleshooting

- `vhs: command not found`: install `vhs` and rerun `just demo`.
- Recording timing drifts after TUI changes: adjust semantic timing keys in `demo/timing.env`, then rerun `just demo`.
- Old files/session state: rerun `just demo`; setup fully rebuilds `demo/tmp` and overwrites outputs.
- `refusing to run: backup already exists`: restore or remove `~/.config/seshmux/config.toml.bak`, then rerun.
