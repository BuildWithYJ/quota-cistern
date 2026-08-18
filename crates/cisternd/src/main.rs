//! The quota-cistern daemon.
//!
//! It listens on a local socket and answers one request per connection.
//! This is where the adapters and the services are built and joined.

// Tests may panic to signal failure.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{fmt::Display, process::ExitCode, thread};

use cistern_contract::exchange;

mod adapter;
mod core;
mod platform;

use adapter::{inbound, outbound};
use core::port::outbound::ConfigurationStore;
use core::{
    port::inbound::{
        Carrying, Declaration, ExecutionUseCase, NotCarried, Page, Refusal, Report, Started,
        Stopped, Trail,
    },
    service::{BacklogService, ConfigurationService, ExecutionService, Outside, ReviewService},
};
use platform::work::Queue;

/// How many tasks the daemon has hands for.
///
/// A guard on the machine: each task is a checkout of a repository and an agent process of
/// its own.
/// The core is told this number rather than holding one of its own, so that it never assigns
/// more than there are threads to run.
const AT_ONCE: usize = 4;

fn main() -> ExitCode {
    if let Err(e) = platform::signal::remove_on_signal() {
        return quit(e);
    }

    // Failing once here beats failing on every request that arrives.
    let Some(configuration_store) =
        outbound::file::configuration::FileConfiguration::in_config_home()
    else {
        return quit("neither XDG_CONFIG_HOME nor HOME is set");
    };
    let Some(backlog_store) = outbound::file::backlog::FileBacklog::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let Some(session_store) = outbound::file::session::FileSessions::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let Some(worktrees) = outbound::git::worktree::GitWorktrees::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let Some(limit) = outbound::claude::limit::ClaudeLimit::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let limit = match limit {
        Ok(limit) => limit,
        Err(e) => return quit(e.reason),
    };
    let Some(traces) = outbound::file::trace::FileTraces::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    // The names this build can run. Adding a vendor is a directory beside
    // `claude` and a line here; the core is not touched.
    let known = [outbound::claude::NAME.to_owned()];
    if let Err(e) = runnable(&configuration_store, &known) {
        return quit(e);
    }

    let roots = outbound::git::roots::GitRoots;
    let results = outbound::git::result::GitResults;
    let surroundings = outbound::git::surroundings::GitSurroundings;
    let drafter = outbound::claude::drafter::ClaudeDrafter;
    let clock = outbound::clock::SystemClock;
    let agent = match outbound::claude::agent::ClaudeAgent::new() {
        Ok(agent) => agent,
        Err(e) => return quit(e.reason),
    };

    let configuration = ConfigurationService::new(&configuration_store, known);
    let backlog = BacklogService::new(&backlog_store, &roots, &results, &surroundings, &drafter);
    let review = ReviewService::new(&backlog_store, &results);
    let execution = ExecutionService::new(
        Outside {
            sessions: &session_store,
            tasks: &backlog_store,
            worktrees: &worktrees,
            agent: &agent,
            clock: &clock,
            limit: &limit,
            traces: &traces,
        },
        AT_ONCE,
    );

    let server = match exchange::listen() {
        Ok(server) => server,
        Err(e) => return quit(e),
    };

    // A task outlives the request that assigned it.
    // The core says what one task is, and this arranges when it happens.
    let queued = Queue::default();
    let execution = Queueing {
        execution: &execution,
        queued: &queued,
    };

    thread::scope(|threads| {
        // The same number the core was given, so a task it assigns has a thread waiting.
        for _ in 0..AT_ONCE {
            threads.spawn(|| {
                loop {
                    let task = queued.take();
                    if let Err(e) = execution.carry_on(&task) {
                        eprintln!("cisternd: {task} was not carried on: {}", why(&e));
                    }
                }
            });
        }

        // Each group owns the names its commands arrive under, so this offers the request to one and then the next.
        let answer = |request| {
            inbound::configuration::respond(&configuration, request)
                .or_else(|request| inbound::backlog::respond(&backlog, request))
                .or_else(|request| inbound::execution::respond(&execution, request))
                .or_else(|request| inbound::review::respond(&review, request))
                .unwrap_or_else(inbound::unknown)
        };
        platform::serve::serve(&server, answer)
    })
}

/// The core's own execution, with what it assigned put on the queue.
///
/// It stands between the adapter and the service so that neither has to know there are threads.
struct Queueing<'a, U> {
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

    fn sessions(&self, page: Option<&str>, limit: Option<&str>) -> Result<Page, Refusal> {
        self.execution.sessions(page, limit)
    }

    fn session(&self, id: &str) -> Result<Report, Refusal> {
        self.execution.session(id)
    }

    fn trace(&self, id: &str, since: Option<&str>) -> Result<Trail, Refusal> {
        self.execution.trace(id, since)
    }

    fn interrupt(&self) -> Result<Stopped, Refusal> {
        self.execution.interrupt()
    }
}

impl<U: Carrying> Carrying for Queueing<'_, U> {
    fn carry_on(&self, task: &str) -> Result<Vec<String>, NotCarried> {
        let assigned = self.execution.carry_on(task)?;
        for task in &assigned {
            self.queued.add(task.clone());
        }
        Ok(assigned)
    }
}

/// Refuses to start on a stored vendor this build cannot run.
///
/// The check belongs here because only this file knows which adapters it holds.
/// Failing once beats failing on every task a session assigns.
fn runnable(store: &dyn ConfigurationStore, known: &[String]) -> Result<(), String> {
    let held = store.load().map_err(|e| e.reason)?;
    let Some((_, name)) = held.iter().find(|(key, _)| key == "vendor") else {
        return Ok(());
    };
    if known.contains(name) {
        return Ok(());
    }
    Err(format!(
        "the configuration says vendor {name}, which this build cannot run; it runs {}",
        known.join(", ")
    ))
}

/// What a worker says when a task could not be carried on.
///
/// It goes to the daemon's own output rather than to a surface, so it is one sentence for
/// whoever is reading the log.
fn why(e: &NotCarried) -> String {
    match e {
        NotCarried::NoSuchTask { id } => format!("{id} is no longer in the backlog"),
        NotCarried::Unavailable { reason } => format!("a store could not be read: {reason}"),
    }
}

/// Says why the daemon is stopping, and stops.
fn quit(e: impl Display) -> ExitCode {
    eprintln!("cisternd: {e}");
    ExitCode::FAILURE
}
