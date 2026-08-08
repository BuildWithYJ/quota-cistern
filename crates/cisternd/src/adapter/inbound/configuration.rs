//! The envelope for `config set` and `config get`.

use cistern_contract::{Request, Response};
use serde_json::Value;

use crate::core::port::inbound::{Applied, ConfigurationUseCase, View};

use super::{answer, missing, text};

/// Answers the commands this group owns.
///
/// A command it does not own comes back untouched, so that whoever is routing
/// can offer the request to the next group without keeping a list of names.
pub fn respond(
    configuration: &impl ConfigurationUseCase,
    request: Request,
) -> Result<Response, Request> {
    match request.command.as_str() {
        "config_set" => Ok(set(configuration, request)),
        "config_get" => Ok(get(configuration, request)),
        _ => Err(request),
    }
}

fn set(configuration: &impl ConfigurationUseCase, request: Request) -> Response {
    let outcome = match (text(&request, "key"), text(&request, "value")) {
        (Some(key), Some(value)) => configuration.set(key, value).map(said),
        _ => return missing("config_set takes a key and a value, both strings"),
    };
    answer(request.command, outcome)
}

fn get(configuration: &impl ConfigurationUseCase, request: Request) -> Response {
    // A key is optional here. Null is how a surface says it has none, so only a
    // key that is present and is neither is malformed.
    if request
        .params
        .get("key")
        .is_some_and(|key| !key.is_string() && !key.is_null())
    {
        return missing("the key config_get was given is not a string");
    }
    let outcome = configuration.get(text(&request, "key")).map(shown);
    answer(request.command, outcome)
}

fn said(applied: Applied) -> Value {
    serde_json::json!({ "key": applied.key, "value": applied.value })
}

fn shown(view: View) -> Value {
    match view {
        View::One { key, value } => serde_json::json!({ "key": key, "value": value }),
        View::All { entries } => {
            let entries: Vec<Value> = entries
                .into_iter()
                .map(|(key, value)| serde_json::json!({ "key": key, "value": value }))
                .collect();
            serde_json::json!({ "entries": entries })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cistern_contract::code::USAGE_ERROR;

    use super::super::tests::{asked, data, failure};
    use crate::core::port::inbound::Refusal;

    use super::*;

    /// Stands in for the core.
    ///
    /// It answers rather than deciding, and records what it was asked, so that
    /// what is checked here is the envelope and nothing else. No store is
    /// involved, because no command reaches one through this file.
    #[derive(Default)]
    struct Core {
        /// What every command ends in, when it is meant to end badly.
        refuses: Option<Refusal>,
        /// The key the last call was given, and whether one arrived at all.
        asked_for: RefCell<Option<Option<String>>>,
    }

    impl Core {
        fn refusing(refusal: Refusal) -> Self {
            Core {
                refuses: Some(refusal),
                asked_for: RefCell::new(None),
            }
        }

        fn reached(&self) -> bool {
            self.asked_for.borrow().is_some()
        }
    }

    impl ConfigurationUseCase for Core {
        fn set(&self, key: &str, value: &str) -> Result<Applied, Refusal> {
            *self.asked_for.borrow_mut() = Some(Some(key.to_owned()));
            match &self.refuses {
                Some(refusal) => Err(refusal.clone()),
                None => Ok(Applied {
                    key: key.to_owned(),
                    value: value.to_owned(),
                }),
            }
        }

        fn get(&self, key: Option<&str>) -> Result<View, Refusal> {
            *self.asked_for.borrow_mut() = Some(key.map(str::to_owned));
            match &self.refuses {
                Some(refusal) => Err(refusal.clone()),
                None => Ok(match key {
                    Some(key) => View::One {
                        key: key.to_owned(),
                        value: Some("held".to_owned()),
                    },
                    None => View::All {
                        entries: vec![("plan".to_owned(), "pro".to_owned())],
                    },
                }),
            }
        }
    }

    fn answered(core: &Core, command: &str, params: Value) -> Response {
        match respond(core, asked(command, params)) {
            Ok(response) => response,
            Err(request) => panic!("this group does not own {}", request.command),
        }
    }

    #[test]
    fn setting_a_key_answers_with_what_was_applied() {
        let core = Core::default();
        let response = answered(
            &core,
            "config_set",
            serde_json::json!({ "key": "plan", "value": "max-20x" }),
        );
        assert_eq!(
            data(response),
            serde_json::json!({ "key": "plan", "value": "max-20x" })
        );
    }

    #[test]
    fn reading_one_key_answers_with_its_value() {
        let core = Core::default();
        let response = answered(&core, "config_get", serde_json::json!({ "key": "vendor" }));
        assert_eq!(
            data(response),
            serde_json::json!({ "key": "vendor", "value": "held" })
        );
    }

    #[test]
    fn reading_with_no_key_answers_with_every_key_that_holds_something() {
        let core = Core::default();
        let response = answered(&core, "config_get", serde_json::json!({}));
        assert_eq!(
            data(response),
            serde_json::json!({ "entries": [{ "key": "plan", "value": "pro" }] })
        );
    }

    #[test]
    fn a_null_key_is_how_a_surface_says_it_has_none() {
        let core = Core::default();
        answered(&core, "config_get", serde_json::json!({ "key": null }));
        assert_eq!(*core.asked_for.borrow(), Some(None));
    }

    #[test]
    fn a_key_that_is_not_a_string_is_told_from_no_key_at_all() {
        let core = Core::default();
        let refused = failure(answered(
            &core,
            "config_get",
            serde_json::json!({ "key": 7 }),
        ));
        assert_eq!(refused.code, USAGE_ERROR);
        assert!(!core.reached());
    }

    #[test]
    fn a_request_without_the_fields_the_command_needs_never_reaches_the_core() {
        let core = Core::default();
        let refused = failure(answered(
            &core,
            "config_set",
            serde_json::json!({ "key": "plan" }),
        ));
        assert_eq!(refused.code, USAGE_ERROR);
        assert!(!core.reached());
    }

    #[test]
    fn a_refusal_carries_the_code_and_the_sentence_it_becomes() {
        let core = Core::refusing(Refusal::UnknownKey {
            key: "colour".to_owned(),
        });
        let refused = failure(answered(
            &core,
            "config_set",
            serde_json::json!({ "key": "colour", "value": "red" }),
        ));
        assert_eq!(refused.code, USAGE_ERROR);
        assert!(refused.message.contains("colour"), "{}", refused.message);
    }

    #[test]
    fn a_command_this_group_does_not_own_comes_back() {
        let core = Core::default();
        let handed_back = respond(&core, asked("task_add", Value::Null));
        assert!(matches!(handed_back, Err(request) if request.command == "task_add"));
    }
}
