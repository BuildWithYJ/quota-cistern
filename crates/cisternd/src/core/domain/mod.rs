//! The entities and the rules over them.
//!
//! One file per concept. The module is private, so what a file inside declares
//! public is still out of reach from outside `core`.

mod configuration;
mod session;
mod task;

pub use configuration::{Configuration, Key, Setting};
pub use session::{
    Budget, Held, NotASessionSet, NotOpened, SessionId, SessionState, Sessions, Span,
    StoppedReason, Usage,
};
pub use task::{Backlog, NotABacklog, RemovalRefused, Repository, Restored, TaskId, TaskState};
