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
        AT_CEILING, Backlog, Consumption, Decision, HUNDREDTHS, Observation, Policy, Ran, Session,
        SessionId, SessionState, Sizings, Spending, Standing, StoppedReason, TaskId, TaskState,
        Timing, Usage, decide,
    },
    port::{
        inbound::Refusal,
        outbound::{Agent, BacklogStore, Clock, Limit, Run, Runs, SessionStore, Traces, Worktrees},
    },
};

use super::{backlog, sessions};

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
    policy: Policy,
}

impl<'a> Supervisor<'a> {
    pub fn new(outside: Outside<'a>, at_once: usize) -> Self {
        Self::running_by(outside, at_once, Policy::default())
    }

    /// The same, run by a policy other than the one that ships.
    pub(super) fn running_by(outside: Outside<'a>, at_once: usize, policy: Policy) -> Self {
        Supervisor {
            outside,
            at_once,
            policy,
        }
    }

    /// The same, told what a person chose in the words they wrote it in.
    ///
    /// Text because the domain is private to the core, which is how the vendor's name reaches
    /// `ConfigurationService`. Nothing chosen leaves the policy as it ships, and a word this
    /// does not know is refused by name rather than ignored.
    pub fn timed_by(
        outside: Outside<'a>,
        at_once: usize,
        timing: Option<&str>,
    ) -> Result<Self, String> {
        let Some(said) = timing else {
            return Ok(Supervisor::new(outside, at_once));
        };
        let Some(timing) = Timing::parse(said) else {
            return Err(format!(
                "the configuration says timing {said}, which is neither fits nor any"
            ));
        };
        Ok(Supervisor::running_by(
            outside,
            at_once,
            Policy {
                timing,
                ..Policy::default()
            },
        ))
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
                    .map(|_| Vec::new());
            }
            (Usage::Share(_), spent) => spent,
            (Usage::Tokens(_), _) => None,
        };

        // What runs have cost, in the unit this session declared, and how long they took. Both
        // are worked out from one read of the ledger, which grows a line for every run there
        // has ever been. File reads rather than vendor ones, and taken before the hold like the
        // vendor reading was.
        let ledger = self.outside.runs.read()?;
        let sizings = self.sizings(held.budget().usage, &ledger);
        let lasting = self.lasting(&ledger);

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
                    self.policy,
                    now,
                )) {
                    // Stopping takes this store again and the sessions store with it, so it happens
                    // once this hold is given up rather than under it.
                    Decision::Stop(why) => Settled::Stop(spent, why),
                    // How many of these start is the machine's to say. The decision says
                    // what the budget will cover; this box has so many hands, and a task
                    // assigned with no thread waiting for it would sit still holding its
                    // share of the budget.
                    Decision::Start(allowed) => Settled::Started(
                        spent,
                        allowed
                            .iter()
                            .take(self.at_once.saturating_sub(tasks.running_in(session)))
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
                recorded.and_then(|()| self.stop(session, why).map(|_| Vec::new()))
            }
            Settled::Started(_, assigned) => Ok(assigned),
        }
    }

    /// Stops the session and ends whatever it still had running.
    ///
    /// Marking a task interrupted does not end the run behind it. The run is a process this
    /// core started, and a session that stopped while one was still going would go on spending
    /// against a budget it had already reported as spent.
    /// Answers with the tasks that were running and now are not, which is what a person who
    /// asked for the stopping is told.
    pub(super) fn stop(
        &self,
        session: SessionId,
        why: StoppedReason,
    ) -> Result<Vec<TaskId>, Refusal> {
        let now = self.outside.clock.now();
        // The runs end before the tasks do, so nothing is recorded as ended while its agent is
        // still working.
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
            Ok(tasks.interrupt(session, &why.to_string(), now))
        })
    }

    /// What runs have cost, by the model that ran them, in the unit a session declared.
    ///
    /// A share and a count are different questions of the same ledger. A run's price is there
    /// whatever its session declared; how far it moved the vendor's limit is there only for a
    /// session that was watching that limit, since only those read it.
    ///
    /// Runs that say nothing in the unit asked for are left out rather than counted as zero.
    /// A figure worked out from runs that reported nothing is a figure about nothing.
    ///
    /// A run that finished says what its task takes. A run stopped at its ceiling says where it
    /// was stopped, which the sizing holds as a floor rather than counting as a measure. Runs
    /// that ended any other way are left out: a run the vendor turned away or that failed on its
    /// own spent what it spent before it went wrong, which is neither.
    fn sizings(&self, usage: Usage, held: &[Run]) -> Sizings {
        // How far the vendor's limit moves for a millionth of what it prices a run at, over
        // every run that reported both. Nothing here is in the unit a share is declared in
        // until a run has said both, which is where a session declared as a share starts.
        let per_millionth = match usage {
            Usage::Share(_) => match moved_per_millionth(held) {
                // No run has said both yet, so nothing here can be put in the unit this
                // session declared. A figure of nothing would size every run at nothing and
                // allow every run almost nothing, so there is no figure at all: one task
                // starts with what is left and is measured, as at the beginning.
                (0, _) | (_, 0) => return Sizings::default(),
                both => Some(both),
            },
            Usage::Tokens(_) => None,
        };
        sized(self.policy, held, |run| {
            let counted = backlog::counted(run.spent.as_ref()?)?;
            match per_millionth {
                None => Some(counted.tokens()),
                Some((moved, over)) => Some(counted.cost.checked_mul(moved)? / over.max(1)),
            }
        })
    }

    /// How long a run of each model has taken, from the ledger, in seconds.
    ///
    /// Told apart the same way as what runs cost: a run stopped part way through took less
    /// time than its task needs, for the same reason it spent less.
    fn lasting(&self, held: &[Run]) -> Sizings {
        sized(self.policy, held, |run| {
            let started = run.started_at.parse::<u64>().ok()?;
            let ended = run.ended_at.parse::<u64>().ok()?;
            ended.checked_sub(started)
        })
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
            .map(|_| ())
    }

    /// How far the vendor's limit was spent when this session last looked.
    ///
    /// A store read rather than a vendor one, so it costs nothing. Nothing for a session
    /// declared in tokens, which never looks.
    pub(super) fn limit_last_seen(&self, session: SessionId) -> Result<Option<u64>, Refusal> {
        Ok(sessions::read(self.outside.sessions)?
            .find(session)
            .and_then(Session::limit_last_seen))
    }

    pub(super) fn spending_of(&self, session: SessionId) -> Result<Option<Spending>, Refusal> {
        let held = sessions::read(self.outside.sessions)?;
        let held = held.find(session).ok_or(Refusal::NoSessionRunning)?;
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
        Ok(sessions::read(self.outside.sessions)?
            .find(session)
            .filter(|held| held.state() == SessionState::Running)
            .cloned())
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
fn moved_per_millionth(held: &[Run]) -> (u64, u64) {
    let (mut moved, mut priced) = (0u64, 0u64);
    for run in held {
        let Some(spent) = run.spent.as_ref() else {
            continue;
        };
        let Some(counted) = backlog::counted(spent) else {
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
    (moved, priced)
}

/// What runs come to, by the model that ran them, under one figure taken from each.
///
/// Two questions of the same ledger, what a run cost and how long it took, and the same runs
/// answer both. A run that finished says what its task takes; one stopped at its ceiling says
/// only where it was stopped, which the sizing holds as a floor; the rest say neither.
fn sized(policy: Policy, held: &[Run], figure: impl Fn(&Run) -> Option<u64>) -> Sizings {
    Sizings::under(
        policy,
        held.iter().filter_map(|run| {
            let ran = sampled(run)?;
            Some(ran(run.model.as_deref(), figure(run)?))
        }),
    )
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

/// How a session stands, from the backlog as it is held.
///
/// Everything read from a store rather than from the vendor, so this costs nothing and takes
/// no time. What the vendor had to be asked was asked before the hold was taken.
fn standing(
    tasks: &Backlog,
    held: &Session,
    spent: Spending,
    sizings: &Sizings,
    lasting: &Sizings,
    policy: Policy,
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
        unreadable: matches!(tasks.consumed_by(session), Observation::Unreadable { .. }),
        policy,
    }
}

#[cfg(test)]
mod tests;
