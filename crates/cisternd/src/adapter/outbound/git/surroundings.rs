//! What the working tree has changed, as git reads it.
//!
//! The only place that knows the command, and none of it reaches the core.

use std::process::Command;

use crate::core::port::outbound::Surroundings;

/// The uncommitted changes read out of the repository the task was added from.
pub struct GitSurroundings;

impl Surroundings for GitSurroundings {
    fn changed(&self, repository: &str) -> Vec<String> {
        // Changes against HEAD, staged or not. What is committed is behind the author, not what
        // they are in the middle of.
        let done = Command::new("git")
            .args([
                "-C",
                repository,
                "--no-pager",
                "diff",
                "--name-only",
                "HEAD",
            ])
            .output();

        match done {
            Ok(done) if done.status.success() => String::from_utf8_lossy(&done.stdout)
                .lines()
                .map(str::to_owned)
                .collect(),
            // A repository that cannot be read is not a run's to guess in.
            _ => Vec::new(),
        }
    }
}
