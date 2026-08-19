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

        let assigned = self.supervising.settle(opened)?;

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
        let session = held
            .sessions()
            .iter()
            .find(|session| session.id() == wanted)
            .ok_or_else(|| Refusal::NoSuchSession {
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
        let now = self.outside.clock.now();

        // The runs end before the tasks do, so nothing is recorded as ended while its agent is still working.
        for task in backlog::read(self.outside.tasks)?.taken_by(running) {
            if task.state() == TaskState::Running {
                self.outside.agent.stop(&task.id().to_string());
            }
        }

        let interrupted = backlog::change(self.outside.tasks, |tasks| {
            Ok(tasks.interrupt(running, &StoppedReason::Interrupted.to_string(), now))
        })?;
        sessions::change(self.outside.sessions, |sessions| {
            sessions.stop(running, StoppedReason::Interrupted, now);
            Ok(())
        })?;

        let held = sessions::read(self.outside.sessions)?;
        let session = held
            .sessions()
            .iter()
            .find(|session| session.id() == running)
            .ok_or(Refusal::NoSessionRunning)?;

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

impl ExecutionService<'_> {}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use crate::core::{
        domain::HUNDREDTHS,
        port::{
            inbound::{Carrying, ExecutionUseCase},
            outbound::{Ended, Outcome, StoredSession, StoredSessions, StoredTask},
        },
    };

    use super::super::fixtures::*;
    use super::super::{ExecutionService, Outside, Supervisor, WorkService};
    use super::*;

    #[test]
    fn a_session_opens_and_answers_what_it_was_declared_with() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        let started = execution.run(declaring("50%", "8h")).unwrap();
        assert_eq!(started.session, "session:1");
        assert_eq!(started.state, "running");
        assert_eq!(started.assigned, vec!["task:1".to_owned()]);
        assert_eq!(started.budget.usage, "50%");
        assert_eq!(started.budget.time, "8h");
    }

    #[test]
    fn what_was_opened_is_there_for_the_next_command_to_read() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        execution.run(declaring("2M", "30m")).unwrap();

        let held = sessions.load();
        assert_eq!(held.next_id, "2");
        assert_eq!(held.sessions.len(), 1);
        assert_eq!(held.sessions[0].state, "running");
        assert_eq!(held.sessions[0].usage, "2000000");
    }

    /// A task that was assigned is running and belongs to the session that took it.
    /// Nothing else may take it as well.
    #[test]
    fn the_task_that_was_assigned_says_which_session_took_it() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        execution.run(declaring("50%", "8h")).unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Running");
        assert_eq!(held.session.as_deref(), Some("1"));
    }

    #[test]
    fn a_second_session_is_refused_while_one_is_running() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        execution.run(declaring("50%", "8h")).unwrap();

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::AlreadyRunning {
                id: "session:1".to_owned()
            })
        );
        assert_eq!(sessions.load().sessions.len(), 1);
    }

    #[test]
    fn a_run_with_nothing_to_start_is_refused_and_opens_no_session() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(Vec::new());
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::NothingToAssign)
        );
        assert!(sessions.load().sessions.is_empty());
    }

    #[test]
    fn a_declaration_that_cannot_be_read_is_refused_as_a_bad_argument() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        assert_eq!(
            execution.run(declaring("50%", "8x")),
            Err(Refusal::BadValue {
                key: "time".to_owned(),
                value: "8x".to_owned()
            })
        );
        assert_eq!(
            execution.run(declaring("half", "8h")),
            Err(Refusal::BadValue {
                key: "usage".to_owned(),
                value: "half".to_owned()
            })
        );
    }

    #[test]
    fn a_stored_session_that_cannot_be_read_fails_as_a_store() {
        let sessions = Remembered::holding(StoredSessions {
            next_id: "2".to_owned(),
            sessions: vec![StoredSession {
                started_at: "1000".to_owned(),
                limit_at_start: None,
                limit_last_seen: None,
                consumed: "0".to_owned(),
                updated_at: "1000".to_owned(),
                resets_at: None,
                id: "1".to_owned(),
                state: "sprinting".to_owned(),
                stopped_reason: None,
                usage: "50%".to_owned(),
                time: "8h".to_owned(),
                model: None,
            }],
        });
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        let refused = execution.run(declaring("50%", "8h")).unwrap_err();
        assert!(matches!(refused, Refusal::Unavailable { reason } if reason.contains("sprinting")));
    }

    #[test]
    fn a_page_that_is_not_a_page_is_refused_as_a_bad_argument() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        for (page, limit) in [(Some("0"), None), (None, Some("0")), (Some("one"), None)] {
            assert!(matches!(
                execution.sessions(page, limit),
                Err(Refusal::BadValue { .. })
            ));
        }
    }

    #[test]
    fn nothing_has_run_and_the_list_is_empty() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        assert_eq!(execution.sessions(None, None).unwrap().sessions, Vec::new());
    }

    /// Section 2.2 leaves the reason empty while a session runs.
    #[test]
    fn a_running_session_says_nothing_about_why_it_stopped() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();

        let report = execution.session("1").unwrap();
        assert_eq!(report.state, "running");
        assert_eq!(report.stopped_reason, None);
        assert_eq!(report.resets_at, None);
    }

    #[test]
    fn a_session_nobody_opened_is_not_there() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        assert_eq!(
            execution.session("7"),
            Err(Refusal::NoSuchSession {
                id: "session:7".to_owned()
            })
        );
    }

    #[test]
    fn interrupting_stops_the_session_and_ends_what_was_running() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        let stopped = execution.interrupt().unwrap();

        assert_eq!(stopped.session, "session:1");
        assert_eq!(stopped.state, "stopped");
        assert_eq!(stopped.interrupted_tasks, ["task:1"]);

        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.stopped_reason.as_deref(), Some("interrupted"));
        assert_eq!(tasks.first().state, "Interrupted");
        assert_eq!(tasks.first().reason.as_deref(), Some("interrupted"));
    }

    /// The run has to end before the task does.
    ///
    /// A task recorded as ended while its agent still works is a task nobody is watching.
    #[test]
    fn interrupting_ends_the_run_of_every_task_it_interrupts() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        execution.interrupt().unwrap();

        assert_eq!(
            agent
                .stopped
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_slice(),
            ["1"]
        );
    }

    #[test]
    fn interrupting_with_nothing_running_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        assert_eq!(execution.interrupt(), Err(Refusal::NoSessionRunning));
    }

    /// A session that has run as long as it declared stops, and whatever it still had running ends where it got to.
    #[test]
    fn a_session_out_of_time_stops_and_interrupts_what_was_running() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let late = Frozen(1_000 + 8 * 3_600);
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let on_time = Supervisor::new(outside, AT_ONCE);
        ExecutionService::new(outside, &on_time)
            .run(declaring("2M", "8h"))
            .unwrap();

        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &late,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let assigned = WorkService::new(outside, &supervisor)
            .carry_on("task:1")
            .unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
    }

    /// A reader is told whether more can still arrive.
    ///
    /// The run's state answers that, and the run is this service's to know.
    #[test]
    fn a_trace_says_whether_it_can_still_grow() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![StoredTask {
            session: Some("1".to_owned()),
            state: "Running".to_owned(),
            ..a_pending_task()
        }]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        assert!(!execution.trace("1", None).unwrap().done);

        tasks.stored.lock().unwrap().tasks[0].state = "Completed".to_owned();
        assert!(execution.trace("1", None).unwrap().done);
    }

    #[test]
    fn the_trace_of_a_task_nobody_registered_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(Vec::new());
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);

        assert_eq!(
            execution.trace("7", None),
            Err(Refusal::NoSuchTask {
                id: "task:7".to_owned()
            })
        );
    }

    /// Section 2.2 lists them newest first, which is the order the numbers were handed out in.
    #[test]
    fn sessions_are_listed_newest_first() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        // A budget one task overruns.
        // The first session ends after one, and the second task is left for a session of its own.
        execution.run(declaring("1000", "8h")).unwrap();
        work.carry_on("task:1").unwrap();
        execution.run(declaring("1000", "2h")).unwrap();

        let listed = execution.sessions(None, None).unwrap();
        let ids: Vec<&str> = listed.sessions.iter().map(|one| one.id.as_str()).collect();
        assert_eq!(ids, ["session:2", "session:1"]);
        assert_eq!(listed.page, 1);
        assert_eq!(listed.limit, 20);
    }

    #[test]
    fn a_page_holds_what_it_was_given_room_for() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("1000", "8h")).unwrap();
        work.carry_on("task:1").unwrap();
        execution.run(declaring("1000", "2h")).unwrap();

        let second = execution.sessions(Some("2"), Some("1")).unwrap();
        assert_eq!(second.sessions.len(), 1);
        assert_eq!(second.sessions[0].id, "session:1");
    }

    #[test]
    fn a_session_reports_what_it_declared_beside_what_it_consumed() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let report = execution.session("1").unwrap();
        assert_eq!(report.budget.usage, "2000000");
        assert_eq!(report.budget.time, "8h");
        // The stand-in agent reports the same count for every task.
        assert_eq!(report.consumed.usage, "295816");
        assert_eq!(report.tasks.len(), 2);
        assert_eq!(report.tasks[0].id, "task:1");
        assert_eq!(report.tasks[0].state, "Completed");
        assert_eq!(report.tasks[0].branch.as_deref(), Some("cistern/1"));
    }

    /// Section 2.2 gives this to a session the vendor turned away, and a share reads the window
    /// at every look, so a share that stopped for any other reason has one to report and must
    /// not.
    #[test]
    fn a_share_that_stopped_for_another_reason_says_nothing_about_the_limit_starting_over() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        // One point every time it is asked, against a budget of one point.
        let moving = Advancing {
            used: Mutex::new(0),
            step: 100,
        };
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &moving,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("1%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        // The store holds the window every look recorded.
        assert_eq!(
            sessions.load().sessions[0].resets_at.as_deref(),
            Some("1786285800")
        );

        let report = execution.session("1").unwrap();
        assert_eq!(report.stopped_reason.as_deref(), Some("budget hardlock"));
        assert_eq!(report.resets_at, None);
    }

    #[test]
    fn a_session_the_vendor_turned_away_says_when_the_limit_starts_over() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it stopped".to_owned()),
            observed: spending(),
        });
        let full = AtPercent {
            used: Mutex::new(100 * HUNDREDTHS),
            refuse: Mutex::new(false),
        };
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &full,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let report = execution.session("1").unwrap();
        assert_eq!(report.stopped_reason.as_deref(), Some("vendor limit"));
        assert_eq!(report.resets_at.as_deref(), Some("1786285800"));
    }

    /// The thread waiting on a killed agent comes back to say it failed.
    /// The task has already ended for a better reason.
    #[test]
    fn what_the_agent_says_afterwards_does_not_undo_the_interruption() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it was killed".to_owned()),
            observed: spending(),
        });
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        execution.interrupt().unwrap();
        work.carry_on("task:1").unwrap();

        assert_eq!(tasks.first().state, "Interrupted");
        assert_eq!(tasks.first().reason.as_deref(), Some("interrupted"));
    }
}
