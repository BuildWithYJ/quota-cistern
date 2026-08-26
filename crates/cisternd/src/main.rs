//! The quota-cistern daemon.
//!
//! It listens on a local socket and answers one request per connection.
//! This is where the adapters and the services are built and joined.

// Tests may panic to signal failure.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{fmt::Display, process::ExitCode, thread, time::Duration};

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
    service::{
        BacklogService, ConfigurationService, ExecutionService, Outside, ReviewService, Supervisor,
        WorkService,
    },
};
use platform::work::Queue;

/// How many tasks the daemon has hands for.
///
/// A guard on the machine: each task is a checkout of a repository and an agent process of
/// its own.
/// The core is told this number rather than holding one of its own, so that it never assigns
/// more than there are threads to run.
///
/// A placeholder. Nothing has measured what a machine carries, and the one thing known about
/// the figure it replaces is that four bound before any budget worth declaring did. What would
/// settle it is a session run against a real backlog until the machine or the vendor is the
/// thing that gives, rather than this.
const AT_ONCE: usize = 20;

/// The vendor a configuration that names none falls back to.
const BY_DEFAULT: &str = "claude";

/// The longest this waits before looking again for a session to hold to its deadline.
///
/// A ceiling rather than a period. A session says how long it has and this sleeps that long
/// where that is shorter, so a deadline is met within a second of itself; a session that has
/// none, or one already past its deadline, waits out the whole of this instead of spinning.
const LOOKS_EVERY: Duration = Duration::from_secs(60);

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
    // Read once and handed to both of the things that ask it something, so a file changed
    // between two reads cannot leave the daemon started under two different configurations.
    let held = match configuration_store.load() {
        Ok(held) => held,
        Err(e) => return quit(e.reason),
    };
    // The names there is a definition for, whether it ships or the user placed it.
    // Adding a vendor is a file; nothing here and nothing in the core is touched.
    let known = outbound::program::Definition::known();
    let named = match chosen(&held, &known) {
        Ok(named) => named,
        Err(e) => return quit(e),
    };
    let definition = match outbound::program::Definition::found(&named) {
        Ok(definition) => definition,
        Err(e) => return quit(e.reason),
    };
    let Some(limit) = outbound::program::limit::ProgramLimit::in_data_home(definition.clone())
    else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };
    let Some(traces) = outbound::file::trace::FileTraces::in_data_home(shapes_of(&definition))
    else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };

    let Some(runs) = outbound::file::run::FileRuns::in_data_home() else {
        return quit("neither XDG_DATA_HOME nor HOME is set");
    };

    let roots = outbound::git::roots::GitRoots;
    let results = outbound::git::result::GitResults;
    let surroundings = outbound::git::surroundings::GitSurroundings;
    let clock = outbound::clock::SystemClock;
    // Asked before a task is registered rather than while one runs, so it holds a
    // definition of its own rather than taking one from the agent.
    let drafter = outbound::program::drafter::ProgramDrafter::new(definition.clone());
    let agent = outbound::program::agent::ProgramAgent::new(definition);

    let configuration = ConfigurationService::new(&configuration_store, known);
    let backlog = BacklogService::new(&backlog_store, &roots, &results, &surroundings, &drafter);
    let review = ReviewService::new(&backlog_store, &results, &worktrees);
    // The ports, once. Each service takes its own copy rather than reaching for another's.
    let outside = Outside {
        sessions: &session_store,
        tasks: &backlog_store,
        worktrees: &worktrees,
        agent: &agent,
        clock: &clock,
        limit: &limit,
        traces: &traces,
        runs: &runs,
    };

    // One judgement, asked by the commands that need a decision and by the workers that carry
    // out what one assigned.
    let supervisor = match Supervisor::chosen_by(outside, AT_ONCE, &held) {
        Ok(supervisor) => supervisor,
        Err(e) => return quit(e),
    };
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    let server = match exchange::listen() {
        Ok(server) => server,
        Err(e) => return quit(e),
    };

    // A task outlives the request that assigned it.
    // The core says what one task is, and this arranges when it happens.
    let queued = Queue::default();
    let execution = Queueing {
        service: &execution,
        queued: &queued,
    };
    let work = Queueing {
        service: &work,
        queued: &queued,
    };

    // Each group owns the names its commands arrive under, so this offers the request to one and then the next.
    // Made outside the threads below, since each of them answers with it and one made inside
    // would go before they do.
    let answer = |request| {
        inbound::configuration::respond(&configuration, request)
            .or_else(|request| inbound::backlog::respond(&backlog, request))
            .or_else(|request| inbound::execution::respond(&execution, request))
            .or_else(|request| inbound::review::respond(&review, request))
            .unwrap_or_else(inbound::unknown)
    };

    thread::scope(|threads| {
        // The other half of the budget. A decision is reached when a task ends, so a session
        // with one long run going would pass the time it declared with nobody looking.
        threads.spawn(|| {
            loop {
                if let Err(e) = supervisor.stop_if_out_of_time() {
                    eprintln!("cisternd: the deadline could not be checked: {e:?}");
                }
                // Never longer than the interval and never shorter. Longer, and a session
                // opened while this slept would not be looked at until the one it read had
                // run out. Shorter, and a session past its deadline with a run still going
                // would be looked at every second for as long as that run takes, which is a
                // read and a write of the session store each time against every worker
                // recording a run.
                thread::sleep(match supervisor.time_left() {
                    Ok(Some(left)) => LOOKS_EVERY.min(Duration::from_secs(left.max(1))),
                    _ => LOOKS_EVERY,
                });
            }
        });

        // The same number the core was given, so a task it assigns has a thread waiting.
        for _ in 0..AT_ONCE {
            threads.spawn(|| {
                loop {
                    let task = queued.take();
                    if let Err(e) = work.carry_on(&task) {
                        eprintln!("cisternd: {task} was not carried on: {}", why(&e));
                    }
                }
            });
        }

        platform::serve::serve(&server, &answer, threads)
    })
}

/// The core's own execution, with what it assigned put on the queue.
///
/// It stands between the adapter and the service so that neither has to know there are threads.
struct Queueing<'a, S> {
    service: &'a S,
    queued: &'a Queue,
}

impl<S: ExecutionUseCase> ExecutionUseCase for Queueing<'_, S> {
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal> {
        let started = self.service.run(declared)?;
        for task in &started.assigned {
            self.queued.add(task.clone());
        }
        Ok(started)
    }

    fn sessions(&self, page: Option<&str>, limit: Option<&str>) -> Result<Page, Refusal> {
        self.service.sessions(page, limit)
    }

    fn session(&self, id: &str) -> Result<Report, Refusal> {
        self.service.session(id)
    }

    fn trace(&self, id: &str, since: Option<&str>) -> Result<Trail, Refusal> {
        self.service.trace(id, since)
    }

    fn interrupt(&self) -> Result<Stopped, Refusal> {
        self.service.interrupt()
    }
}

impl<S: Carrying> Carrying for Queueing<'_, S> {
    fn carry_on(&self, task: &str) -> Result<Vec<String>, NotCarried> {
        let assigned = self.service.carry_on(task)?;
        for task in &assigned {
            self.queued.add(task.clone());
        }
        Ok(assigned)
    }
}

/// Which vendor to run, refusing a name nothing defines.
///
/// Failing once here beats failing on every task a session assigns.
fn chosen(held: &[(String, String)], known: &[String]) -> Result<String, String> {
    let Some((_, name)) = held.iter().find(|(key, _)| key == "vendor") else {
        return Ok(BY_DEFAULT.to_owned());
    };
    if known.contains(name) {
        return Ok(name.clone());
    }
    Err(format!(
        "the configuration says vendor {name}, which nothing defines; there is {}",
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

/// What a vendor's stream lines are shaped like, as the trace store asks for it.
///
/// The two are named apart on purpose. A file store has no business naming a vendor's
/// module, so the names cross here rather than through a reference between adapters.
fn shapes_of(definition: &outbound::program::Definition) -> outbound::file::trace::Shapes {
    let held = &definition.trace;
    outbound::file::trace::Shapes {
        said: held.said.clone(),
        came_back: held.came_back.clone(),
        blocks: held.blocks.clone(),
        text: held.text.clone(),
        reached_for: held.reached_for.clone(),
        result: held.result.clone(),
        errored: held.errored.clone(),
        held: held.held.clone(),
        called: held.called.clone(),
        given: held.given.clone(),
        subject: held.subject.clone(),
        subject_path: held.subject_path.clone(),
    }
}

/// Says why the daemon is stopping, and stops.
fn quit(e: impl Display) -> ExitCode {
    eprintln!("cisternd: {e}");
    ExitCode::FAILURE
}
