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
    /// Sets the vendor.
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
    /// Reads what a task's run has written.
    Trace {
        task: String,
        /// Keeps printing until the task ends.
        #[arg(long)]
        follow: bool,
        /// Prints only what follows that point.
        #[arg(long)]
        since: Option<String>,
    },
    /// Stops the running session.
    Interrupt,
    /// Reads sessions.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Declares a budget and starts the session's unattended loop.
    Run {
        /// A share of the vendor's five-hour limit, or a count of tokens.
        #[arg(long)]
        usage: String,
        /// How long the session may run.
        #[arg(long)]
        time: String,
        /// Default model for tasks that name none.
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Lists sessions, newest first.
    Ls {
        /// Page number. Defaults to 1.
        #[arg(long)]
        page: Option<String>,
        /// Items per page. Defaults to 20.
        #[arg(long)]
        limit: Option<String>,
    },
    /// Reads one session and the tasks it held.
    Show { session: String },
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
