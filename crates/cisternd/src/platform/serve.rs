//! Answering connections for as long as the process runs.

use std::thread::Scope;

use cistern_contract::{Request, Response, exchange::Server};

/// Answers each connection on a thread of its own, until the process ends.
///
/// The socket is given up in the signal handler.
/// That ends the process while this is blocked waiting for the next connection.
///
/// A thread each rather than one after another. Reading the vendor's limit takes as long as
/// ninety seconds, and a command that asks for it would otherwise hold every other command
/// behind it. The stores are already shared with the threads that run tasks, and each holds a
/// lock from a read to the write that follows it.
///
/// Nothing counts the threads. A connection is a local command that lives for one exchange,
/// and a cap would be a guess at a number nobody has needed.
pub fn serve<'scope, 'env>(
    server: &'env Server,
    respond: &'env (dyn Fn(Request) -> Response + Sync),
    threads: &'scope Scope<'scope, 'env>,
) -> ! {
    loop {
        // One surface giving up is not a reason to stop serving the rest.
        // A core that answers nothing should not do it quietly either.
        match server.accept() {
            Ok(exchange) => {
                threads.spawn(move || {
                    if let Err(e) = exchange.answer(respond) {
                        eprintln!("cisternd: a connection failed: {e}");
                    }
                });
            }
            Err(e) => eprintln!("cisternd: a connection failed: {e}"),
        }
    }
}
