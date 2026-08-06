//! The link to the core.
//!
//! `docs/ipc.md` records the framing this follows.

use std::io::{self, BufRead, BufReader, Write};

use cistern_contract::{Request, Response, VERSION, address};
use interprocess::local_socket::{Stream, prelude::*};

/// Sends one request and reads the answer.
///
/// Fails with [`io::ErrorKind::ConnectionRefused`] or
/// [`io::ErrorKind::NotFound`] when no core is listening.
pub fn ask(command: &str, params: serde_json::Value) -> io::Result<Response> {
    let stream = Stream::connect(address::name()?)?;

    let request = Request {
        version: VERSION.to_owned(),
        command: command.to_owned(),
        params,
    };
    let mut out = &stream;
    writeln!(out, "{}", serde_json::to_string(&request)?)?;
    out.flush()?;

    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    Response::from_line(&line).map_err(io::Error::from)
}
