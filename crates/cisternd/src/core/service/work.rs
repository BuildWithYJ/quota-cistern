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

/// How a run came to an end, in the two places that have to hear it.
///
/// A person is told one word for a ceiling whatever ceiling it was, since that is the state
/// `docs/cli.md` gives the task. The ledger is told what the vendor said, since a figure
/// worked out from runs has to know which ceiling stopped each of them.
#[derive(Debug, Clone, Default)]
struct Ended {
    /// What the task is left with, which a person reads.
    told: Option<String>,
    /// What the vendor said, which the ledger keeps.
    said: Option<String>,
    /// The conversation the run was in, which the task keeps so a later run may carry it on.
    conversation: Option<String>,
}

impl Ended {
    /// A run that ended without either being said.
    fn nothing() -> Self {
        Ended::default()
    }

    /// A run whose own words are what both are told.
    fn of(said: Option<String>) -> Self {
        Ended {
            told: said.clone(),
            said,
            conversation: None,
        }
    }

    /// The same, in a conversation a later run may carry on.
    fn conversing(mut self, conversation: Option<String>) -> Self {
        self.conversation = conversation;
        self
    }
}

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

        let (repository, base, branch, instruction, model, conversation) = {
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
                held.conversation().map(str::to_owned),
            )
        };

        let at = match self.outside.worktrees.prepare(Cut {
            repository: &repository,
            base: &base,
            branch: &branch,
            task: &id.to_string(),
        }) {
            Ok(at) => at,
            // A task with nowhere to work has ended, and nothing ran, so there is nothing to have consumed.
            Err(e) => {
                return self.ended(
                    id,
                    TaskState::Error,
                    Ended::of(Some(e.reason)),
                    Observation::NotYet,
                );
            }
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
            conversation: conversation.as_deref(),
        });
        match ended {
            Ok(ended) => {
                let consumed = observed(ended.observed);
                match ended.outcome {
                    // A task that finished has nothing left to say, so no conversation is
                    // kept for it.
                    Outcome::Finished => {
                        self.ended(id, TaskState::Completed, Ended::nothing(), consumed)
                    }
                    // Section 1 gives a run stopped at its ceiling a reason of its own.
                    // It also says the session carries on.
                    //
                    // One word for the task, whatever ceiling it was, since that is what a
                    // person is told. The vendor's own sentence goes to the ledger beside it:
                    // a run held back by its turns and a run held back by what it may spend
                    // say different things about the task, and one word for both loses that.
                    Outcome::AtCeiling => self.ended(
                        id,
                        TaskState::Interrupted,
                        Ended {
                            told: Some(AT_CEILING.to_owned()),
                            said: ended.reason,
                            conversation: ended.conversation,
                        },
                        consumed,
                    ),
                    // Only the vendor's limit tells a run it would not take from one that went wrong.
                    Outcome::Failed => match self.supervising.at_its_limit() {
                        true => self.turned_away(id, consumed),
                        false => self.ended(
                            id,
                            TaskState::Error,
                            Ended::of(ended.reason).conversing(ended.conversation),
                            consumed,
                        ),
                    },
                }
            }
            Err(e) => self.ended(
                id,
                TaskState::Error,
                Ended::of(Some(e.reason)),
                Observation::NotYet,
            ),
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
            // Nothing was said about how it ended: the vendor would not take it, which is
            // the session's state rather than anything about this task.
            let run = held.map(|held| ran(held, &Ended::nothing(), now, (None, None)));
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
        why: Ended,
        consumed: Observation,
    ) -> Result<Vec<String>, Refusal> {
        let now = self.outside.clock.now();
        let (session, ended) = backlog::change(self.outside.tasks, |tasks| {
            tasks.finish(id, state, why.told.clone(), now);
            tasks.record(id, consumed.clone());
            tasks.conversed(id, why.conversation.clone());
            let held = tasks.find(id);
            Ok((held.and_then(Task::session), held.cloned()))
        })?;

        let Some(session) = session else {
            self.remember(
                ended
                    .as_ref()
                    .map(|held| ran(held, &why, now, (None, None))),
            )?;
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
        self.remember(
            ended
                .as_ref()
                .map(|held| ran(held, &why, now, (before, after))),
        )?;

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
fn ran(held: &Task, why: &Ended, now: u64, over: (Option<u64>, Option<u64>)) -> Run {
    Run {
        task: held.id().to_string(),
        session: held.session().map(|session| session.to_string()),
        model: held.model().map(str::to_owned),
        started_at: held.started_at().unwrap_or(now).to_string(),
        ended_at: held.ended_at().unwrap_or(now).to_string(),
        outcome: held.state().to_string(),
        reason: held.reason().map(str::to_owned),
        said: why.said.clone(),
        spent: backlog::kept(held.consumed()),
        unreadable: match held.consumed() {
            Observation::Unreadable { why } => Some(why.clone()),
            _ => None,
        },
        ceiling: held.ceiling().map(|at| at.to_string()),
        limit_before: over.0.map(|at| at.to_string()),
        limit_after: over.1.map(|at| at.to_string()),
    }
}

#[cfg(test)]
mod tests;
