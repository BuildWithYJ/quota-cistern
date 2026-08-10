//! What answers the repository ports, through git.
//!
//! Every git invocation is here. Which commands answer which question is this
//! directory's, and none of them reaches the core.
//!
//! `roots` is here too, though it runs nothing: what marks a repository is
//! git's rule whether or not a command is asked about it.

use std::process::Output;

pub mod result;
pub mod roots;
pub mod worktree;

/// What git said about a command that failed.
///
/// It writes its complaint to standard error and says nothing on standard
/// output, but a command that fails without a word is worse than one that
/// repeats itself, so both are read.
fn said(done: &Output) -> String {
    let complained = String::from_utf8_lossy(&done.stderr).trim().to_owned();
    match complained.is_empty() {
        false => complained,
        true => String::from_utf8_lossy(&done.stdout).trim().to_owned(),
    }
}
