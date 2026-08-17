//! Where a surface and the core meet.
//!
//! `docs/ipc.md` records the address.
//! Both sides read it from here so the two cannot drift apart.

#[cfg(unix)]
mod platform {
    use std::{env, ffi::OsString, io, path::PathBuf};

    use interprocess::local_socket::{GenericFilePath, Name, prelude::*};

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

    /// `$XDG_RUNTIME_DIR/cistern/sock`, or `~/.local/state/cistern/sock`.
    fn path() -> io::Result<PathBuf> {
        base()
            .map(|dir| dir.join("cistern").join("sock"))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "neither XDG_RUNTIME_DIR nor HOME is set",
                )
            })
    }

    /// The socket file, as the local socket API names it.
    pub fn name() -> io::Result<Name<'static>> {
        path()?.to_fs_name::<GenericFilePath>()
    }

    /// Makes the directory the socket goes in.
    ///
    /// Only the core calls this; a surface finding it absent has found that no core is running.
    pub fn prepare() -> io::Result<()> {
        match path()?.parent() {
            Some(parent) => std::fs::create_dir_all(parent),
            None => Ok(()),
        }
    }

    /// A socket file outlives a core that was killed, and binding to one that is still there fails.
    ///
    /// Connecting tells the two apart: an answer means a core holds it, a refusal means nobody does.
    pub fn clear_if_dead() -> io::Result<()> {
        let path = path()?;
        if !path.exists() {
            return Ok(());
        }
        match interprocess::local_socket::Stream::connect(name()?) {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "a core is already listening on the socket",
            )),
            Err(_) => std::fs::remove_file(path),
        }
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
    use std::io;

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

    /// Nothing to take away.
    pub fn remove() -> io::Result<()> {
        Ok(())
    }
}

pub use platform::{clear_if_dead, name, prepare, remove};

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
