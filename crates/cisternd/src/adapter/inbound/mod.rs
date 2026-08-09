//! Where the envelope meets the commands the core offers.
//!
//! One file per command group, each owning the names its commands arrive
//! under. Which group a request belongs to is settled where the services are
//! built, so no file here reaches another beside it.
//!
//! What every group shares is here: reading a field out of the envelope, and
//! turning a refusal into the code and sentence a surface exits with.

pub mod backlog;
pub mod configuration;
pub mod execution;

use cistern_contract::{
    Answer, Failure, Request, Response,
    code::{CORE_ERROR, NOT_FOUND, STATE_CONFLICT, USAGE_ERROR},
};
use serde_json::Value;

use crate::core::port::inbound::Refusal;

/// A command no group claimed.
pub fn unknown(request: Request) -> Response {
    Response::Error(Failure::new(
        USAGE_ERROR,
        format!("unknown request type {}", request.command),
    ))
}

/// One field of the envelope, when it holds a string.
pub(super) fn text<'a>(request: &'a Request, field: &str) -> Option<&'a str> {
    request.params.get(field)?.as_str()
}

/// A request whose envelope does not carry what the command needs.
///
/// It never reaches the core, because what arrived is the envelope's business
/// and the envelope is read here.
pub(super) fn missing(why: &str) -> Response {
    Response::Error(Failure::new(USAGE_ERROR, why.to_owned()))
}

/// Puts what the core decided into the envelope it travels in.
pub(super) fn answer(command: String, outcome: Result<Value, Refusal>) -> Response {
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
        Refusal::NoSuchTask { .. } => NOT_FOUND,
        Refusal::NotPending { .. }
        | Refusal::NotARepository { .. }
        | Refusal::AlreadyRunning { .. }
        | Refusal::NoPlanConfigured => STATE_CONFLICT,
        Refusal::Unavailable { .. } => CORE_ERROR,
    }
}

fn message_for(refusal: &Refusal) -> String {
    match refusal {
        Refusal::UnknownKey { key } => format!("no such key {key}"),
        Refusal::BadValue { key, value } => format!("{key} does not take {value}"),
        Refusal::NoSuchTask { id } => format!("{id} does not exist"),
        Refusal::NotPending { id } => format!("{id} is not pending"),
        Refusal::NotARepository { at } => format!("{at} is not inside a repository"),
        Refusal::AlreadyRunning { id } => format!("{id} is already running"),
        Refusal::NoPlanConfigured => {
            "no plan is configured, so a share of one cannot be measured".to_owned()
        }
        Refusal::Unavailable { reason } => format!("the store cannot be read: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use cistern_contract::VERSION;

    use super::*;

    /// A request as a surface of this version sends one.
    pub(super) fn asked(command: &str, params: Value) -> Request {
        Request {
            version: VERSION.to_owned(),
            command: command.to_owned(),
            params,
        }
    }

    pub(super) fn failure(response: Response) -> Failure {
        match response {
            Response::Error(failure) => failure,
            Response::Data(answer) => panic!("expected a refusal, got {answer:?}"),
        }
    }

    pub(super) fn data(response: Response) -> Value {
        match response {
            Response::Data(answer) => answer.data,
            Response::Error(failure) => panic!("expected an answer, got {failure:?}"),
        }
    }

    #[test]
    fn a_command_no_group_claims_is_a_usage_error() {
        let refused = failure(unknown(asked("banana", Value::Null)));
        assert_eq!(refused.code, USAGE_ERROR);
        assert!(refused.message.contains("banana"), "{}", refused.message);
    }

    #[test]
    fn each_refusal_becomes_the_code_section_two_five_gives_it() {
        let codes = [
            (
                Refusal::UnknownKey {
                    key: "colour".to_owned(),
                },
                USAGE_ERROR,
            ),
            (
                Refusal::BadValue {
                    key: "plan".to_owned(),
                    value: "max-40x".to_owned(),
                },
                USAGE_ERROR,
            ),
            (
                Refusal::NoSuchTask {
                    id: "task:7".to_owned(),
                },
                NOT_FOUND,
            ),
            (
                Refusal::NotPending {
                    id: "task:1".to_owned(),
                },
                STATE_CONFLICT,
            ),
            (
                Refusal::NotARepository {
                    at: "/tmp".to_owned(),
                },
                STATE_CONFLICT,
            ),
            (
                Refusal::Unavailable {
                    reason: "not valid JSON".to_owned(),
                },
                CORE_ERROR,
            ),
        ];

        for (refusal, code) in codes {
            assert_eq!(code_for(&refusal), code, "{refusal:?}");
        }
    }
}
