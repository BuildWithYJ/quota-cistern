//! What answers a request.
//!
//! `docs/ipc.md` records the envelope and `docs/cli.md` the commands. Only
//! what is implemented appears here.

use cistern_contract::{
    Answer, Failure, Request, Response, VERSION,
    code::{CORE_ERROR, USAGE_ERROR},
};

/// Answers one request.
pub fn respond(request: Request) -> Response {
    match refuse_other_versions(&request) {
        Some(refusal) => refusal,
        None => dispatch(request),
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

fn dispatch(request: Request) -> Response {
    match request.command.as_str() {
        "core_version" => Response::Data(Answer {
            command: request.command,
            data: serde_json::json!({ "version": VERSION }),
        }),
        other => Response::Error(Failure::new(
            USAGE_ERROR,
            format!("unknown request type {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(version: &str, command: &str) -> Request {
        Request {
            version: version.to_owned(),
            command: command.to_owned(),
            params: serde_json::json!({}),
        }
    }

    fn failure(response: Response) -> Failure {
        match response {
            Response::Error(failure) => failure,
            Response::Data(answer) => panic!("expected a refusal, got {answer:?}"),
        }
    }

    #[test]
    fn the_core_answers_with_its_own_version() {
        let response = respond(asked(VERSION, "core_version"));
        let Response::Data(answer) = response else {
            panic!("expected an answer")
        };
        assert_eq!(answer.command, "core_version");
        assert_eq!(answer.data, serde_json::json!({ "version": VERSION }));
    }

    #[test]
    fn a_surface_of_another_version_can_still_ask_which_core_this_is() {
        let response = respond(asked("0.0.1", "core_version"));
        assert!(matches!(response, Response::Data(_)));
    }

    #[test]
    fn another_version_is_refused_with_both_versions() {
        let refusal = failure(respond(asked("0.0.1", "task_add")));
        assert_eq!(refusal.code, CORE_ERROR);
        assert_eq!(
            refusal.data,
            Some(serde_json::json!({ "core": VERSION, "surface": "0.0.1" }))
        );
    }

    #[test]
    fn a_command_the_core_does_not_know_is_a_usage_error() {
        let refusal = failure(respond(asked(VERSION, "banana")));
        assert_eq!(refusal.code, USAGE_ERROR);
        assert!(refusal.message.contains("banana"), "{}", refusal.message);
    }
}
