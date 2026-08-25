//! The commands over sessions.
//!
//! Declaring a budget opens a session and asks the supervisor what to start; interrupting asks
//! it to stop. Reading a session back and reading what a run wrote are answered from the
//! stores without a decision at all.

use crate::core::{
    domain::{
        Budget, NotOpened, Opening, Session, SessionId, SessionState, Span, StoppedReason, TaskId,
        TaskState, Usage,
    },
    port::inbound::{
        Declaration, Declared, ExecutionUseCase, Happened, Listed, Page, Ran, Refusal, Report,
        Started, Stopped, Trail,
    },
};

use super::{
    backlog, labelled, sessions,
    supervision::{Outside, Supervisor},
};

/// The commands over sessions.
///
/// What a session does next is not decided here. Declaring a budget opens one and asks the
/// supervisor what to start; interrupting asks it to stop.
pub struct ExecutionService<'a> {
    outside: Outside<'a>,
    supervising: &'a Supervisor<'a>,
}

impl<'a> ExecutionService<'a> {
    pub fn new(outside: Outside<'a>, supervising: &'a Supervisor<'a>) -> Self {
        ExecutionService {
            outside,
            supervising,
        }
    }
}

impl ExecutionUseCase for ExecutionService<'_> {
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal> {
        let usage = Usage::parse(declared.usage).ok_or_else(|| Refusal::BadValue {
            key: "usage".to_owned(),
            value: declared.usage.to_owned(),
        })?;
        let time = Span::parse(declared.time).ok_or_else(|| Refusal::BadValue {
            key: "time".to_owned(),
            value: declared.time.to_owned(),
        })?;

        // Asked before a session is opened.
        // A run with nothing to do would otherwise leave one behind that has to be stopped again.
        if backlog::read(self.outside.tasks)?
            .next_to_assign()
            .is_none()
        {
            return Err(Refusal::NothingToAssign);
        }

        let budget = Budget { usage, time };
        let model = declared.model.map(str::to_owned);

        // Read before the session opens.
        // A share is measured from where the vendor's limit stood when this session had spent nothing.
        let started_at = self.outside.clock.now();
        let limit_at_start = match usage {
            Usage::Share(_) => Some(self.supervising.limit_now()?.0),
            Usage::Tokens(_) => None,
        };

        let opened = sessions::change(self.outside.sessions, |sessions| {
            sessions
                .open(Opening {
                    budget,
                    model,
                    started_at,
                    limit_at_start,
                })
                .map_err(|NotOpened::AlreadyRunning { id }| Refusal::AlreadyRunning {
                    id: id.labelled(),
                })
        })?;

        // Nothing has run yet, so there is nothing to write down between the two.
        let read = self.supervising.measured(opened)?;
        let assigned = self.supervising.settle(opened, read)?;

        Ok(Started {
            session: opened.labelled(),
            state: SessionState::Running.to_string(),
            assigned: assigned.iter().map(TaskId::labelled).collect(),
            budget: Declared {
                usage: usage.to_string(),
                time: time.to_string(),
            },
        })
    }

    fn sessions(&self, page: Option<&str>, limit: Option<&str>) -> Result<Page, Refusal> {
        let page = counted_from("page", page, 1)?;
        let limit = counted_from("limit", limit, 20)?;

        let held = sessions::read(self.outside.sessions)?;
        let tasks = backlog::read(self.outside.tasks)?;

        // Newest first, which is the order the numbers were handed out in.
        let mut newest: Vec<&Session> = held.sessions().iter().collect();
        newest.sort_by_key(|session| std::cmp::Reverse(session.id()));

        let sessions = newest
            .into_iter()
            .skip(((page - 1) * limit) as usize)
            .take(limit as usize)
            .map(|session| Listed {
                id: session.id().labelled(),
                state: session.state().to_string(),
                consumed: session.consumed().to_string(),
                task_count: tasks.taken_by(session.id()).len(),
                updated_at: session.updated_at().to_string(),
            })
            .collect();

        Ok(Page {
            page,
            limit,
            sessions,
        })
    }

    fn session(&self, id: &str) -> Result<Report, Refusal> {
        let wanted = SessionId::parse(id).ok_or_else(|| Refusal::BadValue {
            key: "session".to_owned(),
            value: id.to_owned(),
        })?;

        let held = sessions::read(self.outside.sessions)?;
        let session = held.find(wanted).ok_or_else(|| Refusal::NoSuchSession {
            id: wanted.labelled(),
        })?;

        let tasks = backlog::read(self.outside.tasks)?;
        let ran = tasks
            .taken_by(wanted)
            .into_iter()
            .map(|task| Ran {
                id: task.id().labelled(),
                state: task.state().to_string(),
                title: task.title().to_owned(),
                branch: task.result_branch(),
                reason: task.reason().map(str::to_owned),
            })
            .collect();

        Ok(Report {
            session: session.id().labelled(),
            state: session.state().to_string(),
            budget: Declared {
                usage: session.budget().usage.to_string(),
                time: session.budget().time.to_string(),
            },
            consumed: Declared {
                usage: session.consumed().to_string(),
                time: self.elapsed(session).to_string(),
            },
            stopped_reason: session.stopped_reason().map(|why| why.to_string()),
            // Section 2.2 gives this to a session the vendor turned away. Every share reads it
            // now, so the reason it stopped is what decides, not whether it was ever read.
            resets_at: match session.stopped_reason() {
                Some(StoppedReason::VendorLimit) => session.resets_at().map(|at| at.to_string()),
                _ => None,
            },
            updated_at: session.updated_at().to_string(),
            tasks: ran,
        })
    }

    fn trace(&self, id: &str, since: Option<&str>) -> Result<Trail, Refusal> {
        let wanted = TaskId::parse(id).ok_or_else(|| Refusal::BadValue {
            key: "task".to_owned(),
            value: id.to_owned(),
        })?;
        let held = backlog::read(self.outside.tasks)?;
        let task = held.find(wanted).ok_or_else(|| Refusal::NoSuchTask {
            id: wanted.labelled(),
        })?;
        // Before the trace is read.
        // A run ending between the two would leave a reader holding the last of it and told there is more.
        let done = task.state() != TaskState::Running;

        let read = self
            .outside
            .traces
            .read(&wanted.to_string(), since.unwrap_or_default())?;
        Ok(Trail {
            events: read
                .events
                .into_iter()
                .map(|one| Happened {
                    at: one.at,
                    said: one.said,
                })
                .collect(),
            cursor: read.cursor,
            done,
        })
    }

    fn interrupt(&self) -> Result<Stopped, Refusal> {
        let held = sessions::read(self.outside.sessions)?;
        let running = held.running().ok_or(Refusal::NoSessionRunning)?.id();

        // Before anything is ended.
        // A task the vendor was still running reports nothing, and a share is read from the vendor.
        //
        // Measuring writes what it read. Where the vendor has stopped answering it writes
        // nothing and what was last recorded stands, since a session stopped by hand is
        // stopped either way.
        self.supervising.spending_of(running)?;
        // The same stopping a session reaches on its own, which is where that whole order is
        // written down. Only the reason differs, and only a person gives this one.
        let interrupted = self.supervising.stop(running, StoppedReason::Interrupted)?;

        let held = sessions::read(self.outside.sessions)?;
        let session = held.find(running).ok_or(Refusal::NoSessionRunning)?;

        Ok(Stopped {
            session: running.labelled(),
            state: session.state().to_string(),
            interrupted_tasks: labelled(interrupted),
            consumed: Declared {
                usage: session.consumed().to_string(),
                time: self.elapsed(session).to_string(),
            },
        })
    }
}

/// A count a caller wrote, or what it defaults to when nobody wrote one.
///
/// Zero is refused for both.
/// Section 2.2 names `--page 0` as an argument error, and a page of nothing is the same kind of nothing.
fn counted_from(key: &str, written: Option<&str>, unless: u32) -> Result<u32, Refusal> {
    let Some(written) = written else {
        return Ok(unless);
    };
    written
        .parse()
        .ok()
        .filter(|&count| count > 0)
        .ok_or_else(|| Refusal::BadValue {
            key: key.to_owned(),
            value: written.to_owned(),
        })
}

impl ExecutionService<'_> {
    /// How long the session has run.
    ///
    /// A session still running has run until now.
    /// One that stopped ran until the moment it last changed, which is the moment it stopped.
    fn elapsed(&self, session: &Session) -> Span {
        let until = match session.state() {
            SessionState::Running => self.outside.clock.now(),
            SessionState::Stopped => session.updated_at(),
        };
        Span::of(until.saturating_sub(session.started_at()))
    }
}

#[cfg(test)]
mod tests;
