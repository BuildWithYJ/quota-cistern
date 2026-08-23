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
//! ```
//!
//! A figure that rests on nothing is not left as the only way a session can be run. `config
//! set` opens the ones a person may choose, and a sweep varies the rest in a loop rather than
//! a build each.

use std::fmt::{self, Display};

use super::Rule;

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
        }
    }
}
