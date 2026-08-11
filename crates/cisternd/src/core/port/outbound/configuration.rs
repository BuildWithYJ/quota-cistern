//! The configuration store, as the core asks for it.

use super::Unavailable;

/// The configuration as a store holds it: text, not entities.
///
/// Every key crosses as the text a user would have typed.
/// A file edited by hand is refused the same way a bad argument is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredConfiguration {
    pub vendor: Option<String>,
}

/// Where the configuration is kept between runs.
pub trait ConfigurationStore: Sync {
    /// Reads what is stored.
    ///
    /// Nothing stored is an empty configuration rather than a failure.
    /// `config get` has to answer before anyone has set anything.
    fn load(&self) -> Result<StoredConfiguration, Unavailable>;

    /// Writes the whole configuration, so that merging one key into what is already there stays in the core.
    fn store(&self, stored: &StoredConfiguration) -> Result<(), Unavailable>;
}
