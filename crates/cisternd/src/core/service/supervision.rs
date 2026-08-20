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
        Backlog, Consumption, Decision, HUNDREDTHS, Observation, Ran, Rule, Session, SessionId,
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
    /// The numbers a run is sized by.
    ///
    /// What ships is `Rule::default`. A sweep that compares two of them varies this, so that
    /// comparing them is a loop rather than a build each.
    rule: Rule,
}

impl<'a> Supervisor<'a> {
    pub fn new(outside: Outside<'a>, at_once: usize) -> Self {
        Self::sizing_by(outside, at_once, Rule::default())
    }

    /// The same, sizing by a rule other than the one that ships.
    pub fn sizing_by(outside: Outside<'a>, at_once: usize, rule: Rule) -> Self {
        Supervisor {
            outside,
            at_once,
            rule,
        }
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
        Ok(Sizings::under(
            self.rule,
            held.into_iter().filter_map(|run| {
                let ran = sampled(&run)?;
                let counted = spending_of(&run.spent?)?;
                let cost = match per_millionth {
                    None => counted.tokens(),
                    Some((moved, over)) => counted.cost.checked_mul(moved)? / over.max(1),
                };
                Some(ran(run.model.as_deref(), cost))
            }),
        ))
    }

    /// How long a run of each model has taken, from the ledger, in seconds.
    ///
    /// Told apart the same way as what runs cost: a run stopped part way through took less
    /// time than its task needs, for the same reason it spent less.
    fn lasting(&self) -> Result<Sizings, Refusal> {
        let held = self.outside.runs.read()?;
        Ok(Sizings::under(
            self.rule,
            held.into_iter().filter_map(|run| {
                let ran = sampled(&run)?;
                let started = run.started_at.parse::<u64>().ok()?;
                let ended = run.ended_at.parse::<u64>().ok()?;
                Some(ran(run.model.as_deref(), ended.checked_sub(started)?))
            }),
        ))
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
    /// Stops a session that has had the time it declared and has nothing left going.
    ///
    /// A decision is only reached when a task ends, so a session whose last task ended before
    /// its time did would sit open until the time ran out with nobody looking. This is what
    /// looks.
    ///
    /// It does not end a run that is still going. The time a session declared is a deadline
    /// for taking work on rather than for finishing it: a run past that time is one whose
    /// length we guessed short, and ending it spends everything it spent for nothing. What
    /// ends a run is its own turn ceiling, or a person.
    ///
    /// So a session with a run going stops at the decision that run's ending reaches.
    pub fn stop_if_out_of_time(&self) -> Result<(), Refusal> {
        let now = self.outside.clock.now();
        let Some(session) = sessions::read(self.outside.sessions)?
            .running()
            .filter(|held| held.out_of_time(now))
            .map(Session::id)
        else {
            return Ok(());
        };
        if backlog::read(self.outside.tasks)?.running_in(session) > 0 {
            return Ok(());
        }
        self.stop(session, StoppedReason::BudgetHardlock)
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
mod tests;
