//! The quota-cistern command line.
//!
//! Only `--version` is implemented, so this only has to choose between it and
//! the usage clap prints on its own.

use std::process::ExitCode;

mod cli;
mod link;
mod version;

fn main() -> ExitCode {
    let cli = cli::parse();
    if cli.version {
        return version::run();
    }
    ExitCode::SUCCESS
}
