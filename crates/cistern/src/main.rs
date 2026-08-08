//! The quota-cistern command line.

use std::process::ExitCode;

mod cli;
mod config;
mod link;
mod task;
mod version;

fn main() -> ExitCode {
    let cli = cli::parse();
    if cli.version {
        return version::run();
    }

    match cli.command {
        Some(cli::Command::Config { command }) => config::run(command),
        Some(cli::Command::Task { command }) => task::run(command),
        Some(cli::Command::Backlog) => task::backlog(),
        // clap answers this on its own, since arg_required_else_help is set.
        None => ExitCode::SUCCESS,
    }
}
