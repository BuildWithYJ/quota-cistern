//! Running a task a session has already assigned.
//!
//! Not a command. A person types `run` and is answered at once; this runs for as long as a
//! task takes, on whatever thread the daemon set aside for it.
//!
//! What to do once a task ends is not decided here. This reports what happened and hands the
//! decision to the supervisor, which answers with what to run next.

use crate::core::{
    domain::{Consumption, Observation, StoppedReason, Task, TaskId, TaskState},
    port::{
        inbound::{Carrying, NotCarried, Refusal},
        outbound::{Cut, Observed, Outcome, Run, Spent, Work},
    },
};

use super::{
    backlog, labelled, sessions,
    supervision::{Outside, Supervisor},
};

/// The reason section 1 gives a task stopped at the ceiling on one run.
///
/// The supervisor reads it back off the ledger: a run that ended this way says where it was
/// stopped rather than what its task takes, which is not a figure to size the next one from.
pub(super) const AT_CEILING: &str = "task ceiling";

/// Carrying tasks on for the sessions the supervisor decides for.
pub struct WorkService<'a> {
    outside: Outside<'a>,
    supervising: &'a Supervisor<'a>,
}

impl<'a> WorkService<'a> {
    pub fn new(outside: Outside<'a>, supervising: &'a Supervisor<'a>) -> Self {
        WorkService {
            outside,
            supervising,
        }
    }
}

impl Carrying for WorkService<'_> {
    fn carry_on(&self, task: &str) -> Result<Vec<String>, NotCarried> {
        self.carrying(task).map_err(not_carried)
    }
}

impl WorkService<'_> {
    /// Puts one run in the ledger.
    ///
    /// A task that is no longer there leaves nothing to write down, which is not a failure:
    /// the run happened and the task it belonged to was removed while it did.
    fn remember(&self, run: Option<Run>) -> Result<(), Refusal> {
        match run {
            Some(run) => Ok(self.outside.runs.append(run)?),
            None => Ok(()),
        }
    }

    /// Everything `carry_on` does, in the words the rest of this file speaks.
    fn carrying(&self, task: &str) -> Result<Vec<String>, Refusal> {
        let id = TaskId::parse(task).ok_or_else(|| Refusal::BadValue {
            key: "task".to_owned(),
            value: task.to_owned(),
        })?;

        let (repository, base, branch, instruction, model, ceiling, usage) = {
            let tasks = backlog::read(self.outside.tasks)?;
            let held = tasks
                .find(id)
                .ok_or_else(|| Refusal::NoSuchTask { id: id.labelled() })?;
            // A task queued before its session stopped is still on the queue when a worker
            // reaches it. Starting it would spend against a session that has already reported
            // what it spent, and nothing would stop the run afterwards.
            if held.state() != TaskState::Running {
                return Ok(Vec::new());
            }
            (
                held.repository().to_string(),
                held.base_branch(),
                held.result_branch().unwrap_or_default(),
                held.instruction().to_owned(),
                held.model().map(str::to_owned),
                held.ceiling(),
                held.session(),
            )
        };

        // What the decision allowed this run, said in the unit the vendor prices runs at.
        let ceiling = match (
            ceiling,
            usage.map(|id| self.supervising.declared(id)).transpose()?,
        ) {
            (Some(ceiling), Some(Some(usage))) => self.supervising.priced(usage, ceiling)?,
            _ => None,
        }
        .map(|priced| priced.to_string());

        let at = match self.outside.worktrees.prepare(Cut {
            repository: &repository,
            base: &base,
            branch: &branch,
            task: &id.to_string(),
        }) {
            Ok(at) => at,
            // A task with nowhere to work has ended, and nothing ran, so there is nothing to have consumed.
            Err(e) => return self.ended(id, TaskState::Error, Some(e.reason), Observation::NotYet),
        };
        backlog::change(self.outside.tasks, |tasks| {
            tasks.work_area(id, at.clone());
            Ok(())
        })?;

        let trace = self.outside.traces.keeping(&id.to_string())?;
        let ended = self.outside.agent.work(Work {
            task: &id.to_string(),
            at: &at,
            trace,
            instruction: &instruction,
            model: model.as_deref(),
            ceiling: ceiling.as_deref(),
        });
        match ended {
            Ok(ended) => {
                let consumed = observed(ended.observed);
                match ended.outcome {
                    Outcome::Finished => self.ended(id, TaskState::Completed, None, consumed),
                    // Section 1 gives a run stopped at its ceiling a reason of its own.
                    // It also says the session carries on.
                    Outcome::AtCeiling => self.ended(
                        id,
                        TaskState::Interrupted,
                        Some(AT_CEILING.to_owned()),
                        consumed,
                    ),
                    // Only the vendor's limit tells a run it would not take from one that went wrong.
                    Outcome::Failed => match self.supervising.at_its_limit() {
                        true => self.turned_away(id, consumed),
                        false => self.ended(id, TaskState::Error, ended.reason, consumed),
                    },
                }
            }
            Err(e) => self.ended(id, TaskState::Error, Some(e.reason), Observation::NotYet),
        }
    }

    /// A task the vendor would not run, and the session it belonged to.
    ///
    /// The task goes back to waiting, since nothing about it was wrong.
    /// It is the vendor that has to change its mind.
    /// The session stops, because every other task in it would be turned away the same way.
    fn turned_away(&self, id: TaskId, consumed: Observation) -> Result<Vec<String>, Refusal> {
        let starts_over = self
            .outside
            .limit
            .read()
            .ok()
            .and_then(|at| at.resets_at.parse().ok());
        let now = self.outside.clock.now();
        let (session, run) = backlog::change(self.outside.tasks, |tasks| {
            tasks.record(id, consumed.clone());
            let held = tasks.find(id);
            let session = held.and_then(Task::session);
            // The vendor turned this run away, so the session stops rather than deciding
            // again, and the reading it is holding is the last there will be.
            let run = held.map(|held| ran(held, now, (None, None)));
            tasks.wait_again(id, now);
            Ok((session, run))
        })?;
        self.remember(run)?;

        if let Some(session) = session {
            if let Some(at) = starts_over {
                sessions::change(self.outside.sessions, |sessions| {
                    sessions.resets_at(session, at);
                    Ok(())
                })?;
            }
            self.supervising.stop(session, StoppedReason::VendorLimit)?;
        }
        Ok(Vec::new())
    }

    /// Moves a task to the state it ended in, records what it consumed, and decides what happens next.
    ///
    /// The first two are one change, so that a task is never stored as ended with what it consumed still missing.
    fn ended(
        &self,
        id: TaskId,
        state: TaskState,
        reason: Option<String>,
        consumed: Observation,
    ) -> Result<Vec<String>, Refusal> {
        let now = self.outside.clock.now();
        let (session, ended) = backlog::change(self.outside.tasks, |tasks| {
            tasks.finish(id, state, reason.clone(), now);
            tasks.record(id, consumed.clone());
            let held = tasks.find(id);
            Ok((held.and_then(Task::session), held.cloned()))
        })?;

        let Some(session) = session else {
            self.remember(ended.as_ref().map(|held| ran(held, now, (None, None))))?;
            return Ok(Vec::new());
        };

        // Measured, written down, and only then decided. The reading the session is holding
        // is the one it took when the run before this ended, which is where this run started
        // from; measuring takes the next one, and the two of them are what this run cost in
        // the unit a share is declared in.
        //
        // The order is what `docs/cli.md` promises: each task's own cost is what decides.
        // Deciding first would decide from the run before this one.
        let before = self.supervising.limit_last_seen(session)?;
        let read = self.supervising.measured(session)?;
        let after = self.supervising.limit_last_seen(session)?;
        self.remember(ended.as_ref().map(|held| ran(held, now, (before, after))))?;

        self.supervising.settle(session, read).map(labelled)
    }
}

/// Reads what the agent said it consumed.
///
/// The port answers in the core's own words already, so this only tells the two answers apart.
/// A count the adapter could not read is not a count of nothing.
/// Section 1 keeps the two apart as far as the reason a session stops.
fn observed(observed: Observed) -> Observation {
    match observed {
        Observed::Unreadable { why } => Observation::Unreadable { why },
        Observed::Spent(spent) => match counted(&spent) {
            Some(counted) => Observation::Spent(counted),
            None => Observation::Unreadable {
                why: "what the agent counted does not read as a number".to_owned(),
            },
        },
    }
}

/// A count as the port hands it over, if every figure in it is one.
fn counted(spent: &Spent) -> Option<Consumption> {
    Some(Consumption {
        input: spent.input.parse().ok()?,
        output: spent.output.parse().ok()?,
        cache_written: spent.cache_written.parse().ok()?,
        cache_read: spent.cache_read.parse().ok()?,
        cost: spent.cost.parse().ok()?,
    })
}

/// A refusal nobody asked for, as what it is to a worker.
fn not_carried(why: Refusal) -> NotCarried {
    match why {
        Refusal::NoSuchTask { id } => NotCarried::NoSuchTask { id },
        Refusal::Unavailable { reason } => NotCarried::Unavailable { reason },
        // Nothing else can reach here: the task was named by a session of this core.
        other => NotCarried::Unavailable {
            reason: format!("{other:?}"),
        },
    }
}

/// One run of one task, as the ledger holds it.
///
/// Taken from the task while the store is held, so the figures are the ones the run left rather
/// than the ones a later run put there.
///
/// `over` is how far the vendor's limit was spent before the run and after it. Both are
/// readings the session already took, so they are handed in rather than asked for.
fn ran(held: &Task, now: u64, over: (Option<u64>, Option<u64>)) -> Run {
    Run {
        task: held.id().to_string(),
        session: held.session().map(|session| session.to_string()),
        model: held.model().map(str::to_owned),
        started_at: held.started_at().unwrap_or(now).to_string(),
        ended_at: held.ended_at().unwrap_or(now).to_string(),
        outcome: held.state().to_string(),
        reason: held.reason().map(str::to_owned),
        spent: backlog::kept(held.consumed()),
        unreadable: match held.consumed() {
            Observation::Unreadable { why } => Some(why.clone()),
            _ => None,
        },
        limit_before: over.0.map(|at| at.to_string()),
        limit_after: over.1.map(|at| at.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::core::{
        domain::{HUNDREDTHS, SessionId},
        port::{
            inbound::{Carrying, ExecutionUseCase},
            outbound::{BacklogStore, Ended, Observed, Outcome, Spent},
        },
    };

    use super::super::fixtures::*;
    use super::super::{ExecutionService, Outside, Supervisor, WorkService};
    use super::*;

    #[test]
    fn a_task_that_was_carried_on_ends_completed_where_it_was_worked_on() {
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
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        assert_eq!(
            areas.cut.lock().unwrap().as_slice(),
            [(
                "/work/api".to_owned(),
                "main".to_owned(),
                "cistern/1".to_owned()
            )]
        );
        assert_eq!(
            agent.asked.lock().unwrap().as_slice(),
            [("/areas/1".to_owned(), "tidy up src/utils".to_owned(), None)]
        );

        let held = tasks.first();
        assert_eq!(held.state, "Completed");
        assert_eq!(held.worktree.as_deref(), Some("/areas/1"));
        assert_eq!(held.reason, None);
    }

    #[test]
    fn a_task_that_ran_is_stored_with_what_it_consumed() {
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
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let counted = tasks.first().consumed.unwrap();
        assert_eq!(counted.input, "77");
        assert_eq!(counted.output, "3377");
        assert_eq!(counted.cache_written, "28879");
        assert_eq!(counted.cache_read, "263483");
        assert_eq!(counted.cost, "92170");
        assert_eq!(tasks.first().unreadable, None);

        // The backlog held one task and it is done.
        // Nothing is left to assign, and the session has nothing more to do.
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("all done"));
    }

    /// A task that never reached the agent has not consumed nothing; it has not consumed at all.
    /// Neither field says otherwise.
    #[test]
    fn a_task_that_never_ran_is_stored_with_no_count_at_all() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas {
            refuse: true,
            ..Areas::default()
        };
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

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.consumed, None);
        assert_eq!(held.unreadable, None);
    }

    /// The agent answered with a count, and one figure in it is not a number.
    #[test]
    fn a_figure_that_does_not_read_as_a_number_is_not_a_count() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Finished,
            reason: None,
            observed: Observed::Spent(Spent {
                input: "a lot".to_owned(),
                output: "3377".to_owned(),
                cache_written: "28879".to_owned(),
                cache_read: "263483".to_owned(),
                cost: "92170".to_owned(),
            }),
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

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        assert_eq!(tasks.first().consumed, None);
        assert!(tasks.first().unreadable.is_some());
    }

    #[test]
    fn an_agent_that_failed_leaves_the_task_in_error_with_what_it_said() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it went wrong".to_owned()),
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

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("it went wrong"));
    }

    /// Section 1 says the session carries on when one task hits the ceiling on a single run.
    #[test]
    fn a_task_stopped_at_its_ceiling_says_so_and_the_session_carries_on() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::AtCeiling,
            reason: Some("the agent was cut off after 200 turns".to_owned()),
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
        work.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Interrupted");
        assert_eq!(held.reason.as_deref(), Some("task ceiling"));
        assert_eq!(sessions.load().sessions[0].state, "running");
    }

    /// A vendor that will not run one task will not run the next either, and nothing about the task was wrong.
    #[test]
    fn a_task_the_vendor_would_not_run_waits_again_and_the_session_stops() {
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

        let held = tasks.first();
        assert_eq!(held.state, "Pending");
        assert_eq!(held.reason, None);
        // The session it was assigned to stays. It paid for whatever the refused run got
        // through, and assigning the task again is what names the next one.
        assert_eq!(held.session.as_deref(), Some("1"));

        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("vendor limit"));
    }

    /// A run can fail on its own account, and the vendor having room left is what says so.
    #[test]
    fn a_task_that_failed_with_room_left_is_an_error() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it went wrong".to_owned()),
            observed: spending(),
        });
        let room = AtPercent {
            used: Mutex::new(40 * HUNDREDTHS),
            refuse: Mutex::new(false),
        };
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &room,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("it went wrong"));
        assert_eq!(sessions.load().sessions[0].state, "running");
    }

    /// A reading nobody could take is not a limit that has been reached.
    #[test]
    fn a_task_that_failed_with_no_reading_to_be_had_is_an_error() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it went wrong".to_owned()),
            observed: spending(),
        });
        let silent = AtPercent {
            used: Mutex::new(0),
            refuse: Mutex::new(true),
        };
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &silent,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        assert_eq!(tasks.first().state, "Error");
    }

    /// The executor is called for one task at a time and several at once.
    /// Two running together have to end in their own places without either losing what the other recorded.
    #[test]
    fn two_tasks_carried_on_at_once_each_end_in_their_own_place() {
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
        let work = WorkService::new(outside, &supervisor);

        // The supervisor is what assigns more than one.
        // Until it exists, a test stands in for it by assigning the second task itself.
        execution.run(declaring("50%", "8h")).unwrap();
        let opened = SessionId::parse("1").unwrap();
        backlog::change(&tasks, |held| {
            let waiting = held.next_to_assign().unwrap();
            Ok(held.assign(waiting, opened, u64::MAX, 0))
        })
        .unwrap();

        let work = &work;
        std::thread::scope(|threads| {
            for task in ["task:1", "task:2"] {
                threads.spawn(move || work.carry_on(task).unwrap());
            }
        });

        let held = tasks.load().unwrap().tasks;
        assert_eq!(held[0].state, "Completed");
        assert_eq!(held[1].state, "Completed");
        assert_eq!(held[0].worktree.as_deref(), Some("/areas/1"));
        assert_eq!(held[1].worktree.as_deref(), Some("/areas/2"));
    }

    /// A task with nowhere to work has ended, and saying so is what keeps it from being read as still running.
    #[test]
    fn a_work_area_that_could_not_be_made_leaves_the_task_in_error() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas {
            refuse: true,
            ..Areas::default()
        };
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

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("no such base branch"));
        assert!(agent.asked.lock().unwrap().is_empty());
    }

    /// The backlog keeps one run to a task, so what a budget is worked out from is kept apart.
    #[test]
    fn a_run_that_ended_is_written_to_the_ledger() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let ledger = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &ledger,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let written = ledger.runs();
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(written[0].task, "1");
        assert_eq!(written[0].session.as_deref(), Some("1"));
        assert_eq!(written[0].outcome, "Completed");
        assert_eq!(
            written[0].spent.as_ref().map(|spent| spent.cost.as_str()),
            Some("92170")
        );
        assert_eq!(written[0].unreadable, None);
    }

    /// A refused run is still a run, and the session it was assigned to paid for it.
    #[test]
    fn a_run_the_vendor_refused_is_written_to_the_ledger_with_its_session() {
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
        let ledger = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &full,
            traces: &NOTHING_KEPT,
            runs: &ledger,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let written = ledger.runs();
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(written[0].task, "1");
        assert_eq!(written[0].session.as_deref(), Some("1"));
        assert_eq!(
            written[0].spent.as_ref().map(|spent| spent.cost.as_str()),
            Some("92170")
        );
    }

    /// What a run consumed may be unreadable, and that is not the same as nothing.
    #[test]
    fn a_run_nobody_could_read_is_written_to_the_ledger_as_unreadable() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Finished,
            reason: None,
            observed: Observed::Unreadable {
                why: "no last line".to_owned(),
            },
        });
        let ledger = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &ledger,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let written = ledger.runs();
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(written[0].spent, None);
        assert_eq!(written[0].unreadable.as_deref(), Some("no last line"));
    }

    /// A task is put on the queue when it is assigned and reached when a worker gets to it.
    ///
    /// The session can stop in between. Starting the task then spends against a session that
    /// has already reported what it spent, and nothing would end the run afterwards.
    #[test]
    fn a_task_whose_session_stopped_before_a_worker_reached_it_does_not_start() {
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
        // Everything the run assigned is interrupted, which is what a session stopping does.
        execution.interrupt().unwrap();
        let asked = agent.asked.lock().unwrap().len();

        let assigned = work.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        assert_eq!(
            agent.asked.lock().unwrap().len(),
            asked,
            "a task was started for a session that had stopped"
        );
        assert!(
            runs.runs().is_empty(),
            "a run was written for a task that never started"
        );
    }

    /// What a run cost in the unit a share is declared in.
    ///
    /// Tokens say what it cost in the other unit, and neither converts to the other on its
    /// own. The two readings are ones the session already took, so they are written down
    /// rather than asked for again.
    #[test]
    fn a_run_of_a_share_records_where_the_limit_stood_either_side_of_it() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        // Opens at 1000, and each look after that is 300 further on.
        let climbing = Advancing {
            used: Mutex::new(1_000),
            step: 300,
        };
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &climbing,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);

        ExecutionService::new(outside, &supervisor)
            .run(declaring("50%", "8h"))
            .unwrap();
        WorkService::new(outside, &supervisor)
            .carry_on("task:1")
            .unwrap();

        let written = runs.runs();
        assert_eq!(written.len(), 1, "{written:?}");
        let (before, after) = (
            written[0].limit_before.as_deref(),
            written[0].limit_after.as_deref(),
        );
        assert!(before.is_some() && after.is_some(), "{written:?}");
        assert_ne!(before, after, "the run cost nothing the limit could see");
    }

    /// A session declared in tokens never asks how far the limit is spent, so a run of one has
    /// no reading either side of it.
    #[test]
    fn a_run_of_a_count_records_no_reading_at_all() {
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

        ExecutionService::new(outside, &supervisor)
            .run(declaring("2M", "8h"))
            .unwrap();
        WorkService::new(outside, &supervisor)
            .carry_on("task:1")
            .unwrap();

        let written = runs.runs();
        assert_eq!(written[0].limit_before, None, "{written:?}");
        assert_eq!(written[0].limit_after, None, "{written:?}");
    }
}
