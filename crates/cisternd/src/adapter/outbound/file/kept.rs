//! A file that holds one document.
//!
//! Three stores keep one file each and treat it the same way: read the whole of
//! it, hand it over, and put the whole of it back. What they do not share is the
//! format and the fields, which stay with each of them.
//!
//! This touches no port, so it is not an adapter.

use std::{
    ffi::OsString,
    fmt::Display,
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};

use crate::core::port::outbound::Unavailable;

/// Where a file is and the lock over it.
///
/// The directories are arguments rather than reads, so that which one is used
/// can be tested without setting a variable the whole process sees.
pub struct Kept {
    path: PathBuf,
    /// Held from a read to the write that follows it, so that two writers do
    /// not each put back what the other never saw.
    writing: Mutex<()>,
}

impl Kept {
    /// Takes the path it is given. This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        Kept {
            path,
            writing: Mutex::new(()),
        }
    }

    /// `$XDG_DATA_HOME/cistern/<name>`, or `~/.local/share/cistern/<name>`.
    ///
    /// Data rather than state, because the XDG specification keeps state for
    /// what is not worth carrying between machines, and what a user registered
    /// is not that.
    pub fn in_data_home(
        data_home: Option<OsString>,
        home: Option<OsString>,
        name: &str,
    ) -> Option<PathBuf> {
        let base = match data_home {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(home?).join(".local").join("share"),
        };
        Some(base.join("cistern").join(name))
    }

    /// `$XDG_CONFIG_HOME/cistern/<name>`, or `~/.config/cistern/<name>`.
    pub fn in_config_home(
        config_home: Option<OsString>,
        home: Option<OsString>,
        name: &str,
    ) -> Option<PathBuf> {
        let base = match config_home {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(home?).join(".config"),
        };
        Some(base.join("cistern").join(name))
    }

    /// What the file holds, or nothing when there is no file yet.
    ///
    /// A file nobody has written is not a failure. Each store says for itself
    /// what nothing means, since each has to answer before anything is
    /// written.
    pub fn read(&self) -> Result<Option<String>, Unavailable> {
        match fs::read_to_string(&self.path) {
            Ok(written) => Ok(Some(written)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(self.failing(e)),
        }
    }

    /// Writes beside the file and renames it into place.
    ///
    /// A write that stops halfway leaves the half-written file beside the real
    /// one, and the real one is whatever it was. Renaming is the one step that
    /// either happened or did not.
    pub fn write(&self, written: &str) -> Result<(), Unavailable> {
        let staged = staged(&self.path);
        replace(&self.path, &staged, written).map_err(|e| self.failing(e))
    }

    /// Holds the file from a read to the write that follows it.
    ///
    /// A thread that panicked while holding this left the file alone, since the
    /// write is the last thing that happens under it.
    pub fn holding(&self) -> MutexGuard<'_, ()> {
        self.writing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What went wrong, said with the file it went wrong on.
    pub fn failing(&self, e: impl Display) -> Unavailable {
        Unavailable::new(format!("{}: {e}", self.path.display()))
    }
}

/// Where the half-written copy goes.
///
/// Beside the real file rather than in a temporary directory, so that the
/// rename stays on one filesystem and cannot become a copy.
fn staged(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".new");
    path.with_file_name(name)
}

fn replace(path: &Path, staged: &Path, written: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Err(e) = fs::write(staged, written) {
        let _ = fs::remove_file(staged);
        return Err(e);
    }
    fs::rename(staged, path)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn some(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn the_data_directory_wins() {
        assert_eq!(
            Kept::in_data_home(some("/x/.share"), some("/home/a"), "backlog.json"),
            Some(PathBuf::from("/x/.share/cistern/backlog.json"))
        );
    }

    #[test]
    fn home_stands_in_where_there_is_no_data_directory() {
        assert_eq!(
            Kept::in_data_home(None, some("/home/a"), "backlog.json"),
            Some(PathBuf::from("/home/a/.local/share/cistern/backlog.json"))
        );
    }

    #[test]
    fn the_configuration_directory_wins() {
        assert_eq!(
            Kept::in_config_home(some("/x/.conf"), some("/home/a"), "config.toml"),
            Some(PathBuf::from("/x/.conf/cistern/config.toml"))
        );
    }

    #[test]
    fn home_stands_in_where_there_is_no_configuration_directory() {
        assert_eq!(
            Kept::in_config_home(None, some("/home/a"), "config.toml"),
            Some(PathBuf::from("/home/a/.config/cistern/config.toml"))
        );
    }

    #[test]
    fn neither_leaves_nowhere_to_put_it() {
        assert_eq!(Kept::in_data_home(None, None, "backlog.json"), None);
        assert_eq!(Kept::in_config_home(None, None, "config.toml"), None);
    }

    #[test]
    fn a_file_nobody_has_written_reads_as_nothing() {
        let held = TempDir::new().unwrap();
        let kept = Kept::at(held.path().join("cistern").join("backlog.json"));
        assert_eq!(kept.read().unwrap(), None);
    }

    #[test]
    fn what_was_written_comes_back() {
        let held = TempDir::new().unwrap();
        let kept = Kept::at(held.path().join("cistern").join("backlog.json"));

        kept.write("{}").unwrap();
        assert_eq!(kept.read().unwrap().as_deref(), Some("{}"));
    }

    /// The directory is made on the way, since nothing else makes it.
    #[test]
    fn writing_makes_the_directory_it_needs() {
        let held = TempDir::new().unwrap();
        let at = held.path().join("deep").join("down").join("backlog.json");

        Kept::at(at.clone()).write("{}").unwrap();
        assert!(at.exists());
    }

    /// Nothing is left beside the file once the write has landed.
    #[test]
    fn the_half_written_copy_does_not_survive_a_write() {
        let held = TempDir::new().unwrap();
        let at = held.path().join("backlog.json");

        Kept::at(at.clone()).write("{}").unwrap();
        assert!(!staged(&at).exists());
    }

    #[test]
    fn the_half_written_copy_goes_beside_the_file() {
        assert_eq!(
            staged(Path::new("/x/cistern/backlog.json")),
            PathBuf::from("/x/cistern/backlog.json.new")
        );
    }
}
