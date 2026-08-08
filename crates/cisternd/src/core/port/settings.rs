//! The configuration store, as the core asks for it.

use super::Unavailable;

/// The configuration as a store holds it: text, not entities.
///
/// A store cannot name an entity, and the core would not trust it to. Every
/// key crosses as the text a user would have typed for it, whatever a store
/// kept it as, so what comes back meets the same parse an argument does and a
/// file edited by hand is refused the same way a bad argument is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stored {
    pub vendor: Option<String>,
    pub plan: Option<String>,
    pub usage_limit: Option<String>,
}

/// Where the configuration is kept between runs.
pub trait Settings {
    /// Reads what is stored.
    ///
    /// Nothing stored is an empty configuration rather than a failure, since
    /// `config get` has to answer before anyone has set anything. Telling that
    /// apart from a store that cannot be read is the implementation's job.
    fn load(&self) -> Result<Stored, Unavailable>;

    /// Writes the whole configuration.
    ///
    /// The whole of it, so that merging one key into what is already there
    /// stays in the core and the implementation never learns the shape.
    fn store(&self, stored: &Stored) -> Result<(), Unavailable>;
}
