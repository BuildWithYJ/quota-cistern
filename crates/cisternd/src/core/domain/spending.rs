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
    /// The figure itself, where it is in the unit a budget was declared in.
    ///
    /// Nothing where it is not. A session is measured in the unit it was declared in, and
    /// whoever read the spending read it for this session, so the other pairing is a figure
    /// about something else. Nothing left is what a session in that state is given, which
    /// stops it rather than measuring it against a figure that is not its own.
    pub fn against(&self, usage: Usage) -> Option<u64> {
        match (usage, self) {
            (Usage::Share(_), Spending::Share(figure))
            | (Usage::Tokens(_), Spending::Tokens(figure)) => Some(*figure),
            _ => None,
        }
    }

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
    /// What was declared, in the unit it was declared in.
    ///
    /// A share as hundredths of a percent and a count as the count, which is the unit
    /// everything measured against it is kept in.
    pub fn declared(&self) -> u64 {
        match self.usage {
            Usage::Share(declared) => u64::from(declared) * HUNDREDTHS,
            Usage::Tokens(declared) => declared,
        }
    }
}

#[cfg(test)]
mod tests;
