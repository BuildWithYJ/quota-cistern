//! What a session does next, worked out from how it stands.
//!
//! Section 2.2 of `docs/cli.md` says assignment is dynamic: each time a task ends, what it
//! consumed decides whether one more fits. The arithmetic is here, apart from the stores and
//! the clock it is made against.
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

use std::{
    collections::BTreeMap,
    fmt::{self, Display},
};

use super::{Budget, StoppedReason, TaskId, Usage};

/// A percentage as hundredths of one.
///
/// A share is declared in whole percent and measured in hundredths.
/// One task moves the vendor's limit by less than a point.
pub const HUNDREDTHS: u64 = 100;

/// What a session has consumed of its usage budget, in the unit it declared.
///
/// Not two spellings of one number.
/// A share is how far the vendor's limit has moved since the session opened, which the account's other work moves too.
/// A count is what this session's own tasks reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spending {
    /// Hundredths of a percent.
    Share(u64),
    Tokens(u64),
}

impl Spending {
    /// Whether this figure was read before the one it is put beside.
    ///
    /// A session only ever spends more, so the lower of two figures of the same kind is the
    /// earlier reading. A share and a count are not put beside each other: they measure
    /// different things, and a session is declared in one of them for its whole life.
    pub fn behind(&self, other: &Spending) -> bool {
        match (self, other) {
            (Spending::Share(one), Spending::Share(another))
            | (Spending::Tokens(one), Spending::Tokens(another)) => one < another,
            _ => false,
        }
    }
}

impl Display for Spending {
    /// A share as the percentage a person declared, a count as the count.
    ///
    /// The hundredths only appear when there is something in them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Spending::Tokens(tokens) => write!(f, "{tokens}"),
            Spending::Share(points) => {
                let (whole, part) = (points / HUNDREDTHS, points % HUNDREDTHS);
                match part {
                    0 => write!(f, "{whole}%"),
                    _ if part % 10 == 0 => write!(f, "{whole}.{}%", part / 10),
                    _ => write!(f, "{whole}.{part:02}%"),
                }
            }
        }
    }
}

impl Budget {
    /// What is left of the usage declared, in the unit it was declared in.
    ///
    /// Nothing is left when more was spent than declared.
    /// Which is what a session that passed its budget between two decisions looks like.
    pub fn left(&self, spent: Spending) -> u64 {
        match (self.usage, spent) {
            (Usage::Share(declared), Spending::Share(spent)) => {
                (u64::from(declared) * HUNDREDTHS).saturating_sub(spent)
            }
            (Usage::Tokens(declared), Spending::Tokens(spent)) => declared.saturating_sub(spent),
            // A session is measured in the unit it was declared in.
            // Whoever read the spending read it for this session.
            _ => 0,
        }
    }
}

/// Whether a run of this model would finish before the session's time runs out.
///
/// Starting one that would not spends what it spends and leaves nothing. A model nothing has
/// been timed on holds nothing back.
fn fits_the_clock(standing: &Standing, model: Option<&str>) -> bool {
    match standing.policy.timing {
        Timing::Any => true,
        Timing::Fits => !standing
            .lasting
            .model(model)
            .is_some_and(|lasts| lasts.estimate > standing.time_left),
    }
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
    let Some(sizing) = standing.sizings.model(model) else {
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
    let mut free = standing.left.saturating_sub(standing.booked);
    let mut given: Vec<Allowance> = Vec::new();

    for (task, model) in &standing.pending {
        let alone = standing.running == 0 && given.is_empty();
        if free == 0 || !fits_the_clock(standing, model.as_deref()) {
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

/// The quantile a run is sized at while others are going, in whole percent.
///
/// The third quarter. Others are going, so a run that goes over eats budget they were counting
/// on, and a size three runs in four come in under is far enough up to make that rare.
const BUSY: u64 = 75;

/// The quantile a run is sized at when nothing else is going, in whole percent.
///
/// The first quarter. With nothing else going there is nobody to take budget from, so a
/// session that would otherwise stop with budget in hand starts one more and is optimistic
/// about it. This is the only place a session is.
const ALONE: u64 = 25;

/// How far a stopped run lifts the estimate above where it was stopped.
///
/// Twice, which is how a backfilling scheduler grows a prediction its job has already outlived.
const LIFT: u64 = 2;

/// How far an estimate is widened for how little it was worked out from.
///
/// One, so an estimate from a single run allows twice it and one from four allows a quarter
/// more. What this should be is not something four sessions on a real repository could say:
/// none of their runs came within half of its ceiling, so any figure here would have ended
/// them the same way. It is a number to sweep rather than one to argue about.
const WIDEN: u64 = 1;

/// How a session is run. One value, swapped whole.
///
/// Every figure a decision turns on is here rather than written into the code that reads it,
/// so comparing two ways of running a session is a loop rather than a build each, and a
/// session can say afterwards which one it ran under. What ships is [`Policy::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Which quantile a run is sized at while others are going.
    pub busy: u64,
    /// Which quantile it is sized at when nothing else is going.
    pub alone: u64,
    /// How far the size is widened for how few runs it came from: `size x (1 + widen/over)`.
    pub widen: u64,
    /// How far a run that was stopped lifts the size above what it spent. Nothing leaves
    /// stopped runs out altogether.
    pub lift: u64,
    /// Whether a run is held back for the clock.
    pub timing: Timing,
}

/// What a session does about a run that the clock may not let finish.
///
/// Section 2.5 of `docs/cli.md` lets a person choose. Which is right is not settled: the time
/// a session declared no longer ends a run that is going, so what holding one back buys is
/// less than it was, and no run of a real session has yet been held back by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    /// Hold it back. Starting one the clock will stop part way spends what it spends and
    /// leaves nothing.
    Fits,
    /// Start it anyway. A session out of time takes nothing more on and lets what is going
    /// finish, so a run past the deadline is not stopped for it.
    Any,
}

impl Timing {
    /// Reads what `config set timing` was given.
    pub fn parse(timing: &str) -> Option<Self> {
        match timing {
            "fits" => Some(Timing::Fits),
            "any" => Some(Timing::Any),
            _ => None,
        }
    }
}

impl Display for Timing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Timing::Fits => "fits",
            Timing::Any => "any",
        })
    }
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            busy: BUSY,
            alone: ALONE,
            widen: WIDEN,
            lift: LIFT,
            timing: Timing::Fits,
        }
    }
}

/// What runs of one model have cost.
///
/// Two figures: one for while others are going and one for when none is. A session is
/// conservative in the first place and optimistic in the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizing {
    /// What to set aside while other runs are going.
    ///
    /// Never under twice what a run of this model was stopped at: a run held to a ceiling was
    /// still working, so its task takes more than that, and a figure set where it stopped
    /// would stop the next one in the same place forever.
    pub estimate: u64,
    /// What to set aside when nothing else is going.
    pub fallback: u64,
    /// How many finished runs these were worked out from.
    pub over: usize,
    /// How far to widen the estimate, from the rule these were worked out under.
    ///
    /// Carried here rather than asked for again, so that a sizing answers what it allows
    /// without whoever holds it having to remember which rule made it.
    pub widen: u64,
}

impl Sizing {
    /// What to set aside for a run of this model, widened by how few runs it came from:
    /// `estimate x (1 + widen/over)`.
    pub fn allowing(&self) -> u64 {
        self.estimate.saturating_add(
            self.estimate
                .saturating_mul(self.widen)
                .saturating_div(self.over.max(1) as u64),
        )
    }
}

/// What one run cost, and whether that figure is what its task takes.
///
/// A run stopped at its ceiling spent what it was stopped at, which is a floor under what its
/// task takes rather than a measure of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    model: Option<String>,
    spent: u64,
    stopped: bool,
}

impl Ran {
    /// A run that did the work, which cost what its task takes.
    pub fn finished(model: Option<&str>, spent: u64) -> Self {
        Self {
            model: model.map(str::to_owned),
            spent,
            stopped: false,
        }
    }

    /// A run held to a ceiling, which says only that its task takes at least this much.
    pub fn stopped(model: Option<&str>, spent: u64) -> Self {
        Self {
            model: model.map(str::to_owned),
            spent,
            stopped: true,
        }
    }
}

/// What runs have cost, by the model that ran them.
///
/// A model nothing has finished a run with has nothing here. A session that meets one starts
/// a single task and measures it rather than guessing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sizings {
    by_model: BTreeMap<String, Sizing>,
    /// Runs of a task that named no model, which the vendor answered with its own default.
    unnamed: Option<Sizing>,
}

impl Sizings {
    /// Works the figures out from what each run cost, keeping the models apart.
    ///
    /// Split by model, since what one model's runs cost differs from another's by several
    /// times over. Runs that were stopped lift the estimate rather than being averaged into
    /// it, and a model with nothing but stopped runs has no figure at all.
    pub fn under(policy: Policy, runs: impl IntoIterator<Item = Ran>) -> Self {
        let mut apart: BTreeMap<Option<String>, (Vec<u64>, u64)> = BTreeMap::new();
        for run in runs {
            let (finished, floor) = apart.entry(run.model).or_default();
            match run.stopped {
                false => finished.push(run.spent),
                true => *floor = (*floor).max(run.spent),
            }
        }
        let mut sizings = Sizings::default();
        for (model, (mut costs, floor)) in apart {
            if costs.is_empty() {
                continue;
            }
            costs.sort_unstable();
            let sizing = Sizing {
                estimate: at(&costs, policy.busy).max(floor.saturating_mul(policy.lift)),
                fallback: at(&costs, policy.alone),
                over: costs.len(),
                widen: policy.widen,
            };
            match model {
                Some(model) => {
                    sizings.by_model.insert(model, sizing);
                }
                None => sizings.unnamed = Some(sizing),
            }
        }
        sizings
    }

    /// What a run of this model has cost, or nothing where none has.
    pub fn model(&self, model: Option<&str>) -> Option<Sizing> {
        match model {
            Some(model) => self.by_model.get(model).copied(),
            None => self.unnamed,
        }
    }
}

/// The value at a quantile of a sorted list, by the rule Hyndman and Fan recommend.
///
/// `h = (n + 1/3)p + 1/3`, read between the two values it falls between and held inside the
/// list at either end. The figure it gives is as often above the quantile it was asked for as
/// below it, whatever shape the values were drawn from.
///
/// Which rule is used matters at these sizes. Taking the nearer rank asks four runs for their
/// 75th and returns the third of them, and the kth of n sorted values sits at k/(n+1) of
/// whatever they came from, so the third of four is the 60th. A ceiling set there stops two
/// runs in five rather than the one in four it was asked for. That is arithmetic about ranks
/// rather than an assumption about the shape: the position of an order statistic follows
/// Beta(k, n - k + 1) for any continuous distribution, whose mean is k/(n+1).
///
/// Held whole throughout. `h` is carried in three-hundredths, which is exact for a quantile
/// given in whole percent, and the reading between two values rounds down: a ceiling that
/// lands under a figure some run cost is the safer way to be a fraction out.
fn at(sorted: &[u64], per_cent: u64) -> u64 {
    let Some(last) = sorted.len().checked_sub(1) else {
        return 0;
    };
    let over = 300 * sorted.len() as u64;
    let h = 3 * per_cent * sorted.len() as u64 + per_cent + 100;
    if h <= 300 {
        return sorted[0];
    }
    if h >= over {
        return sorted[last];
    }
    // Between 1 and n - 1 inclusive, since h is over 300 and under 300n.
    let under = (h / 300) as usize;
    sorted[under - 1] + (sorted[under] - sorted[under - 1]) * (h % 300) / 300
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
    /// What the budget still holds, in the unit it was declared in.
    pub left: u64,
    /// What the runs already going are allowed to take, together.
    ///
    /// Held against the budget until they end. Two runs starting at once are each given what
    /// is left less what the other was given, so what they may spend together is what the
    /// session has, however much either of them actually takes.
    pub booked: u64,
    /// What runs have cost, by the model that ran them.
    pub sizings: Sizings,
    /// How long runs have taken, by the model that ran them, in seconds.
    pub lasting: Sizings,
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
    /// How this session is being run.
    pub policy: Policy,
}

/// What follows from how a session stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Stop(StoppedReason),
    /// Start these, each with what it may take. May be none.
    Start(Vec<Allowance>),
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
        if out_of_time {
            return Decision::Stop(StoppedReason::BudgetHardlock);
        }
        return Decision::Stop(match (standing.pending.is_empty(), standing.blocked) {
            // Tasks that may start and nothing to start them with.
            (false, _) => StoppedReason::BudgetHardlock,
            // Tasks left, and every one of them waits on one that did not complete.
            (true, true) => StoppedReason::Blocked,
            (true, false) => StoppedReason::AllDone,
        });
    }
    Decision::Start(starting)
}

#[cfg(test)]
mod tests;
