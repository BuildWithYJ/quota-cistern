//! Answering connections for as long as the process runs.

use cistern_contract::{Request, Response, exchange::Server};

/// Answers each connection with one response, until the process ends.
///
/// It never returns. The socket is given up in the signal handler, which ends
/// the process while this is blocked waiting for the next connection.
pub fn serve(server: &Server, respond: impl Fn(Request) -> Response) -> ! {
    loop {
        // One surface giving up is not a reason to stop serving the rest, but a
        // core that answers nothing should not do it quietly either.
        if let Err(e) = server
            .accept()
            .and_then(|exchange| exchange.answer(&respond))
        {
            eprintln!("cisternd: a connection failed: {e}");
        }
    }
}
