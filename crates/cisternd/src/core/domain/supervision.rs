//! How many tasks may run, and whether a session has spent what it declared.
//!
//! Section 2.2 of `docs/cli.md` says assignment is dynamic.
//! Each time a task ends, what that task actually consumed decides whether one more fits.
//! The arithmetic behind that decision is here, apart from the stores and the clock the decision is made against.

use std::fmt::{self, Display};

use super::{Budget, StoppedReason, Usage};

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

/// What a set of tasks cost, and how many of them there were.
///
/// The pair rather than the average.
/// The average of a share is a fraction, and a fraction in whole numbers is nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    /// What they cost together, in the unit the budget was declared in.
    pub total: u64,
    /// How many of them there were.
    pub over: u64,
}

/// How many more tasks may be started.
///
/// What is left, divided by what one task cost, multiplying first so that the fraction survives.
/// A set that cost nothing measurable leaves one task to start.
/// Which is what makes the sample the next answer is worked out from.
fn room_for(left: u64, cost: Option<Cost>, running: usize, at_once: usize) -> usize {
    if left == 0 {
        return 0;
    }
    let fits = match cost.filter(|cost| cost.total > 0 && cost.over > 0) {
        Some(cost) => (left.saturating_mul(cost.over) / cost.total).min(at_once as u64) as usize,
        None => 1,
    };
    fits.saturating_sub(running)
}

/// How a session stands at the moment something has to be decided about it.
///
/// Every figure is read before any of them is judged, so the decision that follows is made
/// from one moment rather than from a store that moved while it was being asked.
pub struct Standing {
    /// What the budget still holds, in the unit it was declared in.
    pub left: u64,
    /// What one of this session's tasks has cost, over how many of them reported.
    pub cost: Option<Cost>,
    /// How many of its tasks are running.
    pub running: usize,
    /// Whether the backlog holds a task that may start.
    pub waiting: bool,
    /// Whether the time it declared has run out.
    pub out_of_time: bool,
    /// Whether what it consumed could no longer be read.
    pub unreadable: bool,
    /// The most tasks this machine has hands for.
    pub at_once: usize,
}

/// What follows from how a session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Stop(StoppedReason),
    /// Start this many more, which may be none.
    Start(usize),
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
    // Nothing more fits and nothing is running that would make room.
    // Waiting for a task that will never start is not carrying on.
    if standing.running == 0 && room_for(standing.left, standing.cost, 0, standing.at_once) == 0 {
        return Decision::Stop(StoppedReason::BudgetHardlock);
    }
    if standing.running == 0 && !standing.waiting {
        return Decision::Stop(StoppedReason::AllDone);
    }
    Decision::Start(room_for(
        standing.left,
        standing.cost,
        standing.running,
        standing.at_once,
    ))
}

/// What a set of tasks cost together, and how many reported a cost.
pub fn cost_of(costs: impl IntoIterator<Item = u64>) -> Cost {
    costs
        .into_iter()
        .fold(Cost { total: 0, over: 0 }, |cost, one| Cost {
            total: cost.total.saturating_add(one),
            over: cost.over + 1,
        })
}

#[cfg(test)]
mod tests {
    /// A session with room, nothing wrong, and one task running.
    fn standing() -> Standing {
        Standing {
            left: 1_000,
            cost: Some(Cost {
                total: 100,
                over: 1,
            }),
            running: 1,
            waiting: true,
            out_of_time: false,
            unreadable: false,
            at_once: AT_ONCE,
        }
    }

    #[test]
    fn a_session_with_room_and_something_waiting_starts_more() {
        assert_eq!(decide(&standing()), Decision::Start(3));
    }

    /// Section 1 says a budget that cannot be measured stops the session.
    #[test]
    fn a_consumption_nobody_could_read_stops_it() {
        let unreadable = Standing {
            unreadable: true,
            ..standing()
        };
        assert_eq!(
            decide(&unreadable),
            Decision::Stop(StoppedReason::ObservationUnreadable)
        );
    }

    /// Time is asked about before room, so a session out of time stops even with budget left.
    #[test]
    fn a_session_out_of_time_stops_with_budget_left() {
        let late = Standing {
            out_of_time: true,
            ..standing()
        };
        assert_eq!(decide(&late), Decision::Stop(StoppedReason::BudgetHardlock));
    }

    #[test]
    fn nothing_running_and_no_room_is_the_end_of_the_budget() {
        let spent = Standing {
            left: 0,
            running: 0,
            ..standing()
        };
        assert_eq!(
            decide(&spent),
            Decision::Stop(StoppedReason::BudgetHardlock)
        );
    }

    #[test]
    fn nothing_running_and_nothing_waiting_is_all_done() {
        let over = Standing {
            running: 0,
            waiting: false,
            ..standing()
        };
        assert_eq!(decide(&over), Decision::Stop(StoppedReason::AllDone));
    }

    /// A task still going may yet make room, so nothing is decided until it ends.
    #[test]
    fn a_session_with_nothing_waiting_carries_on_while_one_runs() {
        let running = Standing {
            waiting: false,
            ..standing()
        };
        assert!(matches!(decide(&running), Decision::Start(_)));
    }

    #[test]
    fn a_session_with_no_room_left_starts_none_while_one_runs() {
        let full = Standing {
            left: 50,
            cost: Some(Cost {
                total: 100,
                over: 1,
            }),
            ..standing()
        };
        assert_eq!(decide(&full), Decision::Start(0));
    }

    /// The machine limit the composition root passes in, fixed here so the
    /// arithmetic is what is being checked.
    const AT_ONCE: usize = 4;

    use super::*;
    use crate::core::domain::Span;

    fn declaring(usage: Usage) -> Budget {
        Budget {
            usage,
            time: Span::parse("8h").unwrap(),
        }
    }

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

    fn over(total: u64, over: u64) -> Option<Cost> {
        Some(Cost { total, over })
    }

    #[test]
    fn with_nothing_to_go_on_one_task_starts() {
        assert_eq!(room_for(1_000_000, None, 0, AT_ONCE), 1);
    }

    #[test]
    fn what_is_left_divided_by_what_one_costs_is_how_many_start() {
        assert_eq!(room_for(300, over(100, 1), 0, AT_ONCE), 3);
        assert_eq!(room_for(300, over(300, 3), 0, AT_ONCE), 3);
    }

    /// Two tasks that moved the vendor's limit one point cost half a point each.
    ///
    /// Working out that half first and then dividing by it loses the whole answer.
    /// A session declared as a share would run one task at a time forever.
    #[test]
    fn a_cost_smaller_than_one_still_says_how_many_fit() {
        // One point left, half a point each.
        assert_eq!(room_for(1, over(1, 2), 0, AT_ONCE), 2);
        assert_eq!(room_for(49, over(1, 2), 0, AT_ONCE), AT_ONCE);
    }

    #[test]
    fn what_is_already_running_counts_against_that() {
        assert_eq!(room_for(300, over(100, 1), 2, AT_ONCE), 1);
        assert_eq!(room_for(300, over(100, 1), 3, AT_ONCE), 0);
    }

    /// A large budget divides into more tasks than a machine should run.
    #[test]
    fn no_more_than_a_handful_start_however_large_the_budget() {
        assert_eq!(room_for(1_000_000, over(1, 1), 0, AT_ONCE), AT_ONCE);
    }

    #[test]
    fn nothing_starts_once_the_budget_is_spent() {
        assert_eq!(room_for(0, over(100, 1), 0, AT_ONCE), 0);
        assert_eq!(room_for(0, None, 0, AT_ONCE), 0);
    }

    #[test]
    fn nothing_starts_when_one_task_costs_more_than_is_left() {
        assert_eq!(room_for(50, over(100, 1), 0, AT_ONCE), 0);
    }

    /// A set that has cost nothing at all has not run yet.
    #[test]
    fn a_set_that_cost_nothing_starts_one_at_a_time() {
        assert_eq!(room_for(50, over(0, 2), 0, AT_ONCE), 1);
    }

    #[test]
    fn a_cost_is_what_a_set_came_to_and_how_many_were_in_it() {
        assert_eq!(
            cost_of([100, 200, 300]),
            Cost {
                total: 600,
                over: 3
            }
        );
        assert_eq!(cost_of([]), Cost { total: 0, over: 0 });
    }
}
