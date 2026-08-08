//! What the command line accepts.
//!
//! `docs/cli.md` specifies it. Only what is implemented appears here.

use clap::{Parser, Subcommand};

/// Budget-based workload scheduler for coding agents.
#[derive(Debug, Parser)]
#[command(
    name = "cistern",
    // clap would answer --version itself with the crate version. Ours has to
    // reach the core, so it is an ordinary flag.
    disable_version_flag = true,
    // docs/cli.md: no subcommand prints usage and exits 2.
    arg_required_else_help = true
)]
pub struct Cli {
    /// Prints the version of this command and of the core it reaches.
    #[arg(long)]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sets the vendor and the plan.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Registers and reads tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Lists the tasks waiting to be assigned.
    Backlog,
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// Adds a task to the backlog.
    Add {
        /// Task title.
        #[arg(long)]
        title: String,
        /// Instruction for the agent. `-` reads from stdin.
        #[arg(long)]
        instruction: String,
        /// Branch the task starts from.
        #[arg(long)]
        branch: Option<String>,
        /// Predecessor task. This task waits until that one completes.
        #[arg(long)]
        after: Option<String>,
        /// Model for this task.
        #[arg(long)]
        model: Option<String>,
    },
    /// Removes a task from the backlog.
    Rm { task: String },
    /// Prints the detail of one task.
    Show { task: String },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Stores a value under a key.
    Set { key: String, value: String },
    /// Prints one key, or the whole configuration when no key is given.
    Get { key: Option<String> },
}

/// Reads the command line.
///
/// It stands between `main` and clap, so that swapping the parser touches only
/// this file.
pub fn parse() -> Cli {
    Cli::parse()
}
