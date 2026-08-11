//! Giving up the socket when the process is asked to stop.

use cistern_contract::address;

/// Arranges for the socket to be removed when the process is asked to stop.
///
/// The handler ends the process itself.
/// The accept loop is blocked, and there is nothing else waiting to notice a flag.
pub fn remove_on_signal() -> Result<(), ctrlc::Error> {
    ctrlc::set_handler(|| {
        let _ = address::remove();
        std::process::exit(0);
    })
}
