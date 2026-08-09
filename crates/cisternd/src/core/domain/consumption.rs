//! What running a task consumed.
//!
//! Section 1 of `docs/cli.md` declares a budget in tokens, and a vendor reports
//! tokens in kinds that are not worth the same. The kinds are kept apart here
//! rather than added together, because which of them a budget counts is the
//! supervisor's to decide and nothing here should decide it early.

use std::{
    fmt::{self, Display},
    ops::Add,
};

/// What one run, or a set of runs, consumed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Consumption {
    /// Tokens read that were not already held.
    pub input: u64,
    /// Tokens written in the answer.
    pub output: u64,
    /// Tokens put aside to be read again cheaply.
    pub cache_written: u64,
    /// Tokens read from what was put aside.
    pub cache_read: u64,
    /// What the vendor priced this at, in millionths of its currency.
    ///
    /// A whole number rather than a fraction, because these are added up over a
    /// session and a fraction added a hundred times is no longer the figure it
    /// started as.
    pub cost: u64,
}

/// What is known about what a task consumed.
///
/// A task that has not run and a task whose answer could not be read are
/// different things, and neither is a task that consumed nothing. A vendor that
/// renames a field would otherwise report a full session as having spent
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// The task has not run.
    NotYet,
    /// The task ran and what it consumed could not be read.
    Unreadable { why: String },
    /// The task ran and this is what it consumed.
    Spent(Consumption),
}

impl Consumption {
    /// Every kind added together.
    ///
    /// One number for a set of runs, still keeping the kinds apart within it.
    pub fn total(counted: impl IntoIterator<Item = Consumption>) -> Consumption {
        counted.into_iter().fold(Consumption::default(), Add::add)
    }
}

impl Add for Consumption {
    type Output = Consumption;

    fn add(self, other: Consumption) -> Consumption {
        Consumption {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cache_written: self.cache_written.saturating_add(other.cache_written),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            cost: self.cost.saturating_add(other.cost),
        }
    }
}

impl Display for Consumption {
    /// The kinds in the order a vendor reports them, for a person reading a
    /// store by hand.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} in, {} out, {} written, {} read, {} millionths",
            self.input, self.output, self.cache_written, self.cache_read, self.cost
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spent(input: u64, output: u64) -> Consumption {
        Consumption {
            input,
            output,
            cache_written: 1,
            cache_read: 2,
            cost: 3,
        }
    }

    #[test]
    fn adding_keeps_every_kind_apart() {
        let both = spent(10, 20) + spent(1, 2);

        assert_eq!(both.input, 11);
        assert_eq!(both.output, 22);
        assert_eq!(both.cache_written, 2);
        assert_eq!(both.cache_read, 4);
        assert_eq!(both.cost, 6);
    }

    #[test]
    fn nothing_added_to_something_leaves_it_as_it_was() {
        assert_eq!(spent(10, 20) + Consumption::default(), spent(10, 20));
    }

    #[test]
    fn a_set_of_runs_adds_up_to_one_figure() {
        let counted = Consumption::total([spent(1, 1), spent(2, 2), spent(3, 3)]);

        assert_eq!(counted.input, 6);
        assert_eq!(counted.cost, 9);
    }

    #[test]
    fn nothing_at_all_is_a_set_of_no_runs() {
        assert_eq!(Consumption::total([]), Consumption::default());
    }

    /// A session runs for hours and nobody should have to think about what
    /// happens at the top of a counter.
    #[test]
    fn a_count_that_would_pass_the_top_stops_there() {
        let most = Consumption {
            input: u64::MAX,
            ..Consumption::default()
        };

        assert_eq!((most + spent(1, 1)).input, u64::MAX);
    }
}
