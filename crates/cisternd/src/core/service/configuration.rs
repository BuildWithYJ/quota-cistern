//! What `config set` and `config get` do.

use crate::core::{
    Applied, Refusal, View,
    domain::{Configuration, Key, Setting},
    port::outbound::{ConfigurationStore, StoredConfiguration},
};

/// Stores one setting.
///
/// The value is read before the store is, so a value that was never valid
/// cannot leave a half-written configuration behind.
pub fn set(settings: &impl ConfigurationStore, key: &str, value: &str) -> Result<Applied, Refusal> {
    let Some(parsed) = Key::parse(key) else {
        return Err(Refusal::UnknownKey {
            key: key.to_owned(),
        });
    };
    let Some(setting) = Setting::parse(parsed, value) else {
        return Err(Refusal::BadValue {
            key: parsed.to_string(),
            value: value.to_owned(),
        });
    };

    let mut configuration = read(settings)?;
    configuration.apply(setting);
    settings.store(&written(&configuration))?;

    Ok(Applied {
        key: parsed.to_string(),
        value: value.to_owned(),
    })
}

/// Reads one key, or the whole configuration when no key is named.
pub fn get(settings: &impl ConfigurationStore, key: Option<&str>) -> Result<View, Refusal> {
    let Some(key) = key else {
        let entries = read(settings)?
            .entries()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
        return Ok(View::All { entries });
    };

    let Some(parsed) = Key::parse(key) else {
        return Err(Refusal::UnknownKey {
            key: key.to_owned(),
        });
    };
    Ok(View::One {
        key: parsed.to_string(),
        value: read(settings)?.value_of(parsed),
    })
}

/// Reads the store and holds it to the same standard as an argument.
///
/// A configuration file can be edited by hand, so what a store hands back is a
/// claim rather than a fact. The domain is given values it can take, never the
/// text they were kept as, so reading them is this layer's work.
fn read(settings: &impl ConfigurationStore) -> Result<Configuration, Refusal> {
    let stored = settings.load()?;
    let held = [
        (Key::Vendor, stored.vendor),
        (Key::Plan, stored.plan),
        (Key::UsageLimit, stored.usage_limit),
    ];

    let mut configuration = Configuration::default();
    for (key, value) in held {
        let Some(value) = value else { continue };
        let Some(setting) = Setting::parse(key, &value) else {
            return Err(Refusal::BadValue {
                key: key.to_string(),
                value,
            });
        };
        configuration.apply(setting);
    }
    Ok(configuration)
}

/// Hands the configuration to a store as the text a user would have typed.
fn written(configuration: &Configuration) -> StoredConfiguration {
    StoredConfiguration {
        vendor: configuration.value_of(Key::Vendor),
        plan: configuration.value_of(Key::Plan),
        usage_limit: configuration.value_of(Key::UsageLimit),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::core::port::outbound::Unavailable;

    use super::*;

    /// A store held in memory, so the steps can be checked without a file.
    #[derive(Default)]
    struct Remembered {
        stored: RefCell<StoredConfiguration>,
        /// Makes every read fail, standing in for a store that is there but
        /// cannot be understood.
        broken: bool,
    }

    impl Remembered {
        fn holding(stored: StoredConfiguration) -> Self {
            Remembered {
                stored: RefCell::new(stored),
                broken: false,
            }
        }
    }

    impl ConfigurationStore for Remembered {
        fn load(&self) -> Result<StoredConfiguration, Unavailable> {
            match self.broken {
                true => Err(Unavailable::new("not valid TOML")),
                false => Ok(self.stored.borrow().clone()),
            }
        }

        fn store(&self, stored: &StoredConfiguration) -> Result<(), Unavailable> {
            *self.stored.borrow_mut() = stored.clone();
            Ok(())
        }
    }

    #[test]
    fn what_was_set_comes_back() {
        let settings = Remembered::default();
        set(&settings, "vendor", "claude").unwrap();
        assert_eq!(
            get(&settings, Some("vendor")),
            Ok(View::One {
                key: "vendor".to_owned(),
                value: Some("claude".to_owned())
            })
        );
    }

    #[test]
    fn setting_one_key_leaves_the_others_alone() {
        let settings = Remembered::default();
        set(&settings, "vendor", "claude").unwrap();
        set(&settings, "plan", "max-20x").unwrap();
        assert_eq!(
            get(&settings, None),
            Ok(View::All {
                entries: vec![
                    ("vendor".to_owned(), "claude".to_owned()),
                    ("plan".to_owned(), "max-20x".to_owned()),
                ]
            })
        );
    }

    #[test]
    fn an_unknown_key_is_refused_by_name() {
        let settings = Remembered::default();
        assert_eq!(
            set(&settings, "colour", "red"),
            Err(Refusal::UnknownKey {
                key: "colour".to_owned()
            })
        );
    }

    #[test]
    fn a_value_the_key_does_not_take_is_refused_with_both() {
        let settings = Remembered::default();
        assert_eq!(
            set(&settings, "plan", "max-40x"),
            Err(Refusal::BadValue {
                key: "plan".to_owned(),
                value: "max-40x".to_owned()
            })
        );
    }

    #[test]
    fn a_refused_value_stores_nothing() {
        let settings = Remembered::default();
        set(&settings, "plan", "max-40x").ok();
        assert_eq!(get(&settings, None), Ok(View::All { entries: vec![] }));
    }

    #[test]
    fn reading_a_key_nobody_set_is_not_a_refusal() {
        let settings = Remembered::default();
        assert_eq!(
            get(&settings, Some("plan")),
            Ok(View::One {
                key: "plan".to_owned(),
                value: None
            })
        );
    }

    #[test]
    fn a_store_that_cannot_be_read_stops_a_write() {
        let settings = Remembered {
            broken: true,
            ..Default::default()
        };
        assert!(matches!(
            set(&settings, "vendor", "claude"),
            Err(Refusal::Unavailable { .. })
        ));
    }

    #[test]
    fn what_goes_to_a_store_comes_back_the_same() {
        let settings = Remembered::default();
        set(&settings, "vendor", "claude").unwrap();
        set(&settings, "plan", "max-20x").unwrap();
        set(&settings, "usage-limit", "2000000").unwrap();

        // A second reader over the same store is what a restarted core is.
        let restarted = Remembered::holding(settings.stored.borrow().clone());
        assert_eq!(get(&restarted, None), get(&settings, None));
    }

    /// A store hands over text whatever it kept the value as, so a number that
    /// is not one this key takes is refused where every other value is.
    #[test]
    fn a_stored_value_of_another_type_is_refused_the_same_way() {
        let settings = Remembered::holding(StoredConfiguration {
            usage_limit: Some("-1".to_owned()),
            ..Default::default()
        });
        assert_eq!(
            get(&settings, None),
            Err(Refusal::BadValue {
                key: "usage-limit".to_owned(),
                value: "-1".to_owned()
            })
        );
    }

    #[test]
    fn a_value_edited_into_the_store_by_hand_is_refused_too() {
        let settings = Remembered::holding(StoredConfiguration {
            plan: Some("max-40x".to_owned()),
            ..Default::default()
        });
        assert_eq!(
            get(&settings, None),
            Err(Refusal::BadValue {
                key: "plan".to_owned(),
                value: "max-40x".to_owned()
            })
        );
    }
}
