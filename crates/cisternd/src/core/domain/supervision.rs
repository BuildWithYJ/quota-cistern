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
/// A model nothing has been run with has no figure, so one task starts with the whole of what
/// is left, and what it costs is the figure the next answer is worked out from.
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
        let want = sizing.estimate.max(1);
        let ceiling = if free >= want {
            want
        } else if standing.running == 0 && given.is_empty() && free >= sizing.median.max(1) {
            // Room for the middle of what this model costs but not for three in four of it.
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

/// What runs of one model have cost.
///
/// Two figures rather than one. A prediction used as a hard limit kills a run every time the
/// prediction is low, so what a run is allowed and what a run is expected to take are kept
/// apart: this says what to expect, and what is left of the budget says what is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizing {
    /// Three runs in four cost this or less.
    pub estimate: u64,
    /// Half of them cost this or less.
    ///
    /// A session with nothing running lowers its bar to this rather than starting nothing,
    /// since more than half of what it might start would fit.
    pub median: u64,
    /// How many runs these were worked out from.
    pub over: usize,
}

/// What runs have cost, by the model that ran them.
///
/// A model nobody has run has nothing here, and a session that meets one starts a single task
/// and measures it rather than guessing. The first run is the first sample.
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
    pub fn of(runs: impl IntoIterator<Item = (Option<String>, u64)>) -> Self {
        let mut apart: BTreeMap<Option<String>, Vec<u64>> = BTreeMap::new();
        for (model, cost) in runs {
            apart.entry(model).or_default().push(cost);
        }
        let mut sizings = Sizings::default();
        for (model, mut costs) in apart {
            costs.sort_unstable();
            let sizing = Sizing {
                estimate: at(&costs, 75),
                median: at(&costs, 50),
                over: costs.len(),
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

/// The value at a percentile of a sorted list, taking the nearer rank.
///
/// Nearest rank rather than interpolated. A session has a handful of runs to work from, and a
/// figure between two of them is not one any run cost.
fn at(sorted: &[u64], percentile: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (percentile * sorted.len() as u64).div_ceil(100).max(1) as usize;
    sorted[rank - 1]
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
            sizings: Sizings::of([(Some("opus".to_owned()), 100)]),
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
        assert_eq!(ceilings(decide(&standing())), [100, 100]);
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

    /// Every task that starts leaves with a figure it may not pass, and what one model has
    /// cost is what it is.
    #[test]
    fn a_task_is_allowed_what_runs_of_its_model_have_cost() {
        assert_eq!(ceilings(decide(&standing())), [100, 100]);
    }

    /// What is left less what the runs already going are allowed. Whatever any of them
    /// actually spends, together they cannot pass what the session declared.
    #[test]
    fn what_is_already_allowed_is_held_against_the_budget() {
        assert_eq!(
            ceilings(decide(&Standing {
                left: 250,
                booked: 100,
                ..standing()
            })),
            [100],
            "150 was left, which covers one of these and not two"
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
                left: 250,
                pending: vec![
                    (task(1), Some("opus".to_owned())),
                    (task(2), Some("opus".to_owned())),
                    (task(3), Some("opus".to_owned())),
                ],
                ..standing()
            })),
            [100, 100]
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
                left: 60,
                running: 0,
                sizings: Sizings::of([
                    (Some("opus".to_owned()), 50),
                    (Some("opus".to_owned()), 100),
                ]),
                ..standing()
            })),
            [60]
        );
    }

    /// Not even the middle of it fits, and nothing is running that would make room.
    #[test]
    fn a_session_that_cannot_cover_the_middle_stops() {
        assert_eq!(
            decide(&Standing {
                left: 40,
                running: 0,
                sizings: Sizings::of([
                    (Some("opus".to_owned()), 50),
                    (Some("opus".to_owned()), 100),
                ]),
                ..standing()
            }),
            Decision::Stop(StoppedReason::BudgetHardlock)
        );
    }

    /// The bar is only lowered with nothing running. A session with a run going has that run
    /// to learn from and room may yet come back.
    #[test]
    fn the_bar_is_not_lowered_while_something_runs() {
        assert_eq!(
            decide(&Standing {
                left: 60,
                sizings: Sizings::of([
                    (Some("opus".to_owned()), 50),
                    (Some("opus".to_owned()), 100),
                ]),
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
        let sizings = Sizings::of([
            (Some("haiku".to_owned()), 10),
            (Some("haiku".to_owned()), 20),
            (Some("opus".to_owned()), 300),
            (Some("opus".to_owned()), 400),
        ]);

        assert_eq!(sizings.model(Some("haiku")).unwrap().estimate, 20);
        assert_eq!(sizings.model(Some("opus")).unwrap().estimate, 400);
        assert_eq!(sizings.model(Some("sonnet")), None);
    }

    /// A task that named no model is answered by the vendor's own default, which is a size of
    /// its own rather than any of the named ones.
    #[test]
    fn runs_that_named_no_model_are_their_own() {
        let sizings = Sizings::of([(None, 7), (Some("opus".to_owned()), 900)]);

        assert_eq!(sizings.model(None).unwrap().estimate, 7);
        assert_eq!(sizings.model(Some("opus")).unwrap().estimate, 900);
    }

    /// Nearest rank rather than interpolated. A session has a handful of runs to work from,
    /// and a figure between two of them is not one any run cost.
    #[test]
    fn the_figures_are_ones_a_run_actually_cost() {
        let sizings = Sizings::of([
            (Some("opus".to_owned()), 100),
            (Some("opus".to_owned()), 200),
            (Some("opus".to_owned()), 300),
            (Some("opus".to_owned()), 400),
        ]);
        let sizing = sizings.model(Some("opus")).unwrap();

        assert_eq!((sizing.estimate, sizing.median, sizing.over), (300, 200, 4));
    }

    #[test]
    fn one_run_is_both_figures() {
        let sizings = Sizings::of([(Some("opus".to_owned()), 42)]);
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
