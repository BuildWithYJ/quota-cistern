//! Starting the core when a command finds none listening.
//!
//! Every command but `--version` talks to the core, since the core owns the stores and this
//! side reads none of them. So finding nothing listening is not an answer to give a reader; it
//! is a core to start.
//!
//! `--version` is the exception. It asks whether the two sides match, and a core that is not
//! running is what it has to report.

use std::{
    env,
    ffi::OsString,
    fs::{self, OpenOptions},
    io,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use cistern_contract::{Response, exchange};
use serde_json::Value;

/// How long to wait for the core to answer, and how often to try again.
///
/// A core that starts cleanly answers within a few milliseconds, so this is room for a slow
/// machine rather than a wait anyone should see. Five seconds is also about as long as someone
/// watching a command will take it for working rather than stuck.
const ANSWERS_WITHIN: Duration = Duration::from_secs(5);
const TRY_EVERY: Duration = Duration::from_millis(5);

/// What the core is called.
pub const CORE: &str = "cisternd";

/// Where what the core writes is kept.
const LOG: &str = "daemon.log";

/// Asks the core, starting one first if none is listening.
pub fn ask(command: &str, params: Value) -> io::Result<Response> {
    match exchange::ask(command, params.clone()) {
        Err(e) if nobody_listening(&e) => started_then_asked(command, params),
        first => first,
    }
}

/// Whether the failure is nothing listening rather than something going wrong.
///
/// These are the two `exchange::ask` documents for a socket with no core behind it. Anything
/// else is a failure to report as it is rather than to answer by starting a second core.
fn nobody_listening(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
    )
}

/// Starts a core and asks again until it answers.
///
/// Asking is the test of whether it is ready, so there is no second idea of readiness to keep
/// in step with this one.
///
/// Two commands starting at once each start a core, and the one that loses ends with what it
/// could not take. That is the loser's own message in the log, and the winner answers both.
///
/// So the core this command started stopping is not a reason to stop asking. It stopped
/// because another one holds the stores, which is to say because an answer is coming from
/// somewhere. What this wants is an answer, not an answer from the core it happened to start.
/// Only the deadline ends the waiting, and what the core said is where the message points
/// either way.
fn started_then_asked(command: &str, params: Value) -> io::Result<Response> {
    let kept = kept()?;
    let mut core = start(&kept)?;
    let since = Instant::now();
    let mut stopped = None;

    loop {
        match exchange::ask(command, params.clone()) {
            Err(e) if nobody_listening(&e) => {}
            answered => return answered,
        }
        if stopped.is_none() {
            stopped = core.try_wait()?;
        }
        if since.elapsed() >= ANSWERS_WITHIN {
            // Left running rather than killed. A core that took the stores and is still
            // starting is the one thing that will answer, and a command that gave up is not
            // reason enough to take that away from the next one.
            let why = match stopped {
                Some(status) => format!(
                    "the core this command started stopped ({status}) and no other answered \
                     within {}s",
                    ANSWERS_WITHIN.as_secs()
                ),
                None => format!(
                    "the core did not answer within {}s",
                    ANSWERS_WITHIN.as_secs()
                ),
            };
            if stopped.is_none() {
                // Nothing answered and it is still running, so it is stuck before it
                // listened. Left behind, every command after this starts another.
                let _ = core.kill();
                let _ = core.wait();
            }
            return Err(gave_up(&format!(
                "{why}; what it said is in {}",
                kept.display()
            )));
        }
        thread::sleep(TRY_EVERY);
    }
}

/// Runs the core, with what it writes going to the file rather than to this terminal.
///
/// It is put in a process group of its own so that it outlives the command that started it.
/// A core sharing this one's group would take the interrupt meant for the command.
fn start(kept: &PathBuf) -> io::Result<Child> {
    let writing = OpenOptions::new().create(true).append(true).open(kept)?;
    let mut running = Command::new(beside_this_one().unwrap_or_else(|| PathBuf::from(CORE)));
    running
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(writing)
        .process_group(0);

    running.spawn().or_else(|beside| {
        // Nothing beside the command line, so whatever the PATH holds.
        Command::new(CORE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(OpenOptions::new().create(true).append(true).open(kept)?)
            .process_group(0)
            .spawn()
            .map_err(|on_path| {
                gave_up(&format!(
                    "no {CORE} beside this command ({beside}) and none on the PATH ({on_path})"
                ))
            })
    })
}

/// The core program this command would start, which is the file a build replaces.
///
/// The same two places `start` tries and in the same order, so what this names is what would
/// run. Nothing where neither holds one.
pub fn program() -> Option<PathBuf> {
    beside_this_one().or_else(on_path)
}

/// The first core the `PATH` holds.
fn on_path() -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(CORE))
        .find(|at| at.is_file())
}

/// The core beside this command, which is where an install puts the two.
///
/// Tried before the `PATH` so that a command started from an install talks to that install's
/// core rather than to an older one left somewhere on the `PATH`.
fn beside_this_one() -> Option<PathBuf> {
    let beside = env::current_exe().ok()?.parent()?.join(CORE);
    beside.is_file().then_some(beside)
}

/// The file what the core writes goes to, with its directory made.
///
/// Under `$XDG_STATE_HOME`, which the XDG base directory specification keeps for what a program
/// writes down between runs without it being data anyone asked for.
fn kept() -> io::Result<PathBuf> {
    let at = state_home(env::var_os("XDG_STATE_HOME"), env::var_os("HOME"))
        .ok_or_else(|| gave_up("neither XDG_STATE_HOME nor HOME says where to write"))?
        .join("cistern");
    fs::create_dir_all(&at)?;
    Ok(at.join(LOG))
}

/// `$XDG_STATE_HOME`, or `~/.local/state` where that says nothing usable.
///
/// The two are arguments rather than reads, so the choice between them can be tested without
/// setting a variable the whole process sees.
fn state_home(state: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    match absolute(state) {
        Some(dir) => Some(dir),
        None => Some(absolute(home)?.join(".local").join("state")),
    }
}

/// A variable that names an absolute path, and nothing for one that does not.
///
/// The XDG base directory specification holds that a path in one of these has to be absolute
/// and that anything else is to be ignored. An empty variable taken at its word would put the
/// file under whatever directory the command was run from, which is a different file each time.
fn absolute(dir: Option<OsString>) -> Option<PathBuf> {
    dir.map(PathBuf::from).filter(|dir| dir.is_absolute())
}

/// A failure to start reads as a failure to reach, since that is what the caller was doing.
fn gave_up(why: &str) -> io::Error {
    io::Error::other(why.to_owned())
}

#[cfg(test)]
mod tests;
