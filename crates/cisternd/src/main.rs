//! The quota-cistern daemon.
//!
//! It listens on a local socket and answers one request per connection. This
//! is where the adapters and the services are built and joined.

// Tests may panic to signal failure.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{fmt::Display, process::ExitCode};

use cistern_contract::exchange;

mod adapter;
mod core;
mod platform;

use adapter::{inbound, outbound};
use core::service::{BacklogService, ConfigurationService};

fn main() -> ExitCode {
    if let Err(e) = platform::signal::remove_on_signal() {
        return quit(e);
    }

    // Failing once here beats failing on every request that arrives.
    let Some(configuration_store) = outbound::configuration::FileConfiguration::in_config_home()
    else {
        return quit("neither XDG_CONFIG_HOME nor HOME is set");
    };
    let Some(backlog_store) = outbound::backlog::FileBacklog::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let roots = outbound::repository::GitRoots;

    let configuration = ConfigurationService::new(&configuration_store);
    let backlog = BacklogService::new(&backlog_store, &roots);

    let server = match exchange::listen() {
        Ok(server) => server,
        Err(e) => return quit(e),
    };

    // Each group owns the names its own commands arrive under, so this offers
    // the request to one and then the next. It grows by a line per group, not
    // by a line per command.
    let answer = |request| {
        inbound::configuration::respond(&configuration, request)
            .or_else(|request| inbound::backlog::respond(&backlog, request))
            .unwrap_or_else(inbound::unknown)
    };
    platform::serve::serve(&server, answer)
}

/// Says why the daemon is stopping, and stops.
fn quit(e: impl Display) -> ExitCode {
    eprintln!("cisternd: {e}");
    ExitCode::FAILURE
}
