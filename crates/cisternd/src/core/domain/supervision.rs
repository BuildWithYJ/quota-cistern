//! How many tasks may run, and whether a session has spent what it declared.
//!
//! Section 2.2 of `docs/cli.md` says assignment is dynamic.
//! Each time a task ends, what that task actually consumed decides whether one more fits.
//! The arithmetic behind that decision is here, apart from the stores and the clock the decision is made against.

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

/// What each waiting task may be allowed, taken in the order they wait.
///
/// Not a count. A count says how many fit and says nothing about what each of them may take,
/// so a session that starts four is a session that has given four runs no limit at all. Every
/// task that starts here leaves with a figure it may not pass.
///
/// What keeps the session inside what it declared is the first line: what is left, less what
/// the runs already going are allowed. Their allowances are held against the budget until they
/// end, so whatever they all spend, together they cannot pass it. Nothing about that depends
/// on a figure being right.
///
/// What a run of that model has cost caps it further. That figure can be wrong in either
/// direction without breaking the promise: too high and fewer start than could have, too low
/// and more start, each with a smaller allowance. It buys back the budget a run would
/// otherwise spend going nowhere.
///
/// A model nothing has finished a run with has no figure, so one task starts with the whole of
/// what is left, and what it costs is the figure the next answer is worked out from.
fn allow(standing: &Standing) -> Vec<Allowance> {
    let mut free = standing.left.saturating_sub(standing.booked);
    let mut given: Vec<Allowance> = Vec::new();

    for (task, model) in &standing.pending {
        if standing.running + given.len() >= standing.at_once || free == 0 {
            break;
        }
        let Some(sizing) = standing.sizings.model(model.as_deref()) else {
            // Nothing to go on. One task, measured, and the next decision has a figure.
            if standing.running == 0 && given.is_empty() {
                given.push(Allowance {
                    task: *task,
                    ceiling: free,
                });
            }
            break;
        };
        // A task that cannot finish before the session's time runs out is one the hardlock
        // would stop part way through, which spends what it spends and leaves nothing. Held to
        // the same quantile as a ceiling and for the same reason: not starting costs idle time,
        // and starting something that is stopped costs the work.
        //
        // Taken in the order they wait, so a task that does not fit ends the round rather than
        // letting a shorter one behind it go first. That is how a budget that does not stretch
        // to the next task is treated too.
        if standing
            .lasting
            .model(model.as_deref())
            .is_some_and(|lasts| lasts.estimate > standing.time_left)
        {
            break;
        }
        let want = sizing.allowing().max(1);
        let ceiling = if free >= want {
            want
        } else if standing.running == 0 && given.is_empty() && free >= sizing.median.max(1) {
            // Room for the middle of what this model costs but not for a whole ceiling.
            // More than half of what might start would finish inside what is left, which is
            // worth more than stopping a session that still has budget.
            free
        } else {
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

/// The quantile a ceiling is set at, in whole percent.
///
/// High rather than middling, because the two ways of being wrong do not cost the same. A
/// ceiling under what a run needs stops the run, and everything it spent up to then is paid
/// for with nothing to show; doing the work again means reading the whole conversation back in
/// as well. A ceiling over what a run needs costs nothing at all: what a run is allowed is held
/// against the budget while it goes and comes back unspent when it ends, so the price of aiming
/// high is that fewer run at once rather than that more is spent.
///
/// The figure is borrowed rather than measured. Systems that set a limit on something fatal to
/// exceed use the same range: Kubernetes' vertical autoscaler takes the 95th percentile for
/// memory and the 90th for CPU, and Google's Autopilot takes the 98th or the peak for memory
/// and the mean for batch CPU, each with a margin on top. A run of ours that meets its ceiling
/// loses its work, which puts it with memory rather than with CPU.
const ESTIMATE: u64 = 95;

/// The middle, in whole percent.
const MIDDLE: u64 = 50;

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

/// The numbers a run is sized by.
///
/// Held together and handed in rather than read from the constants above, so that comparing
/// two of them is a loop rather than a build each. What ships is [`Rule::default`], and
/// nothing outside a sweep sets anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    /// Which quantile of what a model's runs have cost a run is sized at.
    pub estimate: u64,
    /// The quantile a session falls back to where what is left will not cover a whole ceiling.
    pub middle: u64,
    /// How far the estimate is widened: `estimate x (1 + widen/over)`.
    pub widen: u64,
    /// How far a run that was stopped lifts the estimate above what it spent.
    ///
    /// Nothing to leave a stopped run out of the estimate altogether.
    pub lift: u64,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            estimate: ESTIMATE,
            middle: MIDDLE,
            widen: WIDEN,
            lift: LIFT,
        }
    }
}

/// What runs of one model have cost.
///
/// Two figures rather than one. A prediction used as a hard limit kills a run every time the
/// prediction is low, so what a run is allowed and what a run is expected to take are kept
/// apart: this says what to expect, and what is left of the budget says what is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizing {
    /// What a run of this model is expected to take, at the quantile above.
    ///
    /// Never under twice what a run of this model was stopped at. A run held to a ceiling was
    /// still working when it was stopped, so its task takes more than that; a figure set at
    /// where the last run stopped would stop the next one at the same place and stay there,
    /// having learnt nothing. Doubling is how a backfilling scheduler grows a prediction its
    /// job has already outlived, and it climbs to what the work takes in a few runs.
    pub estimate: u64,
    /// Half of them cost this or less.
    ///
    /// A session with nothing running lowers its bar to this rather than starting nothing,
    /// since more than half of what it might start would fit.
    pub median: u64,
    /// How many runs these were worked out from.
    ///
    /// Runs that finished. A run that was stopped says where it was stopped rather than what
    /// its task takes, so it raises the floor under `estimate` without being counted here.
    pub over: usize,
    /// How far to widen the estimate, from the rule these were worked out under.
    ///
    /// Carried here rather than asked for again, so that a sizing answers what it allows
    /// without whoever holds it having to remember which rule made it.
    pub widen: u64,
}

impl Sizing {
    /// What a run of this model may be allowed.
    ///
    /// The estimate widened by how little it was worked out from: `estimate x (1 + 1/over)`.
    /// Twice the estimate from one run, half as much again from two, an eighth more from
    /// eight. Kubernetes' vertical autoscaler widens its own upper bound by this shape and for
    /// this reason, and the shape is what recommends it -- nothing here picks a number to say
    /// how much wider, the count of runs says it.
    ///
    /// Why widen at all. The estimate is what a run is expected to take, and a run allowed
    /// exactly that is stopped the moment it takes any more; a figure worked out from one run
    /// is not one to hold the next to, since it says what that run happened to cost and
    /// nothing about what the next task is. This makes being stopped less likely. It does not
    /// make it unlikely: what runs cost is spread far wider than the uncertainty this covers,
    /// and the rest of the tail is not something a multiplier reaches.
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
/// A run stopped at its ceiling spent what it was stopped at. Counting that as what the task
/// costs closes a loop: the stopped figure pulls the estimate down toward the ceiling that
/// stopped it, the lower estimate stops the next run sooner, and the sizing settles under
/// what the work needs with nothing to pull it back up.
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
/// A model nothing has finished a run with has nothing here, and a session that meets one
/// starts a single task and measures it rather than guessing. The first run that finishes is
/// the first sample.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sizings {
    by_model: BTreeMap<String, Sizing>,
    /// Runs of a task that named no model, which the vendor answered with its own default.
    unnamed: Option<Sizing>,
}

impl Sizings {
    /// Works the figures out from what each run cost, keeping the models apart.
    ///
    /// Splitting by model is what keeps the figures from being five times out. The middle of
    /// what one model's runs cost differs from another's by more than that, so a session
    /// running one model against a figure taken over all of them is measured against a size
    /// nothing it runs is.
    /// Runs that were stopped are kept apart from runs that finished. What a stopped run cost
    /// is where we stopped it, which is a floor under what its task takes rather than a
    /// measure of it, so it lifts the estimate without being averaged into it.
    ///
    /// A model with nothing but stopped runs has no figure at all. One task then starts with
    /// the whole of what is left, which is more room than any floor would have given it.
    pub fn under(rule: Rule, runs: impl IntoIterator<Item = Ran>) -> Self {
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
                estimate: at(&costs, rule.estimate).max(floor.saturating_mul(rule.lift)),
                median: at(&costs, rule.middle),
                over: costs.len(),
                widen: rule.widen,
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
    /// Whether the time it declared has run out.
    pub out_of_time: bool,
    /// Whether what it consumed could no longer be read.
    pub unreadable: bool,
    /// The most tasks this machine has hands for.
    pub at_once: usize,
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
    let starting = match standing.out_of_time {
        true => Vec::new(),
        false => allow(standing),
    };
    // Nothing more fits and nothing is running that would make room.
    // Waiting for a task that will never start is not carrying on.
    if standing.running == 0 && starting.is_empty() {
        if standing.out_of_time {
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
