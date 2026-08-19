//! Answering connections for as long as the process runs.

use std::thread::Scope;

use cistern_contract::{Request, Response, exchange::Server};

/// Answers each connection on a thread of its own, until the process ends.
///
/// A thread each rather than one after another. Reading the vendor's limit takes as long as
/// ninety seconds, and a command that asks for it would otherwise hold every other command
/// behind it. The stores are already shared with the threads that run tasks, and each holds a
/// lock from a read to the write that follows it.
///
/// Nothing counts the threads, and this does not stand against a peer that means harm. Anyone
/// who can reach the socket can already run programs on this machine as this user, and the
/// core runs the agent with those same permissions, so holding threads open is the least of
/// what such a peer could do. What the wait on a request is for is a surface that wedged or
/// went away without saying so, which is a thing that happens without anyone meaning it.
///
/// A cap on the threads would be a guess at a number nobody has needed. It is worth having the
/// day a peer that means harm is worth standing against, and that day starts with putting the
/// agent somewhere it cannot reach this machine.
///
/// A connection that panics ends its own thread and nothing else. `file::kept` takes a lock
/// back from a thread that panicked holding it, and the write is the last thing that happens
/// under one, so the file is whatever it was.
///
/// The socket is given up in the signal handler. That ends the process whether this is waiting
/// for the next connection or a thread is part way through answering one, and a surface whose
/// answer never arrives reports that the core is not running.
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
