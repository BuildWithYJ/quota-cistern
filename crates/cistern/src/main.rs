//! The quota-cistern command line.

use std::process::ExitCode;

mod cli;
mod config;
mod daemon;
mod review;
mod session;
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
        Some(cli::Command::Trace {
            task,
            follow,
            since,
        }) => task::trace(&task, follow, since),
        Some(cli::Command::Diff { task, stat }) => review::diff(&task, stat),
        Some(cli::Command::Review { command }) => review::run(command),
        Some(cli::Command::Apply { task }) => review::apply(&task),
        Some(cli::Command::Discard { task }) => review::discard(&task),
        Some(cli::Command::Interrupt) => session::interrupt(),
        Some(cli::Command::Session { command }) => session::query(command),
        Some(cli::Command::Run { usage, time, model }) => session::run(&usage, &time, model),
        // clap answers this on its own, since arg_required_else_help is set.
        None => ExitCode::SUCCESS,
    }
}
