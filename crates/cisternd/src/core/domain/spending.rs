//! What a session has consumed, in the unit it declared.
//!
//! Section 2.2 of `docs/cli.md` lets a budget be declared as a share of the vendor's limit or
//! as a count of tokens, and reports what was consumed in the unit it was declared in. Which
//! of the two a figure is stays with the figure, so the two can never be added together or
//! measured against one another by accident.

use std::fmt::{self, Display};

use super::{Budget, Usage};

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

#[cfg(test)]
mod tests;
