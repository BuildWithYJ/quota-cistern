//! The configuration and what a valid one is.
//!
//! Section 2.5 of `docs/cli.md` fixes the keys and the values each one takes.
//! This module is private, so a value that reached here was parsed on the way in.
//! No later step has to check it again.

use std::fmt::{self, Display};

/// The agent to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Claude,
}

/// A key, whether or not anything is stored under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Vendor,
}

/// A key together with a value that key takes.
///
/// Building one is the only way to name a value, so a value is checked once, where the string is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Vendor(Vendor),
}

/// What is stored.
///
/// Every field is optional because nothing is set until a user sets it.
/// `config get` has to answer before any of it exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Configuration {
    vendor: Option<Vendor>,
}

impl Key {
    /// Reads a key name.
    pub fn parse(key: &str) -> Option<Self> {
        match key {
            "vendor" => Some(Key::Vendor),
            _ => None,
        }
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Key::Vendor => "vendor",
        })
    }
}

/// One spelling for what a user types, what a store holds, and what is printed, so the three cannot drift apart.
impl Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Vendor::Claude => "claude",
        })
    }
}

impl Setting {
    /// Reads a value against the key it was given for.
    ///
    /// The key has to be known first.
    /// A key nobody has heard of is told apart from a key holding a value it does not take.
    pub fn parse(key: Key, value: &str) -> Option<Self> {
        match key {
            Key::Vendor => match value {
                "claude" => Some(Setting::Vendor(Vendor::Claude)),
                _ => None,
            },
        }
    }
}

impl Configuration {
    /// Stores a setting, replacing whatever the key held.
    pub fn apply(&mut self, setting: Setting) {
        match setting {
            Setting::Vendor(vendor) => self.vendor = Some(vendor),
        }
    }

    /// What a key holds, spelled as it is written and printed.
    pub fn value_of(&self, key: Key) -> Option<String> {
        match key {
            Key::Vendor => self.vendor.map(|v| v.to_string()),
        }
    }

    /// Every key that holds something, in the order `docs/cli.md` lists them.
    pub fn entries(&self) -> Vec<(Key, String)> {
        [Key::Vendor]
            .into_iter()
            .filter_map(|key| self.value_of(key).map(|value| (key, value)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(key: &str, value: &str) -> Option<Setting> {
        Setting::parse(Key::parse(key)?, value)
    }

    #[test]
    fn a_key_outside_the_specification_is_not_a_key() {
        assert_eq!(Key::parse("colour"), None);
    }

    #[test]
    fn a_vendor_we_do_not_run_is_refused() {
        assert_eq!(setting("vendor", "codex"), None);
    }

    #[test]
    fn setting_a_key_twice_keeps_the_second_value() {
        let mut config = Configuration::default();
        config.apply(Setting::Vendor(Vendor::Claude));
        config.apply(Setting::Vendor(Vendor::Claude));
        assert_eq!(config.value_of(Key::Vendor), Some("claude".to_owned()));
    }

    #[test]
    fn a_key_nobody_set_holds_nothing() {
        assert_eq!(Configuration::default().value_of(Key::Vendor), None);
    }

    #[test]
    fn only_what_was_set_is_listed() {
        let mut config = Configuration::default();
        config.apply(Setting::Vendor(Vendor::Claude));
        assert_eq!(config.entries(), vec![(Key::Vendor, "claude".to_owned())]);
    }
}
