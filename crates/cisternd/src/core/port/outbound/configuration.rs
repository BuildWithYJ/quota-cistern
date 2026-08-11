//! The configuration store, as the core asks for it.

use super::Unavailable;

/// Where the configuration is kept between runs.
///
/// Every key crosses as the text a user would have typed, and a key nobody has heard of
/// crosses too.
/// Which keys exist is section 2.5's, so a store that refused one first would answer for a
/// file with a code the specification gives to something else.
pub trait ConfigurationStore: Sync {
    /// Every key the file holds, in the order it holds them.
    ///
    /// Nothing stored is an empty configuration rather than a failure.
    /// `config get` has to answer before anyone has set anything.
    fn load(&self) -> Result<Vec<(String, String)>, Unavailable>;

    /// Writes the whole configuration, so that merging one key into what is already there stays in the core.
    fn store(&self, stored: &[(String, String)]) -> Result<(), Unavailable>;
}
