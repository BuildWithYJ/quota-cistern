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
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            estimate: ESTIMATE,
            middle: MIDDLE,
            widen: WIDEN,
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
                estimate: at(&costs, rule.estimate).max(floor.saturating_mul(2)),
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
    if standing.out_of_time {
        return Decision::Stop(StoppedReason::BudgetHardlock);
    }
    let starting = allow(standing);
    // Nothing more fits and nothing is running that would make room.
    // Waiting for a task that will never start is not carrying on.
    if standing.running == 0 && starting.is_empty() {
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
mod tests {
    use crate::core::domain::Span;

    use super::*;

    fn declaring(usage: Usage) -> Budget {
        Budget {
            usage,
            time: Span::parse("8h").unwrap(),
        }
    }

    fn task(n: u32) -> TaskId {
        TaskId::parse(&n.to_string()).unwrap()
    }

    /// A session with room, nothing wrong, one task running, and two waiting.
    fn standing() -> Standing {
        Standing {
            left: 1_000,
            booked: 0,
            sizings: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 100)]),
            // Nothing has been timed, so the clock lets everything start. The tests that are
            // about the clock say what runs have taken.
            lasting: Sizings::default(),
            time_left: 8 * 60 * 60,
            pending: vec![
                (task(1), Some("opus".to_owned())),
                (task(2), Some("opus".to_owned())),
            ],
            running: 1,
            blocked: false,
            out_of_time: false,
            unreadable: false,
            at_once: 4,
        }
    }

    fn ceilings(decision: Decision) -> Vec<u64> {
        match decision {
            Decision::Start(allowed) => allowed.iter().map(|one| one.ceiling).collect(),
            Decision::Stop(why) => panic!("stopped for {why}"),
        }
    }

    // what a decision comes to

    #[test]
    fn a_session_with_room_and_something_waiting_starts_more() {
        assert_eq!(ceilings(decide(&standing())), [200, 200]);
    }

    #[test]
    fn a_consumption_nobody_could_read_stops_it() {
        assert_eq!(
            decide(&Standing {
                unreadable: true,
                ..standing()
            }),
            Decision::Stop(StoppedReason::ObservationUnreadable)
        );
    }

    /// Time is the other half of the budget, and it runs out with money left.
    #[test]
    fn a_session_out_of_time_stops_with_budget_left() {
        assert_eq!(
            decide(&Standing {
                out_of_time: true,
                ..standing()
            }),
            Decision::Stop(StoppedReason::BudgetHardlock)
        );
    }

    /// Nothing running, tasks waiting, and no room for any of them.
    #[test]
    fn nothing_running_and_no_room_is_the_end_of_the_budget() {
        assert_eq!(
            decide(&Standing {
                left: 0,
                running: 0,
                ..standing()
            }),
            Decision::Stop(StoppedReason::BudgetHardlock)
        );
    }

    #[test]
    fn nothing_running_and_nothing_waiting_is_all_done() {
        assert_eq!(
            decide(&Standing {
                running: 0,
                pending: Vec::new(),
                ..standing()
            }),
            Decision::Stop(StoppedReason::AllDone)
        );
    }

    /// A task still running may yet leave room, so the session waits for it.
    #[test]
    fn a_session_with_nothing_waiting_carries_on_while_one_runs() {
        assert_eq!(
            decide(&Standing {
                pending: Vec::new(),
                ..standing()
            }),
            Decision::Start(Vec::new())
        );
    }

    #[test]
    fn a_session_with_no_room_left_starts_none_while_one_runs() {
        assert_eq!(
            decide(&Standing {
                left: 0,
                ..standing()
            }),
            Decision::Start(Vec::new())
        );
    }

    // what each task is allowed

    /// Every task that starts leaves with a figure it may not pass, worked out from what runs
    /// of its model have cost and widened by how few of them there were. One run of 100 here,
    /// so the figure is twice it.
    #[test]
    fn a_task_is_allowed_more_than_runs_of_its_model_have_cost() {
        assert_eq!(ceilings(decide(&standing())), [200, 200]);
    }

    /// What is left less what the runs already going are allowed. Whatever any of them
    /// actually spends, together they cannot pass what the session declared.
    #[test]
    fn what_is_already_allowed_is_held_against_the_budget() {
        assert_eq!(
            ceilings(decide(&Standing {
                left: 500,
                booked: 200,
                ..standing()
            })),
            [200],
            "300 was left, which covers one of these and not two"
        );
    }

    /// The list is taken in order until what is left will not cover the next one.
    ///
    /// The leftover is not handed out. A run allowed less than its model usually takes is a
    /// run that will be cut off, and a task waiting is worth more than a branch nobody can
    /// use. Something is running, so room may yet come back.
    #[test]
    fn what_is_left_runs_out_partway_down_the_list() {
        assert_eq!(
            ceilings(decide(&Standing {
                left: 500,
                pending: vec![
                    (task(1), Some("opus".to_owned())),
                    (task(2), Some("opus".to_owned())),
                    (task(3), Some("opus".to_owned())),
                ],
                ..standing()
            })),
            [200, 200]
        );
    }

    /// A model nothing has been run with has no figure to go on. One task starts with the
    /// whole of what is left, and what it costs is what the next decision is made from.
    #[test]
    fn with_nothing_to_go_on_one_task_starts_with_the_whole_of_it() {
        assert_eq!(
            ceilings(decide(&Standing {
                sizings: Sizings::default(),
                running: 0,
                ..standing()
            })),
            [1_000]
        );
    }

    /// And nothing more starts while that one runs, since it is the sample.
    #[test]
    fn with_nothing_to_go_on_and_one_running_nothing_starts() {
        assert_eq!(
            decide(&Standing {
                sizings: Sizings::default(),
                ..standing()
            }),
            Decision::Start(Vec::new())
        );
    }

    /// Room for the middle of what this model costs but not for three runs in four of it.
    /// More than half of what might start would finish inside what is left, which is worth
    /// more than stopping a session that still has budget.
    #[test]
    fn a_session_with_nothing_running_lowers_its_bar_to_the_middle() {
        assert_eq!(
            ceilings(decide(&Standing {
                left: 80,
                running: 0,
                sizings: Sizings::under(
                    Rule::default(),
                    [
                        Ran::finished(Some("opus"), 50),
                        Ran::finished(Some("opus"), 100),
                    ]
                ),
                ..standing()
            })),
            [80]
        );
    }

    /// Not even the middle of it fits, and nothing is running that would make room.
    #[test]
    fn a_session_that_cannot_cover_the_middle_stops() {
        assert_eq!(
            decide(&Standing {
                left: 40,
                running: 0,
                sizings: Sizings::under(
                    Rule::default(),
                    [
                        Ran::finished(Some("opus"), 50),
                        Ran::finished(Some("opus"), 100),
                    ]
                ),
                ..standing()
            }),
            Decision::Stop(StoppedReason::BudgetHardlock)
        );
    }

    /// The hardlock stops a session part way through whatever it has going, so a task started
    /// with less time than a run of its model takes is one that will be stopped like that,
    /// having spent what it spent and left nothing.
    #[test]
    fn a_task_that_cannot_finish_in_the_time_left_does_not_start() {
        assert_eq!(
            decide(&Standing {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
                time_left: 600,
                ..standing()
            }),
            Decision::Start(Vec::new())
        );
    }

    #[test]
    fn a_task_that_fits_the_time_left_starts() {
        assert_eq!(
            ceilings(decide(&Standing {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
                time_left: 3_600,
                ..standing()
            })),
            [200, 200]
        );
    }

    /// Waiting out the clock with tasks that may not start is not carrying on, and starting one
    /// of them would spend on work the hardlock takes away again.
    #[test]
    fn a_session_with_nothing_running_and_no_time_for_another_run_stops() {
        assert_eq!(
            decide(&Standing {
                running: 0,
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
                time_left: 600,
                ..standing()
            }),
            Decision::Stop(StoppedReason::BudgetHardlock)
        );
    }

    /// A model nothing has been timed on is not held to a clock it has no figure for. The first
    /// run of it is the first sample, as it is for what a run costs.
    #[test]
    fn a_model_nothing_has_been_timed_on_starts_whatever_the_time_left() {
        assert_eq!(
            ceilings(decide(&Standing {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("haiku"), 900)]),
                time_left: 1,
                ..standing()
            })),
            [200, 200]
        );
    }

    /// The bar is only lowered with nothing running. A session with a run going has that run
    /// to learn from and room may yet come back.
    #[test]
    fn the_bar_is_not_lowered_while_something_runs() {
        assert_eq!(
            decide(&Standing {
                left: 60,
                sizings: Sizings::under(
                    Rule::default(),
                    [
                        Ran::finished(Some("opus"), 50),
                        Ran::finished(Some("opus"), 100),
                    ]
                ),
                ..standing()
            }),
            Decision::Start(Vec::new())
        );
    }

    #[test]
    fn no_more_than_a_handful_start_however_large_the_budget() {
        assert_eq!(
            ceilings(decide(&Standing {
                left: 1_000_000,
                running: 0,
                pending: (1..=10)
                    .map(|n| (task(n), Some("opus".to_owned())))
                    .collect(),
                ..standing()
            }))
            .len(),
            4
        );
    }

    // what a model's runs have cost

    /// The middle of what one model's runs cost differs from another's by more than a factor
    /// of five, so a session running one model against a figure taken over all of them is
    /// measured against a size nothing it runs is.
    #[test]
    fn each_model_is_worked_out_from_its_own_runs() {
        let sizings = Sizings::under(
            Rule::default(),
            [
                Ran::finished(Some("haiku"), 10),
                Ran::finished(Some("haiku"), 20),
                Ran::finished(Some("opus"), 300),
                Ran::finished(Some("opus"), 400),
            ],
        );

        assert_eq!(sizings.model(Some("haiku")).unwrap().estimate, 20);
        assert_eq!(sizings.model(Some("opus")).unwrap().estimate, 400);
        assert_eq!(sizings.model(Some("sonnet")), None);
    }

    /// A task that named no model is answered by the vendor's own default, which is a size of
    /// its own rather than any of the named ones.
    #[test]
    fn runs_that_named_no_model_are_their_own() {
        let sizings = Sizings::under(
            Rule::default(),
            [Ran::finished(None, 7), Ran::finished(Some("opus"), 900)],
        );

        assert_eq!(sizings.model(None).unwrap().estimate, 7);
        assert_eq!(sizings.model(Some("opus")).unwrap().estimate, 900);
    }

    /// Read between two runs rather than at the nearer of them.
    ///
    /// The kth of n sorted runs sits at k/(n+1) of what they were drawn from, so the nearer
    /// rank answers a question it was not asked: four runs asked for their 75th return the
    /// third, which is the 60th, and a ceiling there stops two runs in five rather than one in
    /// four. Four runs asked for their 95th have nothing above the largest to offer, and their
    /// middle falls between the second and the third.
    #[test]
    fn the_figures_are_read_between_the_runs() {
        let sizings = Sizings::under(
            Rule::default(),
            [
                Ran::finished(Some("opus"), 100),
                Ran::finished(Some("opus"), 200),
                Ran::finished(Some("opus"), 300),
                Ran::finished(Some("opus"), 400),
            ],
        );
        let sizing = sizings.model(Some("opus")).unwrap();

        assert_eq!((sizing.estimate, sizing.median, sizing.over), (400, 250, 4));
    }

    /// Until there are runs enough for a figure to sit under the largest of them, the estimate
    /// is the largest. Thirteen runs is where it starts pulling in, and a session works from a
    /// handful, so what a ceiling comes to in practice is the dearest run of that model yet.
    #[test]
    fn the_estimate_only_falls_under_the_dearest_run_once_there_are_runs_enough() {
        let costs = |over: u64| {
            Sizings::under(
                Rule::default(),
                (1..=over).map(|each| Ran::finished(Some("opus"), each * 100)),
            )
            .model(Some("opus"))
            .unwrap()
            .estimate
        };

        assert_eq!((costs(4), costs(8), costs(13)), (400, 800, 1_300));
        assert_eq!(costs(14), 1_395);
    }

    /// A run stopped at its ceiling spent what it was stopped at. Counting that as what the
    /// task costs would pull the estimate down toward the ceiling that stopped it, and the
    /// lower estimate would stop the next run sooner.
    /// The numbers are the rule's rather than the module's, so a sweep hands in another one
    /// instead of being built again for each.
    #[test]
    fn a_rule_of_someones_own_is_what_a_sizing_is_worked_out_by() {
        let runs = || (1..=4).map(|each| Ran::finished(Some("opus"), each * 100));
        let sized = |rule| Sizings::under(rule, runs()).model(Some("opus")).unwrap();

        let shipped = Sizings::under(Rule::default(), runs())
            .model(Some("opus"))
            .unwrap();
        let wider = sized(Rule {
            widen: 4,
            ..Rule::default()
        });
        let lower = sized(Rule {
            estimate: 50,
            ..Rule::default()
        });

        assert_eq!((shipped.estimate, shipped.allowing()), (400, 500));
        assert_eq!((wider.estimate, wider.allowing()), (400, 800));
        assert_eq!((lower.estimate, lower.allowing()), (250, 312));
    }

    #[test]
    fn a_run_that_was_stopped_is_not_counted_as_what_its_task_costs() {
        let sizings = Sizings::under(
            Rule::default(),
            [
                Ran::finished(Some("opus"), 300),
                Ran::finished(Some("opus"), 400),
                Ran::stopped(Some("opus"), 20),
            ],
        );
        let sizing = sizings.model(Some("opus")).unwrap();

        assert_eq!((sizing.estimate, sizing.median, sizing.over), (400, 350, 2));
    }

    /// It was still working when it was stopped, so its task takes more than that. Holding the
    /// estimate at where it stopped would stop the next run at the same place and stay there.
    #[test]
    fn a_run_that_was_stopped_lifts_the_estimate_past_where_it_stopped() {
        let sizings = Sizings::under(
            Rule::default(),
            [
                Ran::finished(Some("opus"), 300),
                Ran::finished(Some("opus"), 400),
                Ran::stopped(Some("opus"), 900),
            ],
        );
        let sizing = sizings.model(Some("opus")).unwrap();

        assert_eq!((sizing.estimate, sizing.over), (1_800, 2));
    }

    /// Nothing has finished, so there is nothing to size from. One task then starts with the
    /// whole of what is left, which is more room than the floor would have given it.
    #[test]
    fn a_model_that_has_only_been_stopped_has_no_figure() {
        let sizings = Sizings::under(Rule::default(), [Ran::stopped(Some("opus"), 900)]);

        assert_eq!(sizings.model(Some("opus")), None);
    }

    #[test]
    fn one_run_is_both_figures() {
        let sizings = Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 42)]);
        let sizing = sizings.model(Some("opus")).unwrap();

        assert_eq!((sizing.estimate, sizing.median, sizing.over), (42, 42, 1));
    }

    // the budget itself

    #[test]
    fn a_share_is_shown_as_the_percentage_it_was_declared_in() {
        assert_eq!(Spending::Share(400).to_string(), "4%");
        assert_eq!(Spending::Share(350).to_string(), "3.5%");
        assert_eq!(Spending::Share(405).to_string(), "4.05%");
        assert_eq!(Spending::Share(0).to_string(), "0%");
    }

    #[test]
    fn a_count_is_shown_as_the_count() {
        assert_eq!(Spending::Tokens(2_000_000).to_string(), "2000000");
    }

    #[test]
    fn what_is_left_is_what_was_declared_less_what_was_spent() {
        let budget = declaring(Usage::Tokens(1_000));
        assert_eq!(budget.left(Spending::Tokens(400)), 600);
    }

    /// A share is declared in whole percent and measured in hundredths.
    #[test]
    fn a_share_is_left_over_in_hundredths_of_a_percent() {
        let budget = declaring(Usage::Share(50));
        assert_eq!(budget.left(Spending::Share(2_000)), 3_000);
    }

    /// A session can pass its budget between two decisions.
    /// A decision is made when a task ends and not while one runs.
    #[test]
    fn spending_more_than_was_declared_leaves_nothing() {
        let budget = declaring(Usage::Tokens(1_000));
        assert_eq!(budget.left(Spending::Tokens(4_000)), 0);
    }

    #[test]
    fn nothing_is_left_when_the_unit_is_not_the_one_declared() {
        let budget = declaring(Usage::Share(50));
        assert_eq!(budget.left(Spending::Tokens(1)), 0);
    }

    #[test]
    fn spending_more_of_a_share_than_was_declared_leaves_nothing() {
        let budget = declaring(Usage::Share(1));
        assert_eq!(budget.left(Spending::Share(4_000)), 0);
    }

    /// Tasks are left and every one of them waits on one that did not complete.
    ///
    /// A ceiling makes this ordinary: a task cut off at one ends `Interrupted`, and a task
    /// that waits on it may never start. Reporting it as everything being done says the
    /// opposite of what happened.
    #[test]
    fn tasks_left_that_none_may_start_is_not_everything_being_done() {
        assert_eq!(
            decide(&Standing {
                running: 0,
                pending: Vec::new(),
                blocked: true,
                ..standing()
            }),
            Decision::Stop(StoppedReason::Blocked)
        );
    }
}
