//! How a session is run, as one value, and how far each figure in it is stood on.
//!
//! Apart from the rules that read them, because they are a different kind of thing: a rule
//! follows from the specification and these are figures somebody chose. Each sits with the rule
//! that reads it -- the four a size is worked out by are `sizing`'s -- and this composes them,
//! so that a session can be run under one value and say afterwards which one it was.
//!
//! What each rests on, in one place, so that a figure nothing has measured is not mistaken for
//! one that has been:
//!
//! ```text
//! busy    the third quarter        chosen. Going over eats what others were counting on
//! alone   the first quarter        chosen. There is nobody to take from
//! lift    twice where it stopped   read across from backfilling schedulers, not measured here
//! widen   one over the count       read across from a scaler, and no test has shown it doing
//!                                  anything
//! timing  hold a run back or not   nothing. No real session has been held back by it, which
//!                                  is why `config set timing` can turn it off
//! pacing  hold a run back the      the sweep, which is what a harness may answer: with it off,
//!         budget will not outlast  a third of sessions of one shape spend past what they
//!                                  declared, and no other figure here changes that. What it
//!                                  costs differs by shape, so it ships off
//! locking end runs at the line     none yet. The sweep does not reach the state it acts in,
//!         or let them finish       so it ships as what a session already did
//! ```
//!
//! A figure that rests on nothing is not left as the only way a session can be run. `config
//! set` opens the ones a person may choose, and a sweep varies the rest in a loop rather than
//! a build each.

use std::fmt::{self, Display};

use super::Rule;

/// What a session does about a run the budget will not outlast.
///
/// A run still going when the budget runs out is ended, and what it did since its last commit
/// buys nothing. One ending that way is a session spending what it declared; four is the same
/// figure spent and four results lost, so what this decides is really how many runs a session
/// puts in front of a budget it is close to finishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacing {
    /// Hold it back. What is left goes to a run that can finish inside it.
    Holds,
    /// Start it anyway, and let the budget end where it ends.
    Any,
}

impl Pacing {
    /// Reads what `config set pacing` was given.
    pub fn parse(pacing: &str) -> Option<Self> {
        match pacing {
            "holds" => Some(Pacing::Holds),
            "any" => Some(Pacing::Any),
            _ => None,
        }
    }
}

impl Display for Pacing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Pacing::Holds => "holds",
            Pacing::Any => "any",
        })
    }
}

/// What a session does when it has spent what it declared and runs are still going.
///
/// Section 2.2 of `docs/cli.md` says a session stops at whichever runs out first. What it did
/// not say is whether stopping ends a run that is going. Both answers cost something: cutting
/// them loses what they spent up to their last commit, and letting them finish spends past the
/// figure a person declared, by however much they had left to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locking {
    /// End them. The declared figure is a line, and a session does not cross it.
    Cuts,
    /// Let them finish. What they spend past the line is spent, and nothing they did is lost.
    Waits,
}

impl Locking {
    /// Reads what `config set locking` was given.
    pub fn parse(locking: &str) -> Option<Self> {
        match locking {
            "cuts" => Some(Locking::Cuts),
            "waits" => Some(Locking::Waits),
            _ => None,
        }
    }
}

impl Display for Locking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Locking::Cuts => "cuts",
            Locking::Waits => "waits",
        })
    }
}

/// How a session is run. One value, swapped whole.
///
/// Every figure a decision turns on is here rather than written into the code that reads it,
/// so comparing two ways of running a session is a loop rather than a build each, and a
/// session can say afterwards which one it ran under. What ships is [`Policy::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// The figures a size is worked out by, which `sizing` reads and nothing else does.
    pub sizing: Rule,
    /// Whether a run is held back for the clock, which the decision reads and nothing else
    /// does.
    pub timing: Timing,
    /// What a session does about runs still going once it has spent what it declared.
    pub locking: Locking,
    /// What a session does about a run the budget will not outlast.
    pub pacing: Pacing,
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
            sizing: Rule::default(),
            timing: Timing::Fits,
            locking: Locking::Waits,
            pacing: Pacing::Any,
        }
    }
}
