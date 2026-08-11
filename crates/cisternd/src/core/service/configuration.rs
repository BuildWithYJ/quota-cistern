//! What `config set` and `config get` do.

use crate::core::{
    domain::{Configuration, Key, Known, Setting},
    port::{
        inbound::{Applied, ConfigurationUseCase, Refusal, View},
        outbound::ConfigurationStore,
    },
};

/// The commands over the configuration, and what they need from outside.
///
/// It holds the port these commands use and no others.
/// A command over the configuration cannot reach the backlog store through it.
pub struct ConfigurationService<'a> {
    store: &'a dyn ConfigurationStore,
    /// Which agent names this build can run. The core does not decide them.
    known: Known,
}

impl<'a> ConfigurationService<'a> {
    /// The names are text because the domain is private to the core.
    /// The composition root says which adapters it holds; reading them into a
    /// value is this layer's work, the same as any other stored text.
    pub fn new(store: &'a dyn ConfigurationStore, names: impl IntoIterator<Item = String>) -> Self {
        ConfigurationService {
            store,
            known: Known::of(names),
        }
    }
}

impl ConfigurationUseCase for ConfigurationService<'_> {
    /// The value is read before the store is.
    /// A value that was never valid cannot leave a half-written configuration behind.
    fn set(&self, key: &str, value: &str) -> Result<Applied, Refusal> {
        let Some(parsed) = Key::parse(key) else {
            return Err(Refusal::UnknownKey {
                key: key.to_owned(),
            });
        };
        let Some(setting) = Setting::parse(parsed, value, &self.known) else {
            return Err(Refusal::BadValue {
                key: parsed.to_string(),
                value: value.to_owned(),
            });
        };

        let mut configuration = read(self.store, &self.known)?;
        configuration.apply(setting);
        self.store.store(&written(&configuration))?;

        Ok(Applied {
            key: parsed.to_string(),
            value: value.to_owned(),
        })
    }

    fn get(&self, key: Option<&str>) -> Result<View, Refusal> {
        let Some(key) = key else {
            let entries = read(self.store, &self.known)?
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
            value: read(self.store, &self.known)?.value_of(parsed),
        })
    }
}

/// Reads the store and holds it to the same standard as an argument.
///
/// A configuration file can be edited by hand, so what a store hands back is a claim rather than a fact.
/// The domain is given values it can take, never the text they were kept as, so reading them is this layer's work.
fn read(settings: &dyn ConfigurationStore, known: &Known) -> Result<Configuration, Refusal> {
    let mut configuration = Configuration::default();
    for (key, value) in settings.load()? {
        // A file holding a key section 2.5 does not name is refused the way an argument
        // naming one is, since a person wrote both.
        let Some(parsed) = Key::parse(&key) else {
            return Err(Refusal::UnknownKey { key });
        };
        let Some(setting) = Setting::parse(parsed, &value, known) else {
            return Err(Refusal::BadValue {
                key: parsed.to_string(),
                value,
            });
        };
        configuration.apply(setting);
    }
    Ok(configuration)
}

/// Hands the configuration to a store as the text a user would have typed.
fn written(configuration: &Configuration) -> Vec<(String, String)> {
    configuration
        .entries()
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::core::port::outbound::Unavailable;

    use super::*;

    /// A build that runs one agent, which is what ships today.
    fn over(settings: &Remembered) -> ConfigurationService<'_> {
        ConfigurationService::new(settings, ["claude".to_owned()])
    }

    /// A store held in memory, so the steps can be checked without a file.
    #[derive(Default)]
    struct Remembered {
        stored: Mutex<Vec<(String, String)>>,
        /// Makes every read fail, standing in for a store that is there but cannot be understood.
        broken: bool,
    }

    impl Remembered {
        fn holding(stored: Vec<(String, String)>) -> Self {
            Remembered {
                stored: Mutex::new(stored),
                broken: false,
            }
        }
    }

    impl ConfigurationStore for Remembered {
        fn load(&self) -> Result<Vec<(String, String)>, Unavailable> {
            match self.broken {
                true => Err(Unavailable::new("not valid TOML")),
                false => Ok(self.stored.lock().unwrap().clone()),
            }
        }

        fn store(&self, stored: &[(String, String)]) -> Result<(), Unavailable> {
            *self.stored.lock().unwrap() = stored.to_vec();
            Ok(())
        }
    }

    #[test]
    fn what_was_set_comes_back() {
        let settings = Remembered::default();
        over(&settings).set("vendor", "claude").unwrap();
        assert_eq!(
            over(&settings).get(Some("vendor")),
            Ok(View::One {
                key: "vendor".to_owned(),
                value: Some("claude".to_owned())
            })
        );
    }

    #[test]
    fn an_unknown_key_is_refused_by_name() {
        let settings = Remembered::default();
        assert_eq!(
            over(&settings).set("colour", "red"),
            Err(Refusal::UnknownKey {
                key: "colour".to_owned()
            })
        );
    }

    #[test]
    fn a_value_the_key_does_not_take_is_refused_with_both() {
        let settings = Remembered::default();
        assert_eq!(
            over(&settings).set("vendor", "codex"),
            Err(Refusal::BadValue {
                key: "vendor".to_owned(),
                value: "codex".to_owned()
            })
        );
    }

    #[test]
    fn a_refused_value_stores_nothing() {
        let settings = Remembered::default();
        over(&settings).set("vendor", "codex").ok();
        assert_eq!(over(&settings).get(None), Ok(View::All { entries: vec![] }));
    }

    #[test]
    fn reading_a_key_nobody_set_is_not_a_refusal() {
        let settings = Remembered::default();
        assert_eq!(
            over(&settings).get(Some("vendor")),
            Ok(View::One {
                key: "vendor".to_owned(),
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
            over(&settings).set("vendor", "claude"),
            Err(Refusal::Unavailable { .. })
        ));
    }

    #[test]
    fn what_goes_to_a_store_comes_back_the_same() {
        let settings = Remembered::default();
        over(&settings).set("vendor", "claude").unwrap();

        // A second reader over the same store is what a restarted core is.
        let restarted = Remembered::holding(settings.stored.lock().unwrap().clone());
        assert_eq!(over(&restarted).get(None), over(&settings).get(None));
    }

    /// A store hands over text whatever it kept the value as.
    /// A number that is not one this key takes is refused where every other value is.
    #[test]
    fn a_stored_value_of_another_type_is_refused_the_same_way() {
        let settings = Remembered::holding(vec![("vendor".to_owned(), "-1".to_owned())]);
        assert_eq!(
            over(&settings).get(None),
            Err(Refusal::BadValue {
                key: "vendor".to_owned(),
                value: "-1".to_owned()
            })
        );
    }

    /// Section 2.5 gives an unknown key the code an argument gets, not a store's.
    #[test]
    fn a_key_edited_into_the_store_is_refused_as_an_unknown_key() {
        let settings = Remembered::holding(vec![("colour".to_owned(), "red".to_owned())]);
        assert_eq!(
            over(&settings).get(None),
            Err(Refusal::UnknownKey {
                key: "colour".to_owned()
            })
        );
    }

    #[test]
    fn a_value_edited_into_the_store_by_hand_is_refused_too() {
        let settings = Remembered::holding(vec![("vendor".to_owned(), "codex".to_owned())]);
        assert_eq!(
            over(&settings).get(None),
            Err(Refusal::BadValue {
                key: "vendor".to_owned(),
                value: "codex".to_owned()
            })
        );
    }
}
