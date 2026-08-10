//! What answers the store ports, through files.
//!
//! One file each, read and written whole. `kept` holds what all of them do
//! with a file; each of the rest holds its own format and its own fields.

pub mod backlog;
pub mod configuration;
mod kept;
pub mod session;
pub mod trace;
