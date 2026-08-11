//! One exchange between a surface and the core.
//!
//! `docs/ipc.md` records the address and the framing.
//! Both programs read and write over the same socket.
//! The rule that one line carries one message is written here once.
//! Whether the two versions match is settled here too, from the envelope alone.

use std::io::{self, BufRead, BufReader, Write};

use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream, prelude::*};

use crate::{
    Answer, Failure, Request, Response, VERSION,
    address::{clear_if_dead, name, prepare},
    code::{CORE_ERROR, USAGE_ERROR},
};

/// Sends one request and reads the answer.
///
/// Fails when no core is listening, with [`io::ErrorKind::ConnectionRefused`] or [`io::ErrorKind::NotFound`].
pub fn ask(command: &str, params: serde_json::Value) -> io::Result<Response> {
    ask_at(name()?, command, params)
}

/// Sends one request to a given address.
///
/// [`ask`] is this at the address `docs/ipc.md` names.
/// A test reaches a temporary socket through here, the way an adapter takes the path it is given.
pub(crate) fn ask_at(
    name: Name<'_>,
    command: &str,
    params: serde_json::Value,
) -> io::Result<Response> {
    let stream = Stream::connect(name)?;

    let request = Request {
        version: VERSION.to_owned(),
        command: command.to_owned(),
        params,
    };
    write_line(&stream, &serde_json::to_string(&request)?)?;

    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    Response::from_line(&line).map_err(io::Error::from)
}

/// The socket the core listens on.
///
/// It holds the listener rather than handing it out, so that no caller has to name the socket library.
pub struct Server(Listener);

/// One connection that has arrived and not yet been answered.
pub struct Exchange(Stream);

/// Opens the socket.
///
/// Fails with [`io::ErrorKind::AddrInUse`] when a core is already listening.
pub fn listen() -> io::Result<Server> {
    prepare()?;
    let name = name()?;
    clear_if_dead()?;
    listen_at(name)
}

/// Opens the socket at a given address, without preparing or clearing it.
pub(crate) fn listen_at(name: Name<'_>) -> io::Result<Server> {
    ListenerOptions::new().name(name).create_sync().map(Server)
}

impl Server {
    /// Waits for the next connection.
    ///
    /// How long to keep accepting, and what to do when one connection fails, are the core's to decide.
    /// This hands back a single connection instead of looping over them.
    pub fn accept(&self) -> io::Result<Exchange> {
        self.0.accept().map(Exchange)
    }
}

impl Exchange {
    /// Reads one request, answers it, and writes one response.
    ///
    /// A line that is not a request never reaches `respond`.
    /// Neither does a request from a surface of another version.
    /// What arrived is the framing's business, and the framing is here.
    pub fn answer(self, respond: impl FnOnce(Request) -> Response) -> io::Result<()> {
        let mut line = String::new();
        // Connecting and leaving is how a surface finds out whether anyone is listening.
        // There is nothing to answer and nothing went wrong.
        if BufReader::new(&self.0).read_line(&mut line)? == 0 {
            return Ok(());
        }

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => settle(request, respond),
            Err(e) => Response::Error(Failure::new(USAGE_ERROR, e.to_string())),
        };
        write_line(&self.0, &serde_json::to_string(&response)?)
    }
}

/// Answers what the envelope alone decides, and passes on the rest.
///
/// A daemon outlives the install that replaced it, so a surface can be newer than the core it reached.
/// `core_version` is exempt from the comparison: it is how a surface finds out which side is behind.
fn settle(request: Request, respond: impl FnOnce(Request) -> Response) -> Response {
    if request.command == "core_version" {
        return Response::Data(Answer {
            command: request.command,
            data: serde_json::json!({ "version": VERSION }),
        });
    }
    if request.version != VERSION {
        let message = format!("core is {VERSION}, surface is {}", request.version);
        let both = serde_json::json!({ "core": VERSION, "surface": request.version });
        return Response::Error(Failure::new(CORE_ERROR, message).with_data(both));
    }
    respond(request)
}

fn write_line(stream: &Stream, line: &str) -> io::Result<()> {
    let mut out = stream;
    writeln!(out, "{line}")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn asked(command: &str, version: &str) -> Request {
        Request {
            version: version.to_owned(),
            command: command.to_owned(),
            params: serde_json::Value::Null,
        }
    }

    /// Stands in for the core, and records whether it was reached.
    fn core(reached: &Cell<bool>) -> impl FnOnce(Request) -> Response + '_ {
        move |request| {
            reached.set(true);
            Response::Data(Answer {
                command: request.command,
                data: serde_json::json!({ "reached": true }),
            })
        }
    }

    fn refusal(response: Response) -> Failure {
        match response {
            Response::Error(failure) => failure,
            Response::Data(answer) => panic!("expected a refusal, got {answer:?}"),
        }
    }

    fn answer(response: Response) -> Answer {
        match response {
            Response::Data(answer) => answer,
            Response::Error(failure) => panic!("expected an answer, got {failure:?}"),
        }
    }

    #[test]
    fn the_version_is_answered_without_reaching_the_core() {
        let reached = Cell::new(false);
        let response = settle(asked("core_version", VERSION), core(&reached));
        assert!(!reached.get());
        assert_eq!(
            answer(response).data,
            serde_json::json!({ "version": VERSION })
        );
    }

    #[test]
    fn a_surface_of_another_version_can_still_ask_which_core_this_is() {
        let reached = Cell::new(false);
        let response = settle(asked("core_version", "0.0.1"), core(&reached));
        assert!(matches!(response, Response::Data(_)));
    }

    #[test]
    fn another_version_is_refused_with_both_versions() {
        let reached = Cell::new(false);
        let refused = refusal(settle(asked("config_get", "0.0.1"), core(&reached)));
        assert!(!reached.get());
        assert_eq!(refused.code, CORE_ERROR);
        assert_eq!(
            refused.data,
            Some(serde_json::json!({ "core": VERSION, "surface": "0.0.1" }))
        );
    }

    #[test]
    fn a_request_of_this_version_reaches_the_core() {
        let reached = Cell::new(false);
        settle(asked("config_get", VERSION), core(&reached));
        assert!(reached.get());
    }
}

/// A round trip over a real socket.
///
/// Only the framing is exercised, so the core is a closure that echoes.
/// The socket lives in a directory of the test's own.
/// That leaves a running core alone and keeps two tests from meeting on one address.
#[cfg(all(test, unix))]
mod round_trip {
    use std::{
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    use interprocess::local_socket::GenericFilePath;
    use tempfile::TempDir;

    use super::*;

    fn a_socket() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sock");
        (dir, path)
    }

    fn address(path: &Path) -> Name<'static> {
        path.to_path_buf().to_fs_name::<GenericFilePath>().unwrap()
    }

    /// Answers one connection and records whether the core was reached.
    fn answering(server: Server, reached: Arc<AtomicBool>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let exchange = server.accept().unwrap();
            exchange
                .answer(|request| {
                    reached.store(true, Ordering::SeqCst);
                    Response::Data(Answer {
                        command: request.command,
                        data: serde_json::json!({ "echoed": request.params }),
                    })
                })
                .unwrap();
        })
    }

    #[test]
    fn a_request_comes_back_answered() {
        let (_dir, path) = a_socket();
        let server = listen_at(address(&path)).unwrap();
        let reached = Arc::new(AtomicBool::new(false));
        let serving = answering(server, Arc::clone(&reached));

        let response = ask_at(
            address(&path),
            "config_get",
            serde_json::json!({ "key": "plan" }),
        )
        .unwrap();
        serving.join().unwrap();

        assert!(reached.load(Ordering::SeqCst));
        match response {
            Response::Data(answer) => {
                assert_eq!(answer.command, "config_get");
                assert_eq!(
                    answer.data,
                    serde_json::json!({ "echoed": { "key": "plan" } })
                );
            }
            Response::Error(failure) => panic!("expected an answer, got {failure:?}"),
        }
    }

    #[test]
    fn a_line_that_is_not_a_request_never_reaches_the_core() {
        let (_dir, path) = a_socket();
        let server = listen_at(address(&path)).unwrap();
        let reached = Arc::new(AtomicBool::new(false));
        let serving = answering(server, Arc::clone(&reached));

        let stream = Stream::connect(address(&path)).unwrap();
        write_line(&stream, "this is not a request").unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        serving.join().unwrap();

        assert!(!reached.load(Ordering::SeqCst));
        match Response::from_line(&line).unwrap() {
            Response::Error(failure) => assert_eq!(failure.code, USAGE_ERROR),
            Response::Data(answer) => panic!("expected a refusal, got {answer:?}"),
        }
    }
}
