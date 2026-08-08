//! The quota-cistern daemon.
//!
//! It listens on a local socket and answers one request per connection.
//! Nothing it answers touches a domain yet.

// Tests may panic to signal failure.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{fmt::Display, process::ExitCode};

mod adapter;
mod handler;

fn main() -> ExitCode {
    if let Err(e) = adapter::socket::remove_on_signal() {
        return quit(e);
    }

    let listener = match adapter::socket::listen() {
        Ok(listener) => listener,
        Err(e) => return quit(e),
    };

    match adapter::socket::serve(listener, handler::respond) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => quit(e),
    }
}

/// Says why the daemon is stopping, and stops.
fn quit(e: impl Display) -> ExitCode {
    eprintln!("cisternd: {e}");
    ExitCode::FAILURE
}
