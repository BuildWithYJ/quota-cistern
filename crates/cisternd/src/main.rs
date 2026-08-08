//! The quota-cistern daemon.
//!
//! It listens on a local socket and answers one request per connection. This
//! is where the adapters are built and handed to the core.

// Tests may panic to signal failure.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{fmt::Display, process::ExitCode};

mod adapter;
mod core;
mod handler;

fn main() -> ExitCode {
    if let Err(e) = adapter::socket::remove_on_signal() {
        return quit(e);
    }

    // Failing once here beats failing on every request that arrives.
    let Some(settings) = adapter::settings::FileSettings::in_config_home() else {
        return quit("neither XDG_CONFIG_HOME nor HOME is set");
    };

    let listener = match adapter::socket::listen() {
        Ok(listener) => listener,
        Err(e) => return quit(e),
    };

    match adapter::socket::serve(listener, |request| handler::respond(&settings, request)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => quit(e),
    }
}

/// Says why the daemon is stopping, and stops.
fn quit(e: impl Display) -> ExitCode {
    eprintln!("cisternd: {e}");
    ExitCode::FAILURE
}
