//! The `--version` flag.
//!
//! A daemon outlives the install that replaced it, so a new command can be
//! talking to an old core. Printing both versions is how that shows.

use std::process::ExitCode;

use cistern_contract::{Response, VERSION, code::CORE_ERROR, exchange};

/// What came back when the core was asked for its version.
#[derive(Debug, PartialEq, Eq)]
enum Core {
    /// The version it answered with.
    Version(String),
    /// Nothing was listening.
    NotRunning(String),
    /// Something answered, but not with a version.
    Unusable(String),
}

/// Reads an answer for what it means. It takes the answer rather than fetching
/// one, so that every outcome can be tested without a core to talk to.
fn interpret(response: std::io::Result<Response>) -> Core {
    let response = match response {
        Ok(response) => response,
        Err(e) => return Core::NotRunning(e.to_string()),
    };

    let answer = match response {
        Response::Data(answer) => answer,
        Response::Error(failure) => return Core::Unusable(failure.message),
    };

    match answer.data.get("version").and_then(|v| v.as_str()) {
        Some(core) => Core::Version(core.to_owned()),
        None => Core::Unusable("the core did not answer with a version".to_owned()),
    }
}

/// Prints what this surface is and what core it reached.
pub fn run() -> ExitCode {
    println!("cistern {VERSION}");

    match interpret(exchange::ask("core_version", serde_json::json!({}))) {
        Core::Version(core) if core == VERSION => succeed(&core),
        Core::Version(core) => fail(
            &core,
            &format!("the core is {core} and this is {VERSION}; restart the core"),
        ),
        Core::NotRunning(e) => fail("not running", &format!("the core is not running: {e}")),
        Core::Unusable(why) => fail("unavailable", &why),
    }
}

/// Shows what the core is and ends well.
fn succeed(shown: &str) -> ExitCode {
    println!("core    {shown}");
    ExitCode::SUCCESS
}

/// Shows what stood in for the core, says why, and ends badly.
fn fail(shown: &str, why: &str) -> ExitCode {
    println!("core    {shown}");
    eprintln!("cistern: {why}");
    ExitCode::from(CORE_ERROR)
}

#[cfg(test)]
mod tests {
    use std::io;

    use cistern_contract::{Answer, Failure, FailureTag};

    use super::*;

    fn answered(data: serde_json::Value) -> io::Result<Response> {
        Ok(Response::Data(Answer {
            command: "core_version".to_owned(),
            data,
        }))
    }

    #[test]
    fn a_version_comes_back_as_it_was_given() {
        assert_eq!(
            interpret(answered(serde_json::json!({ "version": "0.2.0" }))),
            Core::Version("0.2.0".to_owned())
        );
    }

    #[test]
    fn an_answer_without_a_version_is_unusable() {
        assert_eq!(
            interpret(answered(serde_json::json!({}))),
            Core::Unusable("the core did not answer with a version".to_owned())
        );
    }

    #[test]
    fn a_version_that_is_not_a_string_is_unusable() {
        assert_eq!(
            interpret(answered(serde_json::json!({ "version": 1 }))),
            Core::Unusable("the core did not answer with a version".to_owned())
        );
    }

    #[test]
    fn a_refusal_carries_its_own_message() {
        let refused = Ok(Response::Error(Failure {
            kind: FailureTag::Error,
            code: 3,
            message: "no such task".to_owned(),
            data: None,
        }));
        assert_eq!(
            interpret(refused),
            Core::Unusable("no such task".to_owned())
        );
    }

    #[test]
    fn nothing_listening_is_not_running() {
        let e = io::Error::new(io::ErrorKind::NotFound, "no such file");
        assert_eq!(
            interpret(Err(e)),
            Core::NotRunning("no such file".to_owned())
        );
    }
}
