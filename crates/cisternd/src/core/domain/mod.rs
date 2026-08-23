//! The entities and the rules over them.
//!
//! One file per concept.
//! The module is private, so what a file inside declares public is still out of reach from outside `core`.

mod configuration;
mod consumption;
mod policy;
mod session;
mod sizing;
mod spending;
mod supervision;
mod task;

pub use configuration::{Configuration, Key, Known, Setting};
pub use consumption::{Consumption, Observation};
pub use policy::{Policy, Timing};
pub use session::{
    Budget, Held, NotASessionSet, NotOpened, Opening, Session, SessionId, SessionState, Sessions,
    Span, StoppedReason, Usage,
};
pub use sizing::{Before, Priced, Rule, Sizings, moved_per_millionth, sampled};
pub use spending::{HUNDREDTHS, Spending};
pub use supervision::{Decision, Standing, decide, done_waiting, nothing_more};
pub use task::{
    AT_CEILING, Backlog, DisposalRefused, Disposition, NotABacklog, RemovalRefused, Repository,
    Restored, Task, TaskId, TaskState,
};
