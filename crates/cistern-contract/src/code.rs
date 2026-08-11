//! The exit codes `docs/cli.md` defines.
//!
//! The core chooses one when it refuses, and the surface exits with it.
//! The meanings have to be the same on both sides.

/// The operation was refused.
pub const GENERAL_FAILURE: u8 = 1;
/// A bad argument or flag.
pub const USAGE_ERROR: u8 = 2;
/// No such session or task id.
pub const NOT_FOUND: u8 = 3;
/// The operation is not possible in the current state.
pub const STATE_CONFLICT: u8 = 4;
/// The core is not running, its version does not match, or it failed.
pub const CORE_ERROR: u8 = 5;
