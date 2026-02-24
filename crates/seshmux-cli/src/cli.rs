use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "seshmux")]
#[command(bin_name = "seshmux")]
#[command(version)]
#[command(about = "Interactive git worktree + tmux workflow manager")]
#[command(arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Create a new worktree and tmux session")]
    New,
    #[command(about = "List registered worktrees and tmux status")]
    List,
    #[command(about = "Attach to an existing worktree session")]
    Attach,
    #[command(about = "Delete a worktree and optional resources")]
    Delete,
    #[command(about = "Run environment and configuration checks")]
    Doctor,
}
