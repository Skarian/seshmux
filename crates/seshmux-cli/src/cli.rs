use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "seshmux")]
#[command(bin_name = "seshmux")]
#[command(version)]
#[command(about = "Interactive git worktree + tmux workflow manager")]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Write diagnostics to ~/.config/seshmux/diagnostics/<timestamp>.log"
    )]
    pub diagnostics: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Run environment and configuration checks")]
    Doctor,
}
