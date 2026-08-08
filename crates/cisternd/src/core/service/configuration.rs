//! What `config set` and `config get` do.

use crate::core::{
    Applied, Refusal, View,
    domain::{Configuration, Key, Setting},
    port::Settings,
};

/// Stores one setting.
///
/// The value is read before the store is, so a value that was never valid
/// cannot leave a half-written configuration behind.
pub fn set(settings: &impl Settings, key: &str, value: &str) -> Result<Applied, Refusal> {
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
    settings.store(&configuration.to_stored())?;

    Ok(Applied {
        key: parsed.to_string(),
        value: value.to_owned(),
    })
}

/// Reads one key, or the whole configuration when no key is named.
pub fn get(settings: &impl Settings, key: Option<&str>) -> Result<View, Refusal> {
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
/// claim rather than a fact.
fn read(settings: &impl Settings) -> Result<Configuration, Refusal> {
    Configuration::from_stored(settings.load()?).map_err(|e| Refusal::BadValue {
        key: e.key.to_string(),
        value: e.value,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::core::port::{Stored, Unavailable};

    use super::*;

    /// A store held in memory, so the steps can be checked without a file.
    #[derive(Default)]
    struct Remembered {
        stored: RefCell<Stored>,
        /// Makes every read fail, standing in for a store that is there but
        /// cannot be understood.
        broken: bool,
    }

    impl Remembered {
        fn holding(stored: Stored) -> Self {
            Remembered {
                stored: RefCell::new(stored),
                broken: false,
            }
        }
    }

    impl Settings for Remembered {
        fn load(&self) -> Result<Stored, Unavailable> {
            match self.broken {
                true => Err(Unavailable::new("not valid TOML")),
                false => Ok(self.stored.borrow().clone()),
            }
        }

        fn store(&self, stored: &Stored) -> Result<(), Unavailable> {
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
    fn a_value_edited_into_the_store_by_hand_is_refused_too() {
        let settings = Remembered::holding(Stored {
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
