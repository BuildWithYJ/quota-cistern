//! The numbers a decision turns on, and how far each of them is stood on.
//!
//! Apart from the decision that reads them, because they are a different kind of thing: the
//! decision is a rule that follows from the specification, and these are figures somebody
//! chose. Each one carries what it rests on, so that a figure nothing has measured is not
//! mistaken for one that has been.
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
//! A figure that rests on nothing is not left as the only way a session can be run. That is
//! what `Policy` being one value is for: `config set` opens the ones a person may choose, and
//! a sweep varies the rest without a build each.

use std::fmt::{self, Display};

/// The quantile a run is sized at while others are going, in whole percent.
///
/// The third quarter. Others are going, so a run that goes over eats budget they were counting
/// on, and a size three runs in four come in under is far enough up to make that rare.
const BUSY: u64 = 75;
/// The quantile a run is sized at when nothing else is going, in whole percent.
///
/// The first quarter. With nothing else going there is nobody to take budget from, so a
/// session that would otherwise stop with budget in hand starts one more and is optimistic
/// about it. This is the only place a session is.
const ALONE: u64 = 25;
/// How far a stopped run lifts the estimate above where it was stopped.
///
/// Twice, which is how a backfilling scheduler grows a prediction its job has already outlived.
const LIFT: u64 = 2;
/// How far an estimate is widened for how little it was worked out from.
///
/// One, so an estimate from a single run allows twice it and one from four allows a quarter
/// more. What this should be is not something four sessions on a real repository could say:
/// none of their runs came within half of its ceiling, so any figure here would have ended
/// them the same way. It is a number to sweep rather than one to argue about.
const WIDEN: u64 = 1;
/// How a session is run. One value, swapped whole.
///
/// Every figure a decision turns on is here rather than written into the code that reads it,
/// so comparing two ways of running a session is a loop rather than a build each, and a
/// session can say afterwards which one it ran under. What ships is [`Policy::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Which quantile a run is sized at while others are going.
    pub busy: u64,
    /// Which quantile it is sized at when nothing else is going.
    pub alone: u64,
    /// How far the size is widened for how few runs it came from: `size x (1 + widen/over)`.
    pub widen: u64,
    /// How far a run that was stopped lifts the size above what it spent. Nothing leaves
    /// stopped runs out altogether.
    pub lift: u64,
    /// Whether a run is held back for the clock.
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
            busy: BUSY,
            alone: ALONE,
            widen: WIDEN,
            lift: LIFT,
            timing: Timing::Fits,
        }
    }
}
