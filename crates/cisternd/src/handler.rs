//! What answers a request.
//!
//! `docs/ipc.md` records the envelope and `docs/cli.md` the commands. Only
//! what is implemented appears here.

use cistern_contract::{
    Answer, Failure, Request, Response, VERSION,
    code::{CORE_ERROR, USAGE_ERROR},
};
use serde_json::Value;

use crate::core::{Applied, Refusal, View, port::Settings, service};

/// Answers one request.
pub fn respond(settings: &impl Settings, request: Request) -> Response {
    match refuse_other_versions(&request) {
        Some(refusal) => refusal,
        None => dispatch(settings, request),
    }
}

/// A daemon outlives the install that replaced it, so a surface can be newer
/// than the core it reached. `core_version` is exempt: it is how a surface
/// finds out which side is behind.
fn refuse_other_versions(request: &Request) -> Option<Response> {
    if request.command == "core_version" || request.version == VERSION {
        return None;
    }
    let message = format!("core is {VERSION}, surface is {}", request.version);
    let both = serde_json::json!({ "core": VERSION, "surface": request.version });
    Some(Response::Error(
        Failure::new(CORE_ERROR, message).with_data(both),
    ))
}

fn dispatch(settings: &impl Settings, request: Request) -> Response {
    match request.command.as_str() {
        "core_version" => Response::Data(Answer {
            command: request.command,
            data: serde_json::json!({ "version": VERSION }),
        }),
        "config_set" => {
            let outcome = match (text(&request, "key"), text(&request, "value")) {
                (Some(key), Some(value)) => service::set(settings, key, value).map(said),
                _ => return missing("config_set takes a key and a value, both strings"),
            };
            answer(request.command, outcome)
        }
        "config_get" => {
            // A key is optional here. Null is how a surface says it has none,
            // so only a key that is present and is neither is malformed.
            if request
                .params
                .get("key")
                .is_some_and(|k| !k.is_string() && !k.is_null())
            {
                return missing("the key config_get was given is not a string");
            }
            let outcome = service::get(settings, text(&request, "key")).map(shown);
            answer(request.command, outcome)
        }
        other => Response::Error(Failure::new(
            USAGE_ERROR,
            format!("unknown request type {other}"),
        )),
    }
}

fn text<'a>(request: &'a Request, field: &str) -> Option<&'a str> {
    request.params.get(field)?.as_str()
}

/// A request whose envelope does not carry what the command needs.
///
/// It never reaches the core, because what arrived is the envelope's business
/// and the envelope is read here.
fn missing(why: &str) -> Response {
    Response::Error(Failure::new(USAGE_ERROR, why.to_owned()))
}

/// Puts what the core decided into the envelope it travels in.
fn answer(command: String, outcome: Result<Value, Refusal>) -> Response {
    match outcome {
        Ok(data) => Response::Data(Answer { command, data }),
        Err(refusal) => Response::Error(Failure::new(code_for(&refusal), message_for(&refusal))),
    }
}

/// Which exit code a refusal becomes.
///
/// The core says only what was wrong, so this is the one place the two meet.
fn code_for(refusal: &Refusal) -> u8 {
    match refusal {
        Refusal::UnknownKey { .. } | Refusal::BadValue { .. } => USAGE_ERROR,
        Refusal::Unavailable { .. } => CORE_ERROR,
    }
}

fn message_for(refusal: &Refusal) -> String {
    match refusal {
        Refusal::UnknownKey { key } => format!("no such key {key}"),
        Refusal::BadValue { key, value } => format!("{key} does not take {value}"),
        Refusal::Unavailable { reason } => format!("the configuration cannot be read: {reason}"),
    }
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

    use crate::core::port::{Stored, Unavailable};

    use super::*;

    #[derive(Default)]
    struct Remembered {
        stored: RefCell<Stored>,
    }

    impl Settings for Remembered {
        fn load(&self) -> Result<Stored, Unavailable> {
            Ok(self.stored.borrow().clone())
        }

        fn store(&self, stored: &Stored) -> Result<(), Unavailable> {
            *self.stored.borrow_mut() = stored.clone();
            Ok(())
        }
    }

    fn asked(command: &str, params: Value) -> Request {
        Request {
            version: VERSION.to_owned(),
            command: command.to_owned(),
            params,
        }
    }

    fn failure(response: Response) -> Failure {
        match response {
            Response::Error(failure) => failure,
            Response::Data(answer) => panic!("expected a refusal, got {answer:?}"),
        }
    }

    fn data(response: Response) -> Value {
        match response {
            Response::Data(answer) => answer.data,
            Response::Error(failure) => panic!("expected an answer, got {failure:?}"),
        }
    }

    #[test]
    fn the_core_answers_with_its_own_version() {
        let response = respond(&Remembered::default(), asked("core_version", Value::Null));
        assert_eq!(data(response), serde_json::json!({ "version": VERSION }));
    }

    #[test]
    fn a_surface_of_another_version_can_still_ask_which_core_this_is() {
        let mut request = asked("core_version", Value::Null);
        request.version = "0.0.1".to_owned();
        let response = respond(&Remembered::default(), request);
        assert!(matches!(response, Response::Data(_)));
    }

    #[test]
    fn another_version_is_refused_with_both_versions() {
        let mut request = asked("config_get", serde_json::json!({}));
        request.version = "0.0.1".to_owned();
        let refusal = failure(respond(&Remembered::default(), request));
        assert_eq!(refusal.code, CORE_ERROR);
        assert_eq!(
            refusal.data,
            Some(serde_json::json!({ "core": VERSION, "surface": "0.0.1" }))
        );
    }

    #[test]
    fn a_command_the_core_does_not_know_is_a_usage_error() {
        let refusal = failure(respond(
            &Remembered::default(),
            asked("banana", Value::Null),
        ));
        assert_eq!(refusal.code, USAGE_ERROR);
        assert!(refusal.message.contains("banana"), "{}", refusal.message);
    }

    #[test]
    fn setting_a_key_answers_with_what_was_stored() {
        let settings = Remembered::default();
        let response = respond(
            &settings,
            asked(
                "config_set",
                serde_json::json!({ "key": "plan", "value": "max-20x" }),
            ),
        );
        assert_eq!(
            data(response),
            serde_json::json!({ "key": "plan", "value": "max-20x" })
        );
    }

    #[test]
    fn reading_one_key_answers_with_its_value() {
        let settings = Remembered::default();
        respond(
            &settings,
            asked(
                "config_set",
                serde_json::json!({ "key": "vendor", "value": "claude" }),
            ),
        );
        let response = respond(
            &settings,
            asked("config_get", serde_json::json!({ "key": "vendor" })),
        );
        assert_eq!(
            data(response),
            serde_json::json!({ "key": "vendor", "value": "claude" })
        );
    }

    #[test]
    fn reading_with_no_key_answers_with_every_key_that_holds_something() {
        let settings = Remembered::default();
        respond(
            &settings,
            asked(
                "config_set",
                serde_json::json!({ "key": "plan", "value": "pro" }),
            ),
        );
        let response = respond(&settings, asked("config_get", serde_json::json!({})));
        assert_eq!(
            data(response),
            serde_json::json!({ "entries": [{ "key": "plan", "value": "pro" }] })
        );
    }

    /// A surface passes what it was given without checking it, so this is what
    /// reaching the core directly looks like.
    #[test]
    fn a_bad_value_sent_straight_to_the_core_is_refused() {
        let refusal = failure(respond(
            &Remembered::default(),
            asked(
                "config_set",
                serde_json::json!({ "key": "vendor", "value": "codex" }),
            ),
        ));
        assert_eq!(refusal.code, USAGE_ERROR);
        assert!(refusal.message.contains("codex"), "{}", refusal.message);
    }

    #[test]
    fn an_unknown_key_is_a_usage_error() {
        let refusal = failure(respond(
            &Remembered::default(),
            asked(
                "config_set",
                serde_json::json!({ "key": "colour", "value": "red" }),
            ),
        ));
        assert_eq!(refusal.code, USAGE_ERROR);
    }

    #[test]
    fn a_request_without_the_fields_the_command_needs_never_reaches_the_core() {
        let refusal = failure(respond(
            &Remembered::default(),
            asked("config_set", serde_json::json!({ "key": "plan" })),
        ));
        assert_eq!(refusal.code, USAGE_ERROR);
    }

    #[test]
    fn a_null_key_is_how_a_surface_says_it_has_none() {
        let settings = Remembered::default();
        let response = respond(
            &settings,
            asked("config_get", serde_json::json!({ "key": null })),
        );
        assert_eq!(data(response), serde_json::json!({ "entries": [] }));
    }

    #[test]
    fn a_key_that_is_not_a_string_is_told_from_no_key_at_all() {
        let refusal = failure(respond(
            &Remembered::default(),
            asked("config_get", serde_json::json!({ "key": 7 })),
        ));
        assert_eq!(refusal.code, USAGE_ERROR);
    }
}
