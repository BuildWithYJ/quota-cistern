//! The `--version` flag.
//!
//! A daemon outlives the install that replaced it, so a new command can be talking to an old core.
//! Printing both versions is how that shows.

use std::{process::ExitCode, time::SystemTime};

use cistern_contract::{Response, VERSION, address, code::CORE_ERROR, exchange};

use crate::daemon;

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

/// Reads an answer for what it means.
///
/// It takes the answer rather than fetching one, so that every outcome can be tested without a core to talk to.
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
///
/// A core of the same version may still be an older build of it, which the comparison above
/// cannot see. That is said here rather than made a failure: the core is answering and the two
/// sides agree on what they are, and what a running program was built from is not something
/// either of them can prove.
fn succeed(shown: &str) -> ExitCode {
    println!("core    {shown}");
    if outrun(address::bound_at().ok(), written_at(&daemon::program())) {
        eprintln!(
            "cistern: the core has been running since before {} was last written; restart it to run that one",
            daemon::CORE
        );
    }
    ExitCode::SUCCESS
}

/// When a file was last written, or nothing where there is no file to ask about.
fn written_at(at: &Option<std::path::PathBuf>) -> Option<SystemTime> {
    std::fs::metadata(at.as_ref()?).ok()?.modified().ok()
}

/// Whether the core now running started before the core program on disk was written.
///
/// The socket is made as a core starts and taken away as it ends, so it is as old as the core
/// holding it. A program written after that is one no running core has ever run. Neither
/// figure being available is not an answer, and no answer is not a warning.
fn outrun(bound_at: Option<SystemTime>, written_at: Option<SystemTime>) -> bool {
    match (bound_at, written_at) {
        (Some(bound_at), Some(written_at)) => written_at > bound_at,
        _ => false,
    }
}

/// Shows what stood in for the core, says why, and ends badly.
fn fail(shown: &str, why: &str) -> ExitCode {
    println!("core    {shown}");
    eprintln!("cistern: {why}");
    ExitCode::from(CORE_ERROR)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

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

    /// A core that has been running since before the program on disk was written is running
    /// something else, and both sides still answer with the same version.
    #[test]
    fn a_core_older_than_the_program_on_disk_is_said_to_be_outrun() {
        let bound_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let written_at = bound_at + Duration::from_secs(1);
        assert!(super::outrun(Some(bound_at), Some(written_at)));
    }

    /// The ordinary case: the core was started from the program that is there now.
    #[test]
    fn a_core_started_after_the_program_was_written_is_left_alone() {
        let written_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let bound_at = written_at + Duration::from_secs(1);
        assert!(!super::outrun(Some(bound_at), Some(written_at)));
    }

    /// Nothing to compare is not something to warn about. A platform whose socket is not a
    /// file, and a command with no core program beside it or on the PATH, both land here.
    #[test]
    fn a_figure_that_could_not_be_had_says_nothing() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        assert!(!super::outrun(None, Some(at)));
        assert!(!super::outrun(Some(at), None));
        assert!(!super::outrun(None, None));
    }
}
