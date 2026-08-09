//! How many tasks may run, and whether a session has spent what it declared.
//!
//! Section 2.2 of `docs/cli.md` says assignment is dynamic: each time a task
//! ends, what that task actually consumed decides whether one more fits. The
//! arithmetic behind that decision is here, apart from the stores and the
//! clock the decision is made against.

use super::{Budget, Usage};

/// The most tasks that run at once, whatever the budget would allow.
///
/// A guard on the machine rather than on the budget. Each task is a checkout
/// of a repository and an agent process of its own, and a session with a large
/// budget would otherwise start as many as the budget divides into.
const AT_ONCE: usize = 4;

/// What a session has consumed of its usage budget, in the unit it declared.
///
/// A share and a count are not two spellings of one number. A share is how far
/// the vendor's own limit has moved since the session opened, which the vendor
/// reports and which the account's other work moves too. A count is the tokens
/// this session's tasks reported, and nothing else adds to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spending {
    Share(u32),
    Tokens(u64),
}

impl Budget {
    /// What is left of the usage declared, in the unit it was declared in.
    ///
    /// Nothing is left when more was spent than declared, which is what a
    /// session that passed its budget between two decisions looks like.
    pub fn left(&self, spent: Spending) -> u64 {
        match (self.usage, spent) {
            (Usage::Share(declared), Spending::Share(spent)) => {
                u64::from(declared.saturating_sub(spent))
            }
            (Usage::Tokens(declared), Spending::Tokens(spent)) => declared.saturating_sub(spent),
            // A session is measured in the unit it was declared in, and
            // whoever read the spending read it for this session.
            _ => 0,
        }
    }
}

/// What a set of tasks cost, and how many of them there were.
///
/// The pair rather than the average, because the average of a share is a
/// fraction. Two tasks that moved the vendor's limit one point cost half a
/// point each, and half a point in whole numbers is nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    /// What they cost together, in the unit the budget was declared in.
    pub total: u64,
    /// How many of them there were.
    pub over: u64,
}

/// How many more tasks may be started.
///
/// What is left, divided by what one of these tasks cost. The multiplication
/// comes first so that the fraction survives it.
///
/// A set that cost nothing measurable says nothing about how many fit, and
/// neither does an empty one. Both leave one task to start, which is what
/// makes the sample the next answer is worked out from.
pub fn room_for(left: u64, cost: Option<Cost>, running: usize) -> usize {
    if left == 0 {
        return 0;
    }
    let fits = match cost.filter(|cost| cost.total > 0 && cost.over > 0) {
        Some(cost) => (left.saturating_mul(cost.over) / cost.total).min(AT_ONCE as u64) as usize,
        None => 1,
    };
    fits.saturating_sub(running)
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
    use super::*;
    use crate::core::domain::Span;

    fn declaring(usage: Usage) -> Budget {
        Budget {
            usage,
            time: Span::parse("8h").unwrap(),
        }
    }

    #[test]
    fn what_is_left_is_what_was_declared_less_what_was_spent() {
        let budget = declaring(Usage::Tokens(1_000));
        assert_eq!(budget.left(Spending::Tokens(400)), 600);
    }

    #[test]
    fn a_share_is_left_over_in_the_same_points_it_was_declared_in() {
        let budget = declaring(Usage::Share(50));
        assert_eq!(budget.left(Spending::Share(20)), 30);
    }

    /// A session can pass its budget between two decisions, since a decision
    /// is made when a task ends and not while one runs.
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

    fn over(total: u64, over: u64) -> Option<Cost> {
        Some(Cost { total, over })
    }

    #[test]
    fn with_nothing_to_go_on_one_task_starts() {
        assert_eq!(room_for(1_000_000, None, 0), 1);
    }

    #[test]
    fn what_is_left_divided_by_what_one_costs_is_how_many_start() {
        assert_eq!(room_for(300, over(100, 1), 0), 3);
        assert_eq!(room_for(300, over(300, 3), 0), 3);
    }

    /// Two tasks that moved the vendor's limit one point cost half a point
    /// each. Working out that half first and then dividing by it loses the
    /// whole answer, so a session declared as a share would run one task at a
    /// time forever.
    #[test]
    fn a_cost_smaller_than_one_still_says_how_many_fit() {
        // One point left, half a point each.
        assert_eq!(room_for(1, over(1, 2), 0), 2);
        assert_eq!(room_for(49, over(1, 2), 0), AT_ONCE);
    }

    #[test]
    fn what_is_already_running_counts_against_that() {
        assert_eq!(room_for(300, over(100, 1), 2), 1);
        assert_eq!(room_for(300, over(100, 1), 3), 0);
    }

    /// A large budget divides into more tasks than a machine should run.
    #[test]
    fn no_more_than_a_handful_start_however_large_the_budget() {
        assert_eq!(room_for(1_000_000, over(1, 1), 0), AT_ONCE);
    }

    #[test]
    fn nothing_starts_once_the_budget_is_spent() {
        assert_eq!(room_for(0, over(100, 1), 0), 0);
        assert_eq!(room_for(0, None, 0), 0);
    }

    #[test]
    fn nothing_starts_when_one_task_costs_more_than_is_left() {
        assert_eq!(room_for(50, over(100, 1), 0), 0);
    }

    /// A set that has cost nothing at all has not run yet.
    #[test]
    fn a_set_that_cost_nothing_starts_one_at_a_time() {
        assert_eq!(room_for(50, over(0, 2), 0), 1);
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
