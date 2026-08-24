//! What a session does next, worked out from how it stands.
//!
//! Section 2.2 of `docs/cli.md` says assignment is dynamic: each time a task ends, what it
//! consumed decides whether one more fits. The rule is here, apart from the stores and the
//! clock it is made against, and apart from the figures it turns on, which are `policy`'s.
//!
//! Three questions, asked apart and answered together.
//!
//! ```text
//! usage   what a run of this model has cost      Sizings
//! time    how long one has taken, and how long the session has left
//! budget  what is left, less what running tasks are already allowed
//! ```
//!
//! None of them knows how many hands the machine has. What the budget covers is what `allow`
//! answers; how many of those actually start is the service's to say.
//!
//! Every verdict a session receives is reached here. A caller that knows a fact the decision
//! turns on hands it over rather than acting on it, so that two callers cannot come to
//! different answers about one session.

use super::{Before, Locking, Pacing, StoppedReason, TaskId, Timing};

/// Whether a run of this model would finish before the session's time runs out.
///
/// Starting one that would not spends what it spends and leaves nothing. A model nothing has
/// been timed on holds nothing back.
fn fits_the_clock(standing: &Standing, model: Option<&str>) -> bool {
    match standing.timing {
        Timing::Any => true,
        Timing::Fits => !standing
            .before
            .lasting
            .model(model)
            .is_some_and(|lasts| lasts.estimate > standing.time_left),
    }
}
/// Whether the budget outlasts a run of this model at the rate it is going.
///
/// A run that is still going when the budget runs out is ended, and what it spent past its last
/// commit buys nothing. One run ending that way is the session spending what it declared; four
/// is the same figure spent and four results lost, so what this guards against grows with how
/// many are going.
///
/// A high rate early is not what this asks about. Early on there is budget enough that the rate
/// does not matter, and this passes whatever the rate; late there is not, and only a short run
/// gets through. The rate is the session's own, wall clock, so runs going at once are already
/// in it.
///
/// Asked at the widened figure rather than at the middle, since the two ways of being wrong do
/// not cost the same: a run held back that would have fitted leaves budget unspent, and a run
/// started that does not fit spends and leaves nothing.
///
/// A model nothing has been timed on holds nothing back, and neither does a session that has
/// yet to spend anything: the first run is what makes the figure this reads.
fn fits_the_budget(standing: &Standing, model: Option<&str>) -> bool {
    if standing.pacing == Pacing::Any {
        return true;
    }
    let Some(until_gone) = standing.until_gone() else {
        return true;
    };
    !standing
        .before
        .lasting
        .model(model)
        .is_some_and(|lasts| lasts.allowing() > until_gone)
}

/// What to set aside for a run of this model, or nothing where what is left will not cover it.
///
/// Two figures and two situations. While others are going a run is sized at what three in four
/// come in under, since going over eats budget they were counting on. With nothing going there
/// is nobody to take from, so what is left goes to one more run rather than being left unspent.
///
/// A model nothing has finished a run with has no figure at all, and one task then starts with
/// what is left and is measured.
fn set_aside(standing: &Standing, model: Option<&str>, free: u64, alone: bool) -> Option<u64> {
    let Some(sizing) = standing.before.cost.model(model) else {
        return alone.then_some(free);
    };
    let want = sizing.allowing().max(1);
    match () {
        _ if free >= want => Some(want),
        _ if alone && free >= sizing.fallback.max(1) => Some(free),
        _ => None,
    }
}
/// What each waiting task may be allowed, taken in the order they wait.
///
/// What holds the session to what it declared is the first line: what is left, less what the
/// runs already going are allowed. Nothing about that depends on any figure being right.
///
/// Taken in order, so a task that does not fit ends the round rather than letting a shorter
/// one behind it go first. How many of these actually start is the machine's to say, not this.
fn allow(standing: &Standing) -> Vec<Allowance> {
    let mut free = standing.left().saturating_sub(standing.booked);
    let mut given: Vec<Allowance> = Vec::new();

    for (task, model) in &standing.pending {
        let alone = standing.running == 0 && given.is_empty();
        if free == 0
            || !fits_the_clock(standing, model.as_deref())
            || !fits_the_budget(standing, model.as_deref())
        {
            break;
        }
        let Some(ceiling) = set_aside(standing, model.as_deref(), free, alone) else {
            break;
        };
        given.push(Allowance {
            task: *task,
            ceiling,
        });
        free -= ceiling;
    }
    given
}
/// What one task is allowed to consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowance {
    pub task: TaskId,
    /// In the unit the budget was declared in.
    ///
    /// What is left of the budget less what running tasks are already allowed, capped at what
    /// a run of this model is expected to take. The first of the two is what keeps the session
    /// inside what it declared however wrong the second is.
    pub ceiling: u64,
}
/// How a session stands at the moment something has to be decided about it.
///
/// Every figure is read before any of them is judged, so the decision that follows is made
/// from one moment rather than from a store that moved while it was being asked.
pub struct Standing {
    /// What the budget declared, in the unit it was declared in.
    pub declared: u64,
    /// What has been spent of it, in that same unit.
    ///
    /// Beside what was declared rather than as the one figure left over, because two questions
    /// are asked of the pair: how much is left to hand out, and how fast it is going.
    pub spent: u64,
    /// How long the session has been running, in seconds.
    pub elapsed: u64,
    /// What the runs already going are allowed to take, together.
    ///
    /// Held against the budget until they end. Two runs starting at once are each given what
    /// is left less what the other was given, so what they may spend together is what the
    /// session has, however much either of them actually takes.
    pub booked: u64,
    /// What the runs before this one came to, by the model that ran them.
    pub before: Before,
    /// How long the session has before the time it declared runs out, in seconds.
    pub time_left: u64,
    /// The tasks that may start, in the order they would be taken, each with the model it
    /// named.
    ///
    /// A count was enough while every task was allowed the same thing. Now each is allowed
    /// what its own model says, so which ones they are is part of the decision.
    pub pending: Vec<(TaskId, Option<String>)>,
    /// How many of its tasks are running.
    pub running: usize,
    /// Whether tasks are left that none of them may start.
    pub blocked: bool,
    /// Whether what it consumed could no longer be read.
    pub unreadable: bool,
    /// Whether a run is held back for the clock.
    pub timing: Timing,
    /// What to do about runs still going once the budget is spent.
    pub locking: Locking,
    /// What to do about a run the budget will not outlast.
    pub pacing: Pacing,
}

impl Standing {
    /// What the budget still holds. Nothing once more was spent than declared, which is what a
    /// session that passed its figure between two decisions looks like.
    pub fn left(&self) -> u64 {
        self.declared.saturating_sub(self.spent)
    }

    /// How long the budget lasts at the rate it has been going, in seconds.
    ///
    /// Nothing where the session has spent nothing or has been running no time, which is where
    /// a first run has yet to say anything. A session with a figure here has one because runs
    /// of its own produced it.
    fn until_gone(&self) -> Option<u64> {
        match (self.spent, self.elapsed) {
            (0, _) | (_, 0) => None,
            (spent, elapsed) => Some(self.left().saturating_mul(elapsed) / spent),
        }
    }
}
/// What follows from how a session stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Stop(StoppedReason),
    /// Start these, each with what it may take. May be none.
    Start(Vec<Allowance>),
}
/// Whether a session that has had the time it declared has anything left to wait for.
///
/// The time a session declared is a deadline for taking work on rather than for finishing it,
/// so one past that time stops when what it had going has ended and not before. Two callers ask
/// it: the decision a task's ending reaches, and the clock, which is the only thing that looks
/// at a session whose last task ended before its time did.
pub fn done_waiting(time_left: u64, running: usize) -> bool {
    time_left == 0 && running == 0
}
/// Why a session with nothing running and nothing it may start stops.
///
/// A backlog that emptied is done whatever else is true of the session. The clock and the
/// budget running out are what stops one that still had tasks it could have run, and telling a
/// person the budget locked when the work simply finished says the wrong thing about both.
///
/// Asked here rather than worked out twice: a decision reaches this state when a task ends,
/// and the clock reaches it when a session outlives its deadline with nothing going.
pub fn nothing_more(waiting: bool, blocked: bool) -> StoppedReason {
    match (waiting, blocked) {
        (true, _) => StoppedReason::BudgetHardlock,
        (false, true) => StoppedReason::Blocked,
        (false, false) => StoppedReason::AllDone,
    }
}

/// What to do about a session, from how it stands.
///
/// Nothing here reaches outside, so the whole of the rule is in one place and a test of it
/// needs no store, no clock, and no vendor.
pub fn decide(standing: &Standing) -> Decision {
    // A count nobody could read leaves a budget that cannot be measured.
    // A budget that cannot be measured cannot be held to.
    if standing.unreadable {
        return Decision::Stop(StoppedReason::ObservationUnreadable);
    }
    // What a person declared is a figure, and a session that has spent it has spent it. This
    // is the only stop that ends runs still going, which is what `Cuts` is for; `Waits` lets
    // them finish and spends past the figure by however far they had to go.
    //
    // Only where something is going. With nothing going there is nothing to end, and what to
    // call the stopping is the question below: a backlog that emptied at the same moment the
    // budget did is done rather than locked out of it.
    if standing.locking == Locking::Cuts && standing.running > 0 && standing.left() == 0 {
        return Decision::Stop(StoppedReason::BudgetHardlock);
    }
    // Out of time starts nothing more. It does not end what is going: the time a session
    // declared is a deadline for taking work on, and a run that is past it is a run whose
    // length we guessed short. Ending it there spends everything it spent and leaves nothing,
    // and the guess was ours rather than anything the task did.
    // Nothing left of the time it declared is what out of time means, and it is the same
    // figure `time_left` holds; a second field for it could disagree with the first.
    let out_of_time = standing.time_left == 0;
    let starting = match out_of_time {
        true => Vec::new(),
        false => allow(standing),
    };
    // Nothing more fits and nothing is running that would make room.
    // Waiting for a task that will never start is not carrying on.
    if standing.running == 0 && starting.is_empty() {
        return Decision::Stop(nothing_more(!standing.pending.is_empty(), standing.blocked));
    }
    Decision::Start(starting)
}

#[cfg(test)]
mod tests;
