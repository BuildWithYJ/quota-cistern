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

/// How many more tasks may be started.
///
/// `each` is what one task has cost so far, in the same unit as `left`.
/// Without it, one. A task that has not run cannot be costed, and starting
/// several on a guess is how a budget is passed before anything is learned.
/// A cost of zero is the same as no cost at all: a share moves in whole
/// percentage points, so a task too small to move it says nothing about how
/// many of them fit.
pub fn room_for(left: u64, each: Option<u64>, running: usize) -> usize {
    if left == 0 {
        return 0;
    }
    let fits = match each.filter(|&each| each > 0) {
        Some(each) => (left / each).min(AT_ONCE as u64) as usize,
        None => 1,
    };
    fits.saturating_sub(running)
}

/// What one task has cost, averaged over the ones that reported a cost.
///
/// Nothing is answered for a set that reported nothing, which is the first
/// task of the first session there has ever been.
pub fn each_of(costs: impl IntoIterator<Item = u64>) -> Option<u64> {
    let (total, counted) = costs
        .into_iter()
        .fold((0u64, 0u64), |(total, counted), one| {
            (total.saturating_add(one), counted + 1)
        });
    (counted > 0).then(|| total / counted)
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

    #[test]
    fn with_nothing_to_go_on_one_task_starts() {
        assert_eq!(room_for(1_000_000, None, 0), 1);
    }

    #[test]
    fn what_is_left_divided_by_what_one_costs_is_how_many_start() {
        assert_eq!(room_for(300, Some(100), 0), 3);
    }

    #[test]
    fn what_is_already_running_counts_against_that() {
        assert_eq!(room_for(300, Some(100), 2), 1);
        assert_eq!(room_for(300, Some(100), 3), 0);
    }

    /// A large budget divides into more tasks than a machine should run.
    #[test]
    fn no_more_than_a_handful_start_however_large_the_budget() {
        assert_eq!(room_for(1_000_000, Some(1), 0), AT_ONCE);
    }

    #[test]
    fn nothing_starts_once_the_budget_is_spent() {
        assert_eq!(room_for(0, Some(100), 0), 0);
        assert_eq!(room_for(0, None, 0), 0);
    }

    #[test]
    fn nothing_starts_when_one_task_costs_more_than_is_left() {
        assert_eq!(room_for(50, Some(100), 0), 0);
    }

    /// A share moves in whole points, so a task smaller than one point reads
    /// as costing nothing. That says how small it is, not how many fit.
    #[test]
    fn a_cost_too_small_to_measure_starts_one_at_a_time() {
        assert_eq!(room_for(50, Some(0), 0), 1);
    }

    #[test]
    fn what_one_task_costs_is_the_average_of_what_they_did() {
        assert_eq!(each_of([100, 200, 300]), Some(200));
    }

    #[test]
    fn a_set_that_reported_nothing_says_nothing() {
        assert_eq!(each_of([]), None);
    }
}
