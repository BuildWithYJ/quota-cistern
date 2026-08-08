//! What decides, and what it needs from outside.
//!
//! ADR 0002 records this arrangement. `domain` is private, so the types below
//! are what the rest of `cisternd` sees.

mod domain;
pub mod port;
pub mod service;

use port::Unavailable;

/// A setting that was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub key: String,
    pub value: String,
}

/// What was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// One key, holding nothing when nobody has set it.
    One { key: String, value: Option<String> },
    /// Every key that holds something.
    All { entries: Vec<(String, String)> },
}

/// Why the core would not do what was asked.
///
/// It names what was wrong and stops there. Which exit code that becomes is
/// the same question as which envelope carries it, and both belong to the
/// caller, so the codes in `docs/cli.md` do not reach in here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No such key in the specification.
    UnknownKey { key: String },
    /// A key that exists, holding a value it does not take.
    BadValue { key: String, value: String },
    /// The store could not be reached or could not be understood.
    Unavailable { reason: String },
}

impl From<Unavailable> for Refusal {
    fn from(e: Unavailable) -> Self {
        Refusal::Unavailable { reason: e.reason }
    }
}
