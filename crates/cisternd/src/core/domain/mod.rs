//! The entities and the rules over them.
//!
//! One file per concept.
//! The module is private, so what a file inside declares public is still out of reach from outside `core`.

mod configuration;
mod consumption;
mod session;
mod supervision;
mod task;

pub use configuration::{Configuration, Key, Known, Setting};
pub use consumption::{Consumption, Observation};
pub use session::{
    Budget, Held, NotASessionSet, NotOpened, Opening, Session, SessionId, SessionState, Sessions,
    Span, StoppedReason, Usage,
};
pub use supervision::{Cost, Decision, HUNDREDTHS, Spending, Standing, cost_of, decide};
pub use task::{
    Backlog, DisposalRefused, Disposition, NotABacklog, RemovalRefused, Repository, Restored, Task,
    TaskId, TaskState,
};
