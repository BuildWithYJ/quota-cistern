//! What the core offers over the configuration.
//!
//! Section 2.5 of `docs/cli.md` fixes the keys, the values each one takes, and what each command answers with.

use super::Refusal;

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

/// `config set` and `config get`.
pub trait ConfigurationUseCase {
    /// Stores one setting, replacing whatever the key held.
    fn set(&self, key: &str, value: &str) -> Result<Applied, Refusal>;

    /// Reads one key, or the whole configuration when no key is named.
    fn get(&self, key: Option<&str>) -> Result<View, Refusal>;
}
