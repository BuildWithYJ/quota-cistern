//! What the working tree has changed, and what the repository holds by a word, as git reads it.
//!
//! The only place that knows the commands, and none of it reaches the core.

use std::process::Command;

use crate::core::port::outbound::Surroundings;

/// The uncommitted changes and by-word matches read out of the repository a task was added from.
pub struct GitSurroundings;

impl Surroundings for GitSurroundings {
    fn changed(&self, repository: &str) -> Vec<String> {
        // Changes against HEAD, staged or not. What is committed is behind the author, not what
        // they are in the middle of.
        run(repository, &["diff", "--name-only", "HEAD"])
            .map(paths)
            .unwrap_or_default()
    }

    fn holding(&self, repository: &str, word: &str) -> Vec<String> {
        // A file that uses the word in a line, then one that only carries it in its name. A line
        // is a closer match than a name, so those come first.
        let mut found = run(
            repository,
            &["grep", "--name-only", "--ignore-case", "-e", word],
        )
        .map(paths)
        .unwrap_or_default();

        if let Some(named) = run(repository, &["ls-files"]) {
            let word = word.to_ascii_lowercase();
            for path in named.lines() {
                if path.to_ascii_lowercase().contains(&word)
                    && !found.iter().any(|held| held == path)
                {
                    found.push(path.to_owned());
                }
            }
        }
        found
    }
}

/// Runs one git command in the repository and hands back what it printed, nothing when it failed.
fn run(repository: &str, args: &[&str]) -> Option<String> {
    let done = Command::new("git")
        .args(["-C", repository, "--no-pager"])
        .args(args)
        .output()
        .ok()?;

    done.status
        .success()
        .then(|| String::from_utf8_lossy(&done.stdout).into_owned())
}

/// The lines of git's output as owned paths.
fn paths(out: String) -> Vec<String> {
    out.lines().map(str::to_owned).collect()
}
