//! The quota-cistern daemon.
//!
//! It listens on a local socket and answers one request per connection. This
//! is where the adapters and the services are built and joined.

// Tests may panic to signal failure.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{fmt::Display, process::ExitCode, thread};

use cistern_contract::exchange;

mod adapter;
mod core;
mod platform;

use adapter::{inbound, outbound};
use core::{
    port::inbound::{Declaration, ExecutionUseCase, Refusal, Started},
    service::{BacklogService, ConfigurationService, ExecutionService},
};
use platform::work::Queue;

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
    let Some(session_store) = outbound::session::FileSessions::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let Some(worktrees) = outbound::worktree::GitWorktrees::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let roots = outbound::repository::GitRoots;
    let agent = match outbound::agent::ClaudeAgent::new() {
        Ok(agent) => agent,
        Err(e) => return quit(e.reason),
    };

    let configuration = ConfigurationService::new(&configuration_store);
    let backlog = BacklogService::new(&backlog_store, &roots);
    let execution = ExecutionService::new(
        &session_store,
        &backlog_store,
        &configuration_store,
        &worktrees,
        &agent,
    );

    let server = match exchange::listen() {
        Ok(server) => server,
        Err(e) => return quit(e),
    };

    // A task outlives the request that assigned it, so what a run leaves behind
    // is queued here and a thread beside the accept loop carries it on. The
    // core says what one task is; when it happens is arranged here.
    let queued = Queue::default();
    let execution = Queueing {
        execution: &execution,
        queued: &queued,
    };

    thread::scope(|threads| {
        threads.spawn(|| {
            loop {
                let task = queued.take();
                if let Err(e) = execution.execution.carry_on(&task) {
                    eprintln!("cisternd: {task} could not be carried on: {e:?}");
                }
            }
        });

        // Each group owns the names its own commands arrive under, so this
        // offers the request to one and then the next. It grows by a line per
        // group, not by a line per command.
        let answer = |request| {
            inbound::configuration::respond(&configuration, request)
                .or_else(|request| inbound::backlog::respond(&backlog, request))
                .or_else(|request| inbound::execution::respond(&execution, request))
                .unwrap_or_else(inbound::unknown)
        };
        platform::serve::serve(&server, answer)
    })
}

/// The core's own execution, with what it assigned put on the queue.
///
/// It stands between the adapter and the service so that neither has to know
/// there are threads. Joining them is what this file is for.
struct Queueing<'a, U: ExecutionUseCase> {
    execution: &'a U,
    queued: &'a Queue,
}

impl<U: ExecutionUseCase> ExecutionUseCase for Queueing<'_, U> {
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal> {
        let started = self.execution.run(declared)?;
        for task in &started.assigned {
            self.queued.add(task.clone());
        }
        Ok(started)
    }

    fn carry_on(&self, task: &str) -> Result<(), Refusal> {
        self.execution.carry_on(task)
    }
}

/// Says why the daemon is stopping, and stops.
fn quit(e: impl Display) -> ExitCode {
    eprintln!("cisternd: {e}");
    ExitCode::FAILURE
}
