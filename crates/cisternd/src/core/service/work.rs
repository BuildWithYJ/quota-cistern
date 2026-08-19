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
        outbound::{Cut, Observed, Outcome, Spent, Work},
    },
};

use super::{backlog, labelled, sessions, supervision::Supervisor};

/// The reason section 1 gives a task stopped at the ceiling on one run.
const AT_CEILING: &str = "task ceiling";

/// Carrying tasks on for the sessions the supervisor decides for.
pub struct WorkService<'a> {
    supervising: &'a Supervisor<'a>,
}

impl<'a> WorkService<'a> {
    pub fn new(supervising: &'a Supervisor<'a>) -> Self {
        WorkService { supervising }
    }
}

impl Carrying for WorkService<'_> {
    fn carry_on(&self, task: &str) -> Result<Vec<String>, NotCarried> {
        self.carrying(task).map_err(not_carried)
    }
}

impl WorkService<'_> {
    /// Everything `carry_on` does, in the words the rest of this file speaks.
    fn carrying(&self, task: &str) -> Result<Vec<String>, Refusal> {
        let id = TaskId::parse(task).ok_or_else(|| Refusal::BadValue {
            key: "task".to_owned(),
            value: task.to_owned(),
        })?;

        let (repository, base, branch, instruction, model) = {
            let tasks = backlog::read(self.supervising.outside.tasks)?;
            let held = tasks
                .find(id)
                .ok_or_else(|| Refusal::NoSuchTask { id: id.labelled() })?;
            (
                held.repository().to_string(),
                held.base_branch(),
                held.result_branch().unwrap_or_default(),
                held.instruction().to_owned(),
                held.model().map(str::to_owned),
            )
        };

        let at = match self.supervising.outside.worktrees.prepare(Cut {
            repository: &repository,
            base: &base,
            branch: &branch,
            task: &id.to_string(),
        }) {
            Ok(at) => at,
            // A task with nowhere to work has ended, and nothing ran, so there is nothing to have consumed.
            Err(e) => return self.ended(id, TaskState::Error, Some(e.reason), Observation::NotYet),
        };
        backlog::change(self.supervising.outside.tasks, |tasks| {
            tasks.work_area(id, at.clone());
            Ok(())
        })?;

        let trace = self.supervising.outside.traces.keeping(&id.to_string())?;
        let ended = self.supervising.outside.agent.work(Work {
            task: &id.to_string(),
            at: &at,
            trace,
            instruction: &instruction,
            model: model.as_deref(),
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
            .supervising
            .outside
            .limit
            .read()
            .ok()
            .and_then(|at| at.resets_at.parse().ok());
        let now = self.supervising.outside.clock.now();
        let session = backlog::change(self.supervising.outside.tasks, |tasks| {
            tasks.record(id, consumed.clone());
            let session = tasks.find(id).and_then(Task::session);
            tasks.wait_again(id, now);
            Ok(session)
        })?;

        if let Some(session) = session {
            if let Some(at) = starts_over {
                sessions::change(self.supervising.outside.sessions, |sessions| {
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
        let now = self.supervising.outside.clock.now();
        let session = backlog::change(self.supervising.outside.tasks, |tasks| {
            tasks.finish(id, state, reason.clone(), now);
            tasks.record(id, consumed.clone());
            Ok(tasks.find(id).and_then(Task::session))
        })?;

        let Some(session) = session else {
            return Ok(Vec::new());
        };
        self.supervising.settle(session).map(labelled)
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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &full,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Pending");
        assert_eq!(held.session, None);
        assert_eq!(held.reason, None);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &room,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &silent,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

        // The supervisor is what assigns more than one.
        // Until it exists, a test stands in for it by assigning the second task itself.
        execution.run(declaring("50%", "8h")).unwrap();
        let opened = SessionId::parse("1").unwrap();
        backlog::change(&tasks, |held| Ok(held.assign(opened, 0))).unwrap();

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
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let execution = ExecutionService::new(&supervisor);
        let work = WorkService::new(&supervisor);

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("no such base branch"));
        assert!(agent.asked.lock().unwrap().is_empty());
    }
}
