//! The configuration and what a valid one is.
//!
//! Section 2.5 of `docs/cli.md` fixes the keys and the values each one takes.
//! This module is private, so a value that reached here was parsed on the way
//! in and no later step has to check it again.

use std::fmt::{self, Display};

use crate::core::port::Stored;

/// The agent to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Claude,
}

/// The subscription a percentage of usage is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Pro,
    Max5x,
    Max20x,
    /// The basis comes from `usage-limit` rather than from a preset.
    Custom,
}

/// A key, whether or not anything is stored under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Vendor,
    Plan,
    UsageLimit,
}

/// A key together with a value that key takes.
///
/// Building one is the only way to name a value, so a value is checked once,
/// where the string is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Vendor(Vendor),
    Plan(Plan),
    UsageLimit(u64),
}

/// What is stored.
///
/// Every field is optional because nothing is set until a user sets it, and
/// `config get` has to answer before any of it exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Configuration {
    vendor: Option<Vendor>,
    plan: Option<Plan>,
    usage_limit: Option<u64>,
}

/// A value a store handed back that no key takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAValue {
    pub key: Key,
    pub value: String,
}

impl Key {
    /// Reads a key name.
    pub fn parse(key: &str) -> Option<Self> {
        match key {
            "vendor" => Some(Key::Vendor),
            "plan" => Some(Key::Plan),
            "usage-limit" => Some(Key::UsageLimit),
            _ => None,
        }
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Key::Vendor => "vendor",
            Key::Plan => "plan",
            Key::UsageLimit => "usage-limit",
        })
    }
}

/// One spelling for what a user types, what a store holds, and what is
/// printed, so the three cannot drift apart.
impl Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Vendor::Claude => "claude",
        })
    }
}

impl Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Plan::Pro => "pro",
            Plan::Max5x => "max-5x",
            Plan::Max20x => "max-20x",
            Plan::Custom => "custom",
        })
    }
}

impl Setting {
    /// Reads a value against the key it was given for.
    ///
    /// The key has to be known first, so that a key nobody has heard of is
    /// told apart from a key holding a value it does not take.
    pub fn parse(key: Key, value: &str) -> Option<Self> {
        match key {
            Key::Vendor => match value {
                "claude" => Some(Setting::Vendor(Vendor::Claude)),
                _ => None,
            },
            Key::Plan => match value {
                "pro" => Some(Setting::Plan(Plan::Pro)),
                "max-5x" => Some(Setting::Plan(Plan::Max5x)),
                "max-20x" => Some(Setting::Plan(Plan::Max20x)),
                "custom" => Some(Setting::Plan(Plan::Custom)),
                _ => None,
            },
            // A limit of zero makes every percentage zero, which is not a
            // limit anyone can have meant.
            Key::UsageLimit => match value.parse::<u64>() {
                Ok(0) | Err(_) => None,
                Ok(limit) => Some(Setting::UsageLimit(limit)),
            },
        }
    }
}

impl Configuration {
    /// Reads what a store handed back.
    ///
    /// A store holds names, not entities, and a configuration file can be
    /// edited by hand, so what comes back is read the same way an argument is.
    pub fn from_stored(stored: Stored) -> Result<Self, NotAValue> {
        let mut configuration = Configuration::default();
        let named = [
            (Key::Vendor, stored.vendor),
            (Key::Plan, stored.plan),
            (
                Key::UsageLimit,
                stored.usage_limit.map(|limit| limit.to_string()),
            ),
        ];

        for (key, value) in named {
            let Some(value) = value else { continue };
            match Setting::parse(key, &value) {
                Some(setting) => configuration.apply(setting),
                None => return Err(NotAValue { key, value }),
            }
        }
        Ok(configuration)
    }

    /// Hands the configuration to a store as names and numbers.
    pub fn to_stored(&self) -> Stored {
        Stored {
            vendor: self.vendor.map(|v| v.to_string()),
            plan: self.plan.map(|p| p.to_string()),
            usage_limit: self.usage_limit,
        }
    }

    /// Stores a setting, replacing whatever the key held.
    pub fn apply(&mut self, setting: Setting) {
        match setting {
            Setting::Vendor(vendor) => self.vendor = Some(vendor),
            Setting::Plan(plan) => self.plan = Some(plan),
            Setting::UsageLimit(limit) => self.usage_limit = Some(limit),
        }
    }

    /// What a key holds, spelled as it is written and printed.
    pub fn value_of(&self, key: Key) -> Option<String> {
        match key {
            Key::Vendor => self.vendor.map(|v| v.to_string()),
            Key::Plan => self.plan.map(|p| p.to_string()),
            Key::UsageLimit => self.usage_limit.map(|n| n.to_string()),
        }
    }

    /// Every key that holds something, in the order `docs/cli.md` lists them.
    pub fn entries(&self) -> Vec<(Key, String)> {
        [Key::Vendor, Key::Plan, Key::UsageLimit]
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
    fn every_plan_the_specification_lists_parses() {
        for plan in ["pro", "max-5x", "max-20x", "custom"] {
            assert!(setting("plan", plan).is_some(), "{plan}");
        }
    }

    #[test]
    fn a_vendor_we_do_not_run_is_refused() {
        assert_eq!(setting("vendor", "codex"), None);
    }

    #[test]
    fn a_usage_limit_is_a_number_above_zero() {
        assert_eq!(
            setting("usage-limit", "2000000"),
            Some(Setting::UsageLimit(2_000_000))
        );
        assert_eq!(setting("usage-limit", "0"), None);
        assert_eq!(setting("usage-limit", "-1"), None);
        assert_eq!(setting("usage-limit", "2M"), None);
    }

    #[test]
    fn setting_a_key_twice_keeps_the_second_value() {
        let mut config = Configuration::default();
        config.apply(Setting::Plan(Plan::Pro));
        config.apply(Setting::Plan(Plan::Max20x));
        assert_eq!(config.value_of(Key::Plan), Some("max-20x".to_owned()));
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

    #[test]
    fn what_goes_to_a_store_comes_back_the_same() {
        let mut config = Configuration::default();
        config.apply(Setting::Vendor(Vendor::Claude));
        config.apply(Setting::Plan(Plan::Max20x));
        config.apply(Setting::UsageLimit(2_000_000));
        assert_eq!(Configuration::from_stored(config.to_stored()), Ok(config));
    }

    #[test]
    fn a_stored_value_no_key_takes_is_refused_with_both() {
        let stored = Stored {
            plan: Some("max-40x".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            Configuration::from_stored(stored),
            Err(NotAValue {
                key: Key::Plan,
                value: "max-40x".to_owned()
            })
        );
    }

    #[test]
    fn an_empty_store_reads_as_a_configuration_nobody_has_set() {
        assert_eq!(
            Configuration::from_stored(Stored::default()),
            Ok(Configuration::default())
        );
    }
}
