//! The judgement a session is run under.
//!
//! Whether a session carries on and with what is one decision, and this is where it is made.
//! `agent.rs` says a run's ceiling is the supervisor's and `consumption.rs` says what counts
//! towards a budget is the supervisor's; this is that role, in one place rather than in the
//! margins of the commands that happen to ask for it.
//!
//! It answers no command and implements no inbound port. A person types `run` and `interrupt`,
//! and the daemon's own workers carry tasks on; both reach a decision through here.

use crate::core::{
    domain::{
        Backlog, Consumption, Decision, HUNDREDTHS, Observation, Ran, Session, SessionId,
        SessionState, Sizings, Spending, Standing, StoppedReason, TaskId, TaskState, Usage, decide,
    },
    port::{
        inbound::Refusal,
        outbound::{
            Agent, BacklogStore, Clock, Limit, Run, Runs, SessionStore, StoredConsumption, Traces,
            Worktrees,
        },
    },
};

use super::{backlog, sessions, work::AT_CEILING};

/// What running a session needs from outside.
///
/// One value rather than an argument each, so that a port added later is a line here and a
/// line where the daemon is built, and nothing in between.
///
/// Copied rather than shared, since it holds nothing but references. Each service that needs
/// the outside takes its own, so that none of them reaches the outside through another.
#[derive(Clone, Copy)]
pub struct Outside<'a> {
    pub sessions: &'a dyn SessionStore,
    pub tasks: &'a dyn BacklogStore,
    pub worktrees: &'a dyn Worktrees,
    pub agent: &'a dyn Agent,
    pub clock: &'a dyn Clock,
    pub limit: &'a dyn Limit,
    pub traces: &'a dyn Traces,
    pub runs: &'a dyn Runs,
}

/// The reading at which the vendor has nothing left to give.
const FULL: u64 = 100 * HUNDREDTHS;

/// Who decides what a session does next.
///
/// The commands and the workers each hold their own ports and ask this one for a decision.
/// Nothing reaches the outside through here: a service that did would be borrowing a role's
/// state rather than holding its own.
///
/// What may be asked is `settle`, `stop`, `spending_of`, `limit_now`, and `at_its_limit`.
/// Everything else is this one's own.
pub struct Supervisor<'a> {
    outside: Outside<'a>,
    /// The most tasks this machine has hands for.
    ///
    /// A guard on the machine rather than on the budget, so the number belongs to whoever
    /// started the daemon rather than to a rule about spending.
    at_once: usize,
}

impl<'a> Supervisor<'a> {
    pub fn new(outside: Outside<'a>, at_once: usize) -> Self {
        Supervisor { outside, at_once }
    }
}

impl Supervisor<'_> {
    /// How far the vendor's limit is spent, and when the window it counts in begins again.
    ///
    /// Only a session declared as a share asks, since only a share is measured against it and
    /// asking costs something.
    ///
    /// A window that cannot be read is nothing rather than a failure. It tells one window from
    /// the next and the figure is still worth having without it.
    pub(super) fn limit_now(&self) -> Result<(u64, Option<u64>), Refusal> {
        let reading = self.outside.limit.read()?;
        let used = reading
            .used
            .parse()
            .map_err(|_| sessions::unreadable("used", &reading.used))?;
        Ok((used, reading.resets_at.parse().ok()))
    }

    /// Whether the vendor has nothing left to give.
    ///
    /// A reading this cannot take is not a limit that has been reached.
    /// The run failed either way.
    /// Calling it the vendor's doing on a question nobody could answer would stop a session that had room left.
    pub(super) fn at_its_limit(&self) -> bool {
        self.limit_now().is_ok_and(|(used, _)| used >= FULL)
    }

    /// Asks the vendor how far the session has spent, and writes what it said down.
    ///
    /// Apart from deciding, because a run that just ended has to reach the ledger before the
    /// decision reads it: `docs/cli.md` says each task's own cost is what decides, and a
    /// decision made before the ledger has that run decides from the one before it.
    ///
    /// Nothing where the session is no longer one this decides for, and nothing where a share
    /// can no longer be read -- section 1 stops a session in that state, and the caller has the
    /// reading either side of it to write down first.
    pub(super) fn measured(&self, session: SessionId) -> Result<Option<Spending>, Refusal> {
        let Some(held) = self.held(session)? else {
            return Ok(None);
        };
        match held.budget().usage {
            // A share is read from the vendor, which is outside and slow.
            Usage::Share(_) => self.spending(&held),
            // A count is the backlog's own sum, taken under the hold the decision is made
            // under rather than here. Another task ending in between would leave that decision
            // made from a budget that has since gone down.
            Usage::Tokens(_) => Ok(Some(Spending::Tokens(0))),
        }
    }

    /// One decision: whether the session carries on, and with what.
    ///
    /// Section 2.2 says assignment is dynamic and this is the whole of it.
    /// When this is called is the composition root's; what it decides is here.
    ///
    /// `read` is what `measured` said, which the caller took first so that the run that just
    /// ended is in the ledger by the time this reads it.
    pub(super) fn settle(
        &self,
        session: SessionId,
        read: Option<Spending>,
    ) -> Result<Vec<TaskId>, Refusal> {
        let Some(held) = self.held(session)? else {
            return Ok(Vec::new());
        };

        let read = match (held.budget().usage, read) {
            (Usage::Share(_), None) => {
                return self
                    .stop(session, StoppedReason::ObservationUnreadable)
                    .map(|()| Vec::new());
            }
            (Usage::Share(_), spent) => spent,
            (Usage::Tokens(_), _) => None,
        };

        // What runs have cost, in the unit this session declared, and how long they took. File
        // reads rather than vendor ones, and taken before the hold like the vendor reading was.
        let sizings = self.sizings(held.budget().usage)?;
        let lasting = self.lasting()?;

        let now = self.outside.clock.now();
        let settled = backlog::change(self.outside.tasks, |tasks| {
            let spent = match read {
                Some(spent) => spent,
                None => Spending::Tokens(Consumption::total(tasks.counted_in(session)).tokens()),
            };
            Ok(
                match decide(&standing(
                    tasks,
                    &held,
                    spent,
                    &sizings,
                    &lasting,
                    self.at_once,
                    now,
                )) {
                    // Stopping takes this store again and the sessions store with it, so it happens
                    // once this hold is given up rather than under it.
                    Decision::Stop(why) => Settled::Stop(spent, why),
                    Decision::Start(allowed) => Settled::Started(
                        spent,
                        allowed
                            .iter()
                            .filter_map(|allowance| {
                                tasks.assign(allowance.task, session, allowance.ceiling, now)
                            })
                            .collect(),
                    ),
                },
            )
        })?;

        // A share cannot be worked out again once the session has stopped.
        // What was read here is what is reported for it afterwards.
        //
        // A store that would not take it does not undo the assignment. The tasks are already
        // written down as running and their numbers are what the caller starts them by, so
        // dropping the numbers here leaves tasks nobody picks up and nothing to pick them up
        // with. The figure is written again at the next task to end, and the assignment is
        // not: one of the two comes back by itself.
        let spent = settled.spent();
        let recorded = sessions::change(self.outside.sessions, |sessions| {
            sessions.record(session, spent, now);
            Ok(())
        });

        match settled {
            // Stopping writes to the same store, so a store that would not take the figure
            // will not take the stopping either. That one is reported.
            Settled::Stop(_, why) => {
                recorded.and_then(|()| self.stop(session, why).map(|()| Vec::new()))
            }
            Settled::Started(_, assigned) => Ok(assigned),
        }
    }

    /// Stops the session and ends whatever it still had running.
    ///
    /// Marking a task interrupted does not end the run behind it. The run is a process this
    /// core started, and a session that stopped while one was still going would go on spending
    /// against a budget it had already reported as spent.
    pub(super) fn stop(&self, session: SessionId, why: StoppedReason) -> Result<(), Refusal> {
        let now = self.outside.clock.now();
        for task in backlog::read(self.outside.tasks)?.taken_by(session) {
            if task.state() == TaskState::Running {
                self.outside.agent.stop(&task.id().to_string());
            }
        }
        sessions::change(self.outside.sessions, |sessions| {
            sessions.stop(session, why, now);
            Ok(())
        })?;
        backlog::change(self.outside.tasks, |tasks| {
            Ok(!tasks.interrupt(session, &why.to_string(), now).is_empty())
        })
        .map(|_: bool| ())
    }

    /// The same, for a session named only by its number.
    /// What runs have cost, by the model that ran them, in the unit a session declared.
    ///
    /// A share and a count are different questions of the same ledger. A run's tokens are
    /// there whatever its session declared; how far it moved the vendor's limit is there only
    /// for a session that was watching that limit, since only those read it.
    ///
    /// Runs that say nothing in the unit asked for are left out rather than counted as zero.
    /// A figure worked out from runs that reported nothing is a figure about nothing.
    /// What a run of each model has cost, from the ledger.
    ///
    /// A run that finished says what its task takes. A run stopped at its ceiling says where it
    /// was stopped, which the sizing holds as a floor rather than counting as a measure. Runs
    /// that ended any other way are left out: a run the vendor turned away or that failed on its
    /// own spent what it spent before it went wrong, which is neither.
    fn sizings(&self, usage: Usage) -> Result<Sizings, Refusal> {
        let held = self.outside.runs.read()?;
        // How far the vendor's limit moves for a millionth of what it prices a run at, over
        // every run that reported both. Nothing here is in the unit a share is declared in
        // until a run has said both, which is where a session declared as a share starts.
        let per_millionth = match usage {
            Usage::Share(_) => match moved_per_millionth(&held)? {
                // No run has said both yet, so nothing here can be put in the unit this
                // session declared. A figure of nothing would size every run at nothing and
                // allow every run almost nothing, so there is no figure at all: one task
                // starts with what is left and is measured, as at the beginning.
                (0, _) | (_, 0) => return Ok(Sizings::default()),
                both => Some(both),
            },
            Usage::Tokens(_) => None,
        };
        Ok(Sizings::of(held.into_iter().filter_map(|run| {
            let ran = sampled(&run)?;
            let counted = spending_of(&run.spent?)?;
            let cost = match per_millionth {
                None => counted.tokens(),
                Some((moved, over)) => counted.cost.checked_mul(moved)? / over.max(1),
            };
            Some(ran(run.model.as_deref(), cost))
        })))
    }

    /// How long a run of each model has taken, from the ledger, in seconds.
    ///
    /// Told apart the same way as what runs cost: a run stopped part way through took less
    /// time than its task needs, for the same reason it spent less.
    fn lasting(&self) -> Result<Sizings, Refusal> {
        let held = self.outside.runs.read()?;
        Ok(Sizings::of(held.into_iter().filter_map(|run| {
            let ran = sampled(&run)?;
            let started = run.started_at.parse::<u64>().ok()?;
            let ended = run.ended_at.parse::<u64>().ok()?;
            Some(ran(run.model.as_deref(), ended.checked_sub(started)?))
        })))
    }

    /// How long the running session still has, or nothing where none is running.
    ///
    /// A store read. Whoever waits on it is not asking the vendor anything.
    pub fn time_left(&self) -> Result<Option<u64>, Refusal> {
        let now = self.outside.clock.now();
        Ok(sessions::read(self.outside.sessions)?
            .running()
            .map(|held| held.time_left(now)))
    }

    /// Stops the running session where it has had the time it declared.
    ///
    /// The other half of the budget. A usage ceiling bounds what a run spends, not how long it
    /// takes, and a decision is only reached when a task ends, so a session with one long run
    /// going would pass the time it declared and nobody would be looking. This is what looks.
    ///
    /// Nothing where none is running or where time is left, so whoever calls it may call it
    /// whenever and this is what decides.
    pub fn stop_if_out_of_time(&self) -> Result<(), Refusal> {
        let now = self.outside.clock.now();
        let Some(session) = sessions::read(self.outside.sessions)?
            .running()
            .filter(|held| held.out_of_time(now))
            .map(Session::id)
        else {
            return Ok(());
        };
        self.stop(session, StoppedReason::BudgetHardlock)
    }

    /// What a session declared its budget in, or nothing where it is no longer there.
    pub(super) fn declared(&self, session: SessionId) -> Result<Option<Usage>, Refusal> {
        Ok(sessions::read(self.outside.sessions)?
            .sessions()
            .iter()
            .find(|held| held.id() == session)
            .map(|held| held.budget().usage))
    }

    /// What an amount in the unit a session declared is worth in the vendor's own.
    ///
    /// A ceiling is worked out in the unit the budget was declared in, and the vendor is told
    /// one in the unit it prices runs at. Nothing says what a token or a point of the limit is
    /// worth in that unit: it differs between subscriptions and the vendor is the one who
    /// decides. So it is not converted by a figure this ships, it is read off what runs here
    /// have already cost -- every run in the ledger reports both.
    ///
    /// Nothing where no run has reported both, which is where a session starts. The vendor is
    /// then told the figure its definition carries, which is a guard against a run that goes
    /// nowhere rather than this session's budget.
    pub(super) fn priced(&self, usage: Usage, amount: u64) -> Result<Option<u64>, Refusal> {
        let held = self.outside.runs.read()?;
        let (mut cost, mut over) = (0u64, 0u64);
        for run in held {
            let Some(spent) = run.spent else { continue };
            let Ok(priced) = spent.cost.parse::<u64>() else {
                continue;
            };
            let took = match usage {
                Usage::Tokens(_) => spending_of(&spent).map(|counted| counted.tokens()),
                Usage::Share(_) => match (run.limit_before, run.limit_after) {
                    (Some(before), Some(after)) => before
                        .parse::<u64>()
                        .ok()
                        .zip(after.parse::<u64>().ok())
                        .and_then(|(before, after)| after.checked_sub(before)),
                    _ => None,
                },
            };
            if let Some(took) = took.filter(|took| *took > 0) {
                cost = cost.saturating_add(priced);
                over = over.saturating_add(took);
            }
        }
        if over == 0 || cost == 0 {
            return Ok(None);
        }
        // Multiplied first, so that a ceiling smaller than what one unit costs is not nothing.
        Ok(Some(amount.saturating_mul(cost) / over))
    }

    /// How far the vendor's limit was spent when this session last looked.
    ///
    /// A store read rather than a vendor one, so it costs nothing. Nothing for a session
    /// declared in tokens, which never looks.
    pub(super) fn limit_last_seen(&self, session: SessionId) -> Result<Option<u64>, Refusal> {
        Ok(sessions::read(self.outside.sessions)?
            .sessions()
            .iter()
            .find(|held| held.id() == session)
            .and_then(Session::limit_last_seen))
    }

    pub(super) fn spending_of(&self, session: SessionId) -> Result<Option<Spending>, Refusal> {
        let held = sessions::read(self.outside.sessions)?;
        let held = held
            .sessions()
            .iter()
            .find(|held| held.id() == session)
            .ok_or(Refusal::NoSessionRunning)?;
        self.spending(held)
    }

    /// What the session has consumed of its usage budget, or nothing where it can no longer
    /// be read.
    ///
    /// A share is the vendor's limit added up look by look, so the looking is what moves the
    /// figure and this writes as well as reads. That is why it is taken before the hold the
    /// decision is made under: asking the vendor is slow, and the adding up has to happen in
    /// the order the looks were taken rather than the order they arrive.
    ///
    /// A count is the backlog's own sum and is taken under that hold instead, so the arm here
    /// only serves a caller asking outside a decision.
    ///
    /// A vendor that stops answering leaves a share unknown rather than zero. Section 1 stops a
    /// session in that state, and nothing is a failure for the caller to carry.
    fn spending(&self, held: &Session) -> Result<Option<Spending>, Refusal> {
        let session = held.id();
        let now = self.outside.clock.now();
        match (held.budget().usage, held.limit_at_start()) {
            (Usage::Share(_), Some(_)) => {
                let Ok((used, resets_at)) = self.limit_now() else {
                    return Ok(None);
                };
                sessions::change(self.outside.sessions, |sessions| {
                    Ok(sessions.measured(session, used, resets_at, now))
                })
            }
            // A share with nothing to measure from is a store this core cannot use.
            // Nothing else can be said about how much of it is spent.
            (Usage::Share(_), None) => Err(Refusal::Unavailable {
                reason: format!(
                    "{} declared a share and does not say what the limit was at",
                    session.labelled()
                ),
            }),
            (Usage::Tokens(_), _) => Ok(Some(Spending::Tokens(
                Consumption::total(backlog::read(self.outside.tasks)?.counted_in(session)).tokens(),
            ))),
        }
    }

    /// The session, if it is one this still decides for.
    fn held(&self, session: SessionId) -> Result<Option<Session>, Refusal> {
        let mut found = None;
        sessions::change(self.outside.sessions, |sessions| {
            found = sessions
                .sessions()
                .iter()
                .find(|held| held.id() == session)
                .filter(|held| held.state() == SessionState::Running)
                .cloned();
            Ok(())
        })?;
        Ok(found)
    }
}

/// What one decision came to, carried out of the hold it was made under.
enum Settled {
    Stop(Spending, StoppedReason),
    Started(Spending, Vec<TaskId>),
}

impl Settled {
    /// What the session had consumed when the decision was made.
    fn spent(&self) -> Spending {
        match self {
            Settled::Stop(spent, _) | Settled::Started(spent, _) => *spent,
        }
    }
}

/// How the session stands, from the backlog as it is held.
///
/// Every figure taken from the backlog comes from the one the assignment is made against, so
/// none of those can be from before another thread assigned.
///
/// What a share spent is not one of them. It is the vendor's, read before this hold, and put
/// beside a count of what ended that is read under it. A task ending between the two is
/// counted as having ended without what it spent being in the figure, so the cost of a task
/// reads low and `room_for` reads high. It is the wrong way to be wrong, and the alternative
/// What a store kept, as the core takes it.
fn spending_of(spent: &StoredConsumption) -> Option<Consumption> {
    Some(Consumption {
        input: spent.input.parse().ok()?,
        output: spent.output.parse().ok()?,
        cache_written: spent.cache_written.parse().ok()?,
        cache_read: spent.cache_read.parse().ok()?,
        cost: spent.cost.parse().ok()?,
    })
}

/// How far the vendor's limit moved for every millionth the ledger's runs were priced at, as a
/// pair to multiply and divide by rather than a fraction to round.
///
/// A run's own share of the limit cannot be read off the limit. The vendor keeps one figure for
/// the account, so two runs going at once move it together and no reading tells them apart;
/// taking a reading when each run ends splits the movement into stretches of time, and the run
/// that ends first is handed whatever the others spent while it ran. What a run reported for
/// itself is the only per-run figure there is.
///
/// So a run's size in the unit a share is declared in is what it was priced at, at the rate the
/// whole ledger moved the limit. The rate is right even where the split was not: the total
/// movement is the total movement however it is divided among the runs that caused it.
///
/// Priced rather than counted, because a token of one model is not a token of another. A rate
/// taken over tokens is out for any one model by however much its tokens cost more or less than
/// the rest of the ledger's, and the vendor has already told us that much in the price. What is
/// left over is whatever the limit weighs differently from the price, which is the smaller
/// question and the one there is no answer to here.
fn moved_per_millionth(held: &[Run]) -> Result<(u64, u64), Refusal> {
    let (mut moved, mut priced) = (0u64, 0u64);
    for run in held {
        let Some(spent) = run.spent.as_ref() else {
            continue;
        };
        let Some(counted) = spending_of(spent) else {
            continue;
        };
        let took = run
            .limit_before
            .as_ref()
            .zip(run.limit_after.as_ref())
            .and_then(|(before, after)| {
                let before: u64 = before.parse().ok()?;
                let after: u64 = after.parse().ok()?;
                after.checked_sub(before)
            });
        // A limit that read lower afterwards is a window that began again, and what was spent
        // before it turned over is in no reading at all.
        if let Some(took) = took.filter(|took| *took > 0) {
            moved = moved.saturating_add(took);
            priced = priced.saturating_add(counted.cost);
        }
    }
    Ok((moved, priced))
}

/// Which kind of sample a run is, or nothing where it is neither.
///
/// A run that finished says what its task takes. A run stopped at its ceiling says where it was
/// stopped. A run that failed or that the vendor turned away says neither.
fn sampled(run: &Run) -> Option<fn(Option<&str>, u64) -> Ran> {
    match (
        TaskState::parse(&run.outcome)?,
        run.reason.as_deref() == Some(AT_CEILING),
    ) {
        (TaskState::Completed, _) => Some(Ran::finished),
        (TaskState::Interrupted, true) => Some(Ran::stopped),
        _ => None,
    }
}

/// is holding the store for the ninety seconds the reading takes.
fn standing(
    tasks: &Backlog,
    held: &Session,
    spent: Spending,
    sizings: &Sizings,
    lasting: &Sizings,
    at_once: usize,
    now: u64,
) -> Standing {
    let session = held.id();
    Standing {
        left: held.budget().left(spent),
        booked: tasks.booked_in(session),
        sizings: sizings.clone(),
        lasting: lasting.clone(),
        time_left: held.time_left(now),
        pending: tasks.waiting(),
        blocked: tasks.blocked(),
        running: tasks.running_in(session),
        out_of_time: held.out_of_time(now),
        unreadable: matches!(tasks.consumed_by(session), Observation::Unreadable { .. }),
        at_once,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::core::{
        domain::{SessionId, StoppedReason},
        port::{
            inbound::{Carrying, ExecutionUseCase},
            outbound::{BacklogStore, Ended, Observed, Outcome},
        },
    };

    use super::super::fixtures::*;
    use super::super::{ExecutionService, Outside, Supervisor, WorkService};

    /// A budget is a figure.
    /// A session that cannot be measured against its own would run past it without anything noticing.
    #[test]
    fn a_session_whose_count_could_not_be_read_stops_and_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Finished,
            reason: None,
            observed: Observed::Unreadable {
                why: "the answer said nothing about it".to_owned(),
            },
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
        assert_eq!(held.consumed, None);
        assert_eq!(
            held.unreadable.as_deref(),
            Some("the answer said nothing about it")
        );

        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(
            session.stopped_reason.as_deref(),
            Some("observation unreadable")
        );
    }

    /// Carries every task a session has started through to its end.
    ///
    /// A session assigns as tasks end, so what is running has to be looked at again each time
    /// rather than listed once.
    fn carry_them_all(work: &WorkService<'_>, tasks: &Tasks) {
        for _ in 0..100 {
            let held = tasks.load().unwrap();
            let Some(running) = held.tasks.iter().find(|task| task.state == "Running") else {
                return;
            };
            work.carry_on(&format!("task:{}", running.id)).unwrap();
        }
        panic!("a task kept running");
    }

    fn states(tasks: &Tasks) -> Vec<String> {
        tasks
            .load()
            .unwrap()
            .tasks
            .iter()
            .map(|task| task.state.clone())
            .collect()
    }

    /// What a ceiling does to a session, with a stand-in that stops where it is told to.
    ///
    /// The tasks take more and more, and nothing has been run before, so the session learns
    /// what a run costs from the runs it has already had. The first task has nothing to go on
    /// and is given the whole budget; every one after it is held to a figure worked out from
    /// the runs before it. That figure is widened by how few runs it came from, which is what
    /// keeps a task that takes somewhat more than the last one from being stopped.
    ///
    /// One at a time, so that each task decides against everything before it. Four at once
    /// would start them all against the first figure.
    #[test]
    fn a_session_carries_tasks_that_take_more_than_the_ones_before_them() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![
            a_pending_task(),
            a_second_task(),
            a_task_numbered("3"),
            a_task_numbered("4"),
            a_task_numbered("5"),
        ]);
        let areas = Areas::default();
        let agent = Costing::taking([100, 200, 300, 400, 400]);
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
        let supervisor = Supervisor::new(outside, 1);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("100000", "8h")).unwrap();
        carry_them_all(&work, &tasks);

        assert_eq!(states(&tasks), ["Completed"; 5]);
    }

    /// Widening is not enough where the next task takes several times the last, and then the
    /// run that was stopped is what raises the figure. Nothing else does: the tasks after it
    /// are the ones that climb the ladder, so a session with none left ends where it stopped.
    #[test]
    fn a_run_that_was_stopped_is_what_lets_the_next_one_through() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![
            a_pending_task(),
            a_second_task(),
            a_task_numbered("3"),
        ]);
        let areas = Areas::default();
        let agent = Costing::taking([100, 400, 400]);
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
        let supervisor = Supervisor::new(outside, 1);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("100000", "8h")).unwrap();
        carry_them_all(&work, &tasks);

        // 100 finishes, 400 is stopped at twice 100, and the third goes through on the floor
        // that stopping left behind.
        assert_eq!(states(&tasks), ["Completed", "Interrupted", "Completed"]);
    }

    /// The other half of the hardlock: a session that has spent the tokens it declared stops.
    /// It stops whether or not its time is up.
    #[test]
    fn a_session_that_spent_what_it_declared_stops_and_says_so() {
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

        // The stand-in agent reports far more than this budget allows, so the first task spends the whole of it.
        execution.run(declaring("1000", "8h")).unwrap();
        let assigned = work.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
        // The second task was never assigned, so it is still waiting.
        assert_eq!(tasks.load().unwrap().tasks[1].state, "Pending");
    }

    /// A task moves the vendor's limit by less than a point, and for a while that read as costing nothing at all.
    ///
    /// Several tasks have to start once there is anything to divide by.
    #[test]
    fn a_share_starts_several_once_it_knows_what_a_task_costs() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![
            a_pending_task(),
            a_second_task(),
            a_task_numbered("3"),
            a_task_numbered("4"),
            a_task_numbered("5"),
        ]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        // Half a point every time it is asked, which is what a task cost when this was measured against the vendor.
        let moving = Advancing {
            used: Mutex::new(0),
            step: 50,
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

        // The first decision has no task to go on, so one starts alone.
        let started = execution.run(declaring("5%", "8h")).unwrap();
        assert_eq!(started.assigned.len(), 1);

        // The second knows what one task cost, and the rest of the budget holds far more than a handful.
        let assigned = work.carry_on("task:1").unwrap();
        assert_eq!(assigned.len(), 4);
    }

    /// A share is spent against a figure the vendor keeps.
    /// A session that reaches it stops however few tokens its own tasks reported.
    #[test]
    fn a_share_that_reached_what_it_declared_stops() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
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
        let assigned = work.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
        assert_eq!(tasks.load().unwrap().tasks[1].state, "Pending");
    }

    /// A session declared as a share outlives the window its limit is kept in.
    ///
    /// What it spent in the window it opened in is counted towards what it declared, and it
    /// stops on that as it would have without the window turning over.
    #[test]
    fn a_share_that_crosses_a_window_counts_both_of_them() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        // Opens at 30%, climbs to 34%, and then the window begins again at 2%.
        let turning = Turning::over(&[3_000, 3_400, 200]);
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &turning,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("10%", "8h")).unwrap();
        let assigned = work.carry_on("task:1").unwrap();

        // 4 points in the first window and 2 in the second, against the 10 it declared.
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.consumed, "600");

        // And the 4 that are left are what the next run may take. A run that crossed the
        // turnover says nothing about what a run of that model costs -- what it spent before
        // the window began again is in no reading at all -- so this is a session with a budget
        // and nothing to go on, which starts one and measures it.
        assert_eq!(assigned.len(), 1);
        assert_eq!(session.stopped_reason, None);
    }

    /// A run that crossed a window turning over says nothing about what a run costs.
    ///
    /// What it spent before the window began again is in no reading at all, so the difference
    /// either side of it is not what it cost. It is left out rather than counted low, and the
    /// session is then one with a budget and nothing to go on: one task starts and is
    /// measured, and the run after it has a figure again.
    ///
    /// Once per window at most, since only a run that spans the turnover is like this.
    #[test]
    fn a_run_that_crossed_a_window_is_no_sample_at_all() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![
            a_pending_task(),
            a_second_task(),
            a_task_numbered("3"),
            a_task_numbered("4"),
            a_task_numbered("5"),
        ]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let turning = Turning::over(&[3_000, 3_100, 100]);
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &turning,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        // Nothing has reported yet, so one starts alone.
        let started = execution.run(declaring("50%", "8h")).unwrap();
        assert_eq!(started.assigned.len(), 1);

        // The window turned over during that run, so it is no sample, and the session starts
        // one more to get one rather than stopping.
        let assigned = work.carry_on("task:1").unwrap();
        assert_eq!(assigned.len(), 1);
        assert_eq!(sessions.load().sessions[0].stopped_reason, None);
    }

    /// Two of a session's tasks ending at once each decide how many more fit.
    ///
    /// A count of what is running that was read before the backlog was held is a count from
    /// before the other thread assigned, and both threads then assign against it. What follows
    /// is more running than the machine was told to run.
    ///
    /// The interleaving is the machine's to choose, so this runs the scene many times rather
    /// than once.
    #[test]
    fn two_tasks_ending_at_once_do_not_start_more_than_the_machine_takes() {
        for _ in 0..64 {
            let sessions = Remembered::empty();
            let tasks = Tasks::holding((1..=10).map(|n| a_task_numbered(&n.to_string())).collect());
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

            // One starts alone, and the ones after it fill the machine.
            execution.run(declaring("4M", "8h")).unwrap();
            work.carry_on("task:1").unwrap();
            assert_eq!(running_in(&tasks), AT_ONCE);

            // Two of those four end together.
            std::thread::scope(|threads| {
                threads.spawn(|| work.carry_on("task:2"));
                threads.spawn(|| work.carry_on("task:3"));
            });

            assert!(
                running_in(&tasks) <= AT_ONCE,
                "{} running where the machine takes {AT_ONCE}",
                running_in(&tasks)
            );
        }
    }

    /// The budget binds before the ceiling on the machine does.
    ///
    /// What a session declared in tokens has spent is the backlog's own sum, and a task ending
    /// records into that same backlog. A count read before the hold is a count from before the
    /// other thread recorded, and the budget left over then reads higher than it is.
    ///
    /// One task here consumes 295,816 tokens and is allowed twice that, the estimate having
    /// been worked out from one run. Two of those fit in what is left of two million and three
    /// do not, so the budget binds before the machine's four does.
    #[test]
    fn two_tasks_ending_at_once_do_not_assign_past_what_is_left() {
        for _ in 0..64 {
            let sessions = Remembered::empty();
            let tasks = Tasks::holding((1..=10).map(|n| a_task_numbered(&n.to_string())).collect());
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
            // Room enough that what is left is what decides, not the machine.
            let supervisor = Supervisor::new(outside, 8);
            let execution = ExecutionService::new(outside, &supervisor);
            let work = WorkService::new(outside, &supervisor);

            // One alone, then two once there is a cost to divide by.
            execution.run(declaring("2M", "8h")).unwrap();
            work.carry_on("task:1").unwrap();
            assert_eq!(running_in(&tasks), 2);

            // Both of those end together, and another would put the session past its two million.
            std::thread::scope(|threads| {
                threads.spawn(|| work.carry_on("task:2"));
                threads.spawn(|| work.carry_on("task:3"));
            });

            let started = tasks
                .load()
                .unwrap()
                .tasks
                .iter()
                .filter(|task| task.state != "Pending")
                .count();
            assert!(
                started <= 6,
                "{started} tasks started against a budget for 6"
            );
        }
    }

    /// Section 1 stops a session whose usage can no longer be read.
    ///
    /// For a share that reading is the vendor's limit, and a session that cannot be measured
    /// must not go on spending against a budget nobody can check.
    #[test]
    fn a_share_whose_limit_stops_being_readable_stops_the_session() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let limit = AtPercent::at(1_000);
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &limit,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);
        execution.run(declaring("50%", "8h")).unwrap();

        limit.refuse();
        work.carry_on("1").unwrap();

        let held = sessions.load();
        assert_eq!(held.sessions[0].state, "stopped");
        assert_eq!(
            held.sessions[0].stopped_reason.as_deref(),
            Some("observation unreadable")
        );
    }

    /// Marking a task interrupted does not end the run behind it.
    ///
    /// A session that stopped while one was still going would go on spending against a budget
    /// it had already reported as spent, and nothing else would end it.
    #[test]
    fn stopping_a_session_ends_the_runs_it_still_had_going() {
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
        ExecutionService::new(outside, &supervisor)
            .run(declaring("2M", "8h"))
            .unwrap();

        let running = tasks.running();
        assert!(!running.is_empty(), "nothing was running to end");
        supervisor
            .stop(SessionId::parse("1").unwrap(), StoppedReason::AllDone)
            .unwrap();

        let ended = agent.stopped.lock().unwrap().clone();
        assert_eq!(ended, running, "a run outlived the session that started it");
    }

    /// A decision is reached when a task ends, so a session with one long run going would pass
    /// the time it declared with nobody looking.
    #[test]
    fn a_session_past_the_time_it_declared_is_stopped_without_anything_ending() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let opened = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        ExecutionService::new(opened, &Supervisor::new(opened, AT_ONCE))
            .run(declaring("2M", "8h"))
            .unwrap();
        assert_eq!(sessions.load().sessions[0].state, "running");

        // Eight hours on, with nothing having ended in between.
        let late = Frozen(1_000 + 8 * 3_600);
        let now = Supervisor::new(
            Outside {
                clock: &late,
                ..opened
            },
            AT_ONCE,
        );
        assert_eq!(now.time_left().unwrap(), Some(0));
        now.stop_if_out_of_time().unwrap();

        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
        // And what it had going was ended with it.
        assert_eq!(agent.stopped.lock().unwrap().len(), 1);
    }

    /// Whoever waits on it may call it whenever, and this is what decides.
    #[test]
    fn a_session_with_time_left_is_left_alone() {
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

        assert_eq!(supervisor.time_left().unwrap(), Some(8 * 3_600));
        supervisor.stop_if_out_of_time().unwrap();

        assert_eq!(sessions.load().sessions[0].state, "running");
    }

    /// Nothing is running, so there is nothing to hold to a deadline and nothing to wait on.
    #[test]
    fn nothing_running_has_no_deadline_to_wait_on() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let supervisor = Supervisor::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
                runs: &runs,
            },
            AT_ONCE,
        );

        assert_eq!(supervisor.time_left().unwrap(), None);
        supervisor.stop_if_out_of_time().unwrap();
    }

    /// What a run of a share-declared session cost is read from what it was priced at, not
    /// from how far the vendor's limit moved while it ran.
    ///
    /// The vendor keeps one figure for the account, so runs going at once move it together and
    /// the reading taken when each ends hands the first to finish whatever the others spent
    /// meanwhile. Two runs of the same size then look nothing alike. Here they report the same
    /// price and the readings split 900 to 100 between them, which is what that looks like;
    /// both are the same size all the same, and what the next task is allowed follows from
    /// that size rather than from the larger slice.
    #[test]
    fn two_runs_of_a_size_are_sized_alike_however_the_readings_split() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger(Mutex::new(vec![
            a_run_of("1", 500_000, ("1000", "1900")),
            a_run_of("2", 500_000, ("1900", "2000")),
        ]));
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

        // A thousand points over a million millionths, so half a million is 500 points. Both
        // runs are that, so the estimate is 500 and two runs to go on widen it by half. Were
        // the readings taken for the truth, the larger slice would make it 900.
        let started = execution.run(declaring("50%", "8h")).unwrap();

        assert_eq!(started.assigned.len(), 2);
        let held = tasks.load().unwrap();
        assert_eq!(held.tasks[0].ceiling.as_deref(), Some("750"));
        assert_eq!(held.tasks[1].ceiling.as_deref(), Some("750"));
    }

    /// Two runs the vendor priced alike are sized alike however far apart their token counts
    /// are.
    ///
    /// What a token costs differs between models by several times over, so a rate taken over
    /// tokens is out for any one model by that much. The price already carries the difference.
    /// Here one run counted ten times the other and was priced the same; both are one size.
    #[test]
    fn two_runs_of_a_price_are_sized_alike_however_far_apart_their_counts_are() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger(Mutex::new(vec![
            a_run_costing("1", 1_000_000, 500_000, ("1000", "1900")),
            a_run_costing("2", 100_000, 500_000, ("1900", "2000")),
        ]));
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

        // The same thousand points over the same million millionths, so both runs are 500 and
        // the estimate is 500 widened by half. Counted instead, the two would be 909 and 90.
        let started = execution.run(declaring("50%", "8h")).unwrap();

        assert_eq!(started.assigned.len(), 2);
        let held = tasks.load().unwrap();
        assert_eq!(held.tasks[0].ceiling.as_deref(), Some("750"));
        assert_eq!(held.tasks[1].ceiling.as_deref(), Some("750"));
    }
}
