//! The local socket the core listens on.
//!
//! `docs/ipc.md` records the address and the framing.

use std::io::{self, BufRead, BufReader, Write};

use cistern_contract::{
    Failure, Request, Response,
    address::{clear_if_dead, name, prepare},
    code::USAGE_ERROR,
};
use interprocess::local_socket::{Listener, ListenerOptions, Stream, prelude::*};

/// Opens the socket and returns what accepts connections on it.
///
/// Fails with [`io::ErrorKind::AddrInUse`] when a core is already listening.
pub fn listen() -> io::Result<Listener> {
    prepare()?;
    let name = name()?;
    clear_if_dead()?;
    ListenerOptions::new().name(name).create_sync()
}

/// Answers each connection with one response, until the process ends.
pub fn serve(listener: Listener, respond: impl Fn(Request) -> Response) -> io::Result<()> {
    for connection in listener.incoming() {
        // One surface giving up is not a reason to stop serving the rest, but
        // a core that answers nothing should not do it quietly either.
        if let Err(e) = connection.and_then(|stream| answer(stream, &respond)) {
            eprintln!("cisternd: a connection failed: {e}");
        }
    }
    Ok(())
}

/// Reads one request and writes one response.
///
/// A line that is not a request never reaches `respond`. What arrived is the
/// framing's business, and the framing is here.
fn answer(stream: Stream, respond: &impl Fn(Request) -> Response) -> io::Result<()> {
    let mut line = String::new();
    // Connecting and leaving is how a surface finds out whether anyone is
    // listening. There is nothing to answer and nothing went wrong.
    if BufReader::new(&stream).read_line(&mut line)? == 0 {
        return Ok(());
    }

    let response = match serde_json::from_str::<Request>(&line) {
        Ok(request) => respond(request),
        Err(e) => Response::Error(Failure::new(USAGE_ERROR, e.to_string())),
    };

    let mut out = &stream;
    writeln!(out, "{}", serde_json::to_string(&response)?)?;
    out.flush()
}

/// Arranges for the socket to be removed when the process is asked to stop.
///
/// The handler ends the process itself, because the accept loop is blocked and
/// there is nothing else waiting to notice a flag.
pub fn remove_on_signal() -> Result<(), ctrlc::Error> {
    ctrlc::set_handler(|| {
        let _ = cistern_contract::address::remove();
        std::process::exit(0);
    })
}
