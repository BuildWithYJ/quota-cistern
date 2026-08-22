//! Where a surface and the core meet.
//!
//! `docs/ipc.md` records the address.
//! Both sides read it from here so the two cannot drift apart.

#[cfg(unix)]
mod platform {
    use std::{
        env,
        ffi::OsString,
        fs::{self, DirBuilder, File, OpenOptions},
        io,
        os::unix::fs::{DirBuilderExt, PermissionsExt},
        path::{Path, PathBuf},
        time::SystemTime,
    };

    use interprocess::local_socket::{GenericFilePath, Name, prelude::*};
    use nix::fcntl::{Flock, FlockArg};

    /// `$XDG_RUNTIME_DIR`, or `~/.local/state` where that says nothing usable.
    ///
    /// The two are arguments rather than reads.
    /// The choice between them can be tested without setting a variable the whole process sees.
    pub(super) fn base_of(runtime: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
        if let Some(runtime) = absolute(runtime) {
            return Some(runtime);
        }
        Some(absolute(home)?.join(".local").join("state"))
    }

    /// A variable that names an absolute path, and nothing for one that does not.
    ///
    /// The XDG base directory specification holds that a path in one of these has to be
    /// absolute and that anything else is to be ignored. An empty variable taken at its word
    /// would put the socket under whatever directory a command was run from, and a command
    /// run somewhere else would find none and start a second core.
    fn absolute(dir: Option<OsString>) -> Option<PathBuf> {
        dir.map(PathBuf::from).filter(|dir| dir.is_absolute())
    }

    fn base() -> Option<PathBuf> {
        base_of(env::var_os("XDG_RUNTIME_DIR"), env::var_os("HOME"))
    }

    /// The directory the socket and the lock share.
    fn dir() -> io::Result<PathBuf> {
        base().map(|dir| dir.join("cistern")).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "neither XDG_RUNTIME_DIR nor HOME is set",
            )
        })
    }

    /// `$XDG_RUNTIME_DIR/cistern/sock`, or `~/.local/state/cistern/sock`.
    fn path() -> io::Result<PathBuf> {
        Ok(dir()?.join("sock"))
    }

    /// The file the kernel is asked to hand to one core at a time.
    ///
    /// Beside the socket rather than being the socket. Binding takes the socket away and puts
    /// a new one back, and a lock is worth nothing if the file it is held on can be unlinked
    /// by the next core to start.
    fn lock_path() -> io::Result<PathBuf> {
        Ok(dir()?.join("lock"))
    }

    /// Only this user may reach the socket.
    ///
    /// `$XDG_RUNTIME_DIR` is already private to the user where the system sets it. Where it
    /// says nothing the socket goes under the home directory, which is commonly readable by
    /// everyone on the machine, and a socket that anyone may connect to is one anyone may give
    /// work to. The core runs that work with this user's permissions.
    const ONLY_THIS_USER: u32 = 0o700;

    /// The socket file, as the local socket API names it.
    pub fn name() -> io::Result<Name<'static>> {
        named(&path()?)
    }

    /// When the socket now in place was bound, which is when the core holding it started.
    ///
    /// The core makes the socket as it starts and takes it away as it ends, so the file is as
    /// old as the core. A surface has no other way to ask how long a core has been running,
    /// and comparing it against the core program on disk is how a core left over from before a
    /// rebuild shows up: both sides report the same version, since a version does not carry a
    /// build.
    pub fn bound_at() -> io::Result<SystemTime> {
        fs::metadata(path()?)?.modified()
    }

    /// Makes the directory the socket goes in, reachable by this user and nobody else.
    ///
    /// Only the core calls this; a surface finding it absent has found that no core is running.
    ///
    /// A directory that already exists is narrowed as well as one that is made here, since the
    /// permissions a directory was made with are what a later run inherits.
    pub fn prepare() -> io::Result<()> {
        prepare_at(&dir()?)
    }

    /// The same, for a directory named rather than read out of the environment.
    pub(crate) fn prepare_at(dir: &Path) -> io::Result<()> {
        if let Some(parent) = dir.parent() {
            fs::create_dir_all(parent)?;
        }
        match DirBuilder::new().mode(ONLY_THIS_USER).create(dir) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            other => other?,
        }
        fs::set_permissions(dir, fs::Permissions::from_mode(ONLY_THIS_USER))
    }

    /// The lock that says this process is the one core, held until it ends.
    ///
    /// The kernel gives it back when the holder dies, however it died, which is what tells a
    /// core that is still running from a file a killed one left behind.
    pub struct Alone(#[allow(dead_code)] Flock<File>);

    /// Takes that lock, or says who has it.
    ///
    /// The stores are held against a lock inside one process. That lock means nothing between
    /// two of them, so being the only core is what makes it true, and it is taken before the
    /// socket is cleared and bound rather than after.
    pub fn hold_alone() -> io::Result<Alone> {
        hold_alone_at(&lock_path()?)
    }

    /// The same, for a file named rather than read out of the environment.
    pub(crate) fn hold_alone_at(lock: &Path) -> io::Result<Alone> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock)?;
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(held) => Ok(Alone(held)),
            Err((_, e)) => Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("another core holds the stores ({e})"),
            )),
        }
    }

    /// A socket file outlives a core that was killed, and binding to one that is still there fails.
    ///
    /// Connecting tells the two apart: an answer means a core holds it, a refusal means nobody does.
    pub fn clear_if_dead() -> io::Result<()> {
        clear_if_dead_at(&path()?)
    }

    /// The same, for a socket named rather than read out of the environment.
    pub(crate) fn clear_if_dead_at(path: &Path) -> io::Result<()> {
        if !path.exists() {
            return Ok(());
        }
        match interprocess::local_socket::Stream::connect(named(path)?) {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "a core is already listening on the socket",
            )),
            Err(_) => fs::remove_file(path),
        }
    }

    /// A socket file, as the local socket API names it.
    pub(crate) fn named(path: &Path) -> io::Result<Name<'static>> {
        path.to_path_buf().to_fs_name::<GenericFilePath>()
    }

    /// Takes the socket file away.
    ///
    /// A file that is already gone is the wanted state, not a failure.
    pub fn remove() -> io::Result<()> {
        match std::fs::remove_file(path()?) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::{io, time::SystemTime};

    use interprocess::local_socket::{GenericNamespaced, Name, prelude::*};

    /// The named pipe, as the local socket API names it.
    pub fn name() -> io::Result<Name<'static>> {
        "cistern".to_ns_name::<GenericNamespaced>()
    }

    /// A named pipe needs no directory.
    pub fn prepare() -> io::Result<()> {
        Ok(())
    }

    /// A named pipe is a kernel object that goes away with the process that held it.
    /// Nothing is left behind to clear.
    pub fn clear_if_dead() -> io::Result<()> {
        Ok(())
    }

    /// Nothing to hold: the name itself is the lock.
    ///
    /// Creating a named pipe under a name another process already holds fails, so the second
    /// core is turned away by the same call that would have bound it.
    pub struct Alone;

    pub fn hold_alone() -> io::Result<Alone> {
        Ok(Alone)
    }

    /// Nothing to take away.
    pub fn remove() -> io::Result<()> {
        Ok(())
    }

    /// A named pipe is not a file, so there is nothing whose age says when the core started.
    pub fn bound_at() -> io::Result<SystemTime> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a named pipe does not say when it was made",
        ))
    }
}

pub use platform::{Alone, bound_at, clear_if_dead, hold_alone, name, prepare, remove};
/// The same steps, for a directory named rather than read out of the environment.
#[cfg(all(unix, test))]
pub(crate) use platform::{clear_if_dead_at, hold_alone_at, named, prepare_at};

#[cfg(all(test, unix))]
mod alone {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::platform::{hold_alone_at, prepare_at};

    /// Listening is what a core does with the lock, so listening has to be holding it.
    ///
    /// This is the one that fails if the lock is ever taken out of the way the socket is opened.
    #[test]
    fn listening_holds_the_lock() {
        let held = TempDir::new().unwrap();
        let dir = held.path().join("cistern");

        let listening = crate::exchange::listen_in(&dir).unwrap();
        assert!(
            hold_alone_at(&dir.join("lock")).is_err(),
            "a core is listening and the lock is free"
        );
        drop(listening);
        assert!(hold_alone_at(&dir.join("lock")).is_ok());
    }

    /// A socket a killed core left behind is cleared, and the one a live core holds is not.
    #[test]
    fn a_socket_left_behind_is_cleared_and_a_live_one_is_not() {
        let held = TempDir::new().unwrap();
        let dir = held.path().join("cistern");
        prepare_at(&dir).unwrap();
        // What a core that was killed leaves.
        fs::write(dir.join("sock"), b"").unwrap();

        let listening = crate::exchange::listen_in(&dir).unwrap();
        assert!(dir.join("sock").exists());

        // A second core is stopped by the lock, before it can decide the live socket is dead.
        assert!(crate::exchange::listen_in(&dir).is_err());
        assert!(dir.join("sock").exists(), "the live socket was taken away");
        drop(listening);
    }

    /// The lock is what says one core holds the stores, so it is what a second core is refused by.
    #[test]
    fn a_second_core_is_turned_away() {
        let held = TempDir::new().unwrap();
        let lock = held.path().join("lock");

        let first = hold_alone_at(&lock).unwrap();
        assert!(hold_alone_at(&lock).is_err(), "two cores hold the stores");
        drop(first);
    }

    /// However a core ended, the kernel gives its lock back.
    /// A lock file that had to be tidied up would leave a killed core holding it forever.
    #[test]
    fn a_core_that_ended_gives_the_lock_back() {
        let held = TempDir::new().unwrap();
        let lock = held.path().join("lock");

        drop(hold_alone_at(&lock).unwrap());
        assert!(
            hold_alone_at(&lock).is_ok(),
            "the lock outlived the core that held it"
        );
    }

    /// Nobody but this user may reach the socket, wherever it landed.
    ///
    /// `$XDG_RUNTIME_DIR` is private to the user where the system sets it, and where it says
    /// nothing the socket goes under a home directory that commonly is not.
    #[test]
    fn the_directory_is_this_user_alone() {
        let held = TempDir::new().unwrap();
        let dir = held.path().join("cistern");
        // A directory left open by an earlier version is narrowed rather than left as it was.
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        prepare_at(&dir).unwrap();

        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the socket directory is open to others");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::platform::base_of;

    fn some(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn the_runtime_directory_wins() {
        assert_eq!(
            base_of(some("/run/user/1000"), some("/home/a")),
            Some(PathBuf::from("/run/user/1000"))
        );
    }

    #[test]
    fn home_stands_in_where_there_is_no_runtime_directory() {
        assert_eq!(
            base_of(None, some("/home/a")),
            Some(PathBuf::from("/home/a/.local/state"))
        );
    }

    #[test]
    fn neither_leaves_nowhere_to_put_it() {
        assert_eq!(base_of(None, None), None);
    }

    /// The specification holds that a path in one of these has to be absolute and that
    /// anything else is to be ignored. A variable taken at its word would put the socket under
    /// whatever directory a command was run from, and a command run somewhere else would find
    /// none and start a second core.
    #[test]
    fn a_variable_that_is_not_an_absolute_path_is_passed_over() {
        assert_eq!(
            base_of(some(""), some("/home/a")),
            Some(PathBuf::from("/home/a/.local/state"))
        );
        assert_eq!(
            base_of(some("run/user/1000"), some("/home/a")),
            Some(PathBuf::from("/home/a/.local/state"))
        );
        assert_eq!(base_of(some(""), some("")), None);
    }
}
