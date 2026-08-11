//! What time it is, as the core asks for it.

/// The reading a session's time limit is measured against.
///
/// A port rather than a call into the standard library.
/// A test can decide that eight hours have passed without waiting for them.
pub trait Clock: Sync {
    /// Seconds since the epoch.
    ///
    /// A count rather than a moment.
    /// The only thing done with it is subtracting one reading from another, and a store keeps it as a number.
    fn now(&self) -> u64;
}
