//! The configuration file.
//!
//! The only place that knows the path and the file format. Neither reaches the
//! core.

use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::core::port::outbound::{ConfigurationStore, StoredConfiguration, Unavailable};

/// The configuration, kept as TOML at a path fixed when this is built.
pub struct FileConfiguration {
    path: PathBuf,
}

/// The file, as TOML sees it.
///
/// It stands apart from the port's type because the key spelling, the
/// absent-field rules, and which TOML type a value is written as all belong to
/// the format, not to what is being stored.
///
/// A value is held as whatever TOML found rather than as what the key is
/// supposed to take. Which values a key takes is the core's to decide, so a
/// file holding the wrong sort of one reaches it and is refused there.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(skip_serializing_if = "Option::is_none")]
    vendor: Option<toml::Value>,
}

/// The text a user would have typed for what TOML holds.
///
/// A string keeps its contents; everything else is rendered as the file writes
/// it, so a number, a boolean, and a table all reach the core as something it
/// can read and refuse.
fn as_text(value: toml::Value) -> String {
    match value {
        toml::Value::String(text) => text,
        other => other.to_string(),
    }
}

impl FileConfiguration {
    /// Takes the path it is given. This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileConfiguration { path }
    }

    /// The path `docs/cli.md` names, or nothing when there is nowhere for it.
    pub fn in_config_home() -> Option<Self> {
        path_of(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME")).map(FileConfiguration::at)
    }

    fn failing(&self, e: impl std::fmt::Display) -> Unavailable {
        Unavailable::new(format!("{}: {e}", self.path.display()))
    }
}

/// `$XDG_CONFIG_HOME/cistern/config.toml`, or `~/.config/cistern/config.toml`.
///
/// The two are arguments rather than reads, so that the choice between them
/// can be tested without setting a variable the whole process sees.
fn path_of(config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match config_home {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home?).join(".config"),
    };
    Some(base.join("cistern").join("config.toml"))
}

impl ConfigurationStore for FileConfiguration {
    fn load(&self) -> Result<StoredConfiguration, Unavailable> {
        let written = match fs::read_to_string(&self.path) {
            Ok(written) => written,
            // Nothing stored yet. Any other read failure is worth saying.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredConfiguration::default());
            }
            Err(e) => return Err(self.failing(e)),
        };

        let document: Document = toml::from_str(&written).map_err(|e| self.failing(e))?;
        Ok(StoredConfiguration {
            vendor: document.vendor.map(as_text),
        })
    }

    /// Writes beside the file and renames it into place.
    ///
    /// Opening the file truncates it first, so a process that stops after that
    /// leaves an empty one behind, and we would be the ones writing the file
    /// `load` then refuses. A rename within one filesystem is atomic, so a
    /// reader sees the old contents or the new ones and nothing between.
    fn store(&self, stored: &StoredConfiguration) -> Result<(), Unavailable> {
        let document = Document {
            vendor: stored.vendor.clone().map(toml::Value::String),
        };
        let written = toml::to_string(&document).map_err(|e| self.failing(e))?;

        // Beside the file, because a rename is atomic only within a filesystem
        // and a temporary directory may be on another one.
        let staged = self.path.with_extension("toml.new");
        replace(&self.path, &staged, &written).map_err(|e| self.failing(e))
    }
}

fn replace(path: &Path, staged: &Path, written: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Err(e) = fs::write(staged, written) {
        // Leaving it would make the next run wonder what this half-written
        // file is.
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

    fn in_a_temporary_directory() -> (TempDir, FileConfiguration) {
        let dir = TempDir::new().unwrap();
        let settings = FileConfiguration::at(dir.path().join("config.toml"));
        (dir, settings)
    }

    fn a_plan() -> StoredConfiguration {
        StoredConfiguration {
            ..Default::default()
        }
    }

    #[test]
    fn the_config_directory_wins() {
        assert_eq!(
            path_of(some("/x/.config"), some("/home/a")),
            Some(PathBuf::from("/x/.config/cistern/config.toml"))
        );
    }

    #[test]
    fn home_stands_in_where_there_is_no_config_directory() {
        assert_eq!(
            path_of(None, some("/home/a")),
            Some(PathBuf::from("/home/a/.config/cistern/config.toml"))
        );
    }

    #[test]
    fn neither_leaves_nowhere_to_put_it() {
        assert_eq!(path_of(None, None), None);
    }

    #[test]
    fn nothing_stored_reads_as_nothing_set() {
        let (_dir, settings) = in_a_temporary_directory();
        assert_eq!(settings.load(), Ok(StoredConfiguration::default()));
    }

    #[test]
    fn what_was_written_is_there_for_the_next_process_to_read() {
        let (dir, settings) = in_a_temporary_directory();
        settings.store(&a_plan()).unwrap();

        // A second reader over the same path is what a restarted core is.
        let restarted = FileConfiguration::at(dir.path().join("config.toml"));
        assert_eq!(restarted.load(), Ok(a_plan()));
    }

    #[test]
    fn a_file_that_is_not_toml_fails_rather_than_reading_as_empty() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "this is not toml").unwrap();
        assert!(settings.load().is_err());
    }

    #[test]
    fn a_key_the_specification_does_not_have_fails() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "colour = \"red\"\n").unwrap();
        assert!(settings.load().is_err());
    }

    /// A value no key takes is the core's to refuse, so the file is read.
    #[test]
    fn a_value_no_key_takes_still_reads() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "vendor = \"codex\"\n").unwrap();
        assert_eq!(
            settings.load(),
            Ok(StoredConfiguration {
                vendor: Some("codex".to_owned()),
            })
        );
    }

    /// A file can hold a TOML type the key does not take. Refusing that is the
    /// core's, so it has to arrive rather than fail on the way.
    #[test]
    fn a_value_of_another_type_reads_as_the_text_it_was_written_as() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "vendor = 123\n").unwrap();
        assert_eq!(
            settings.load(),
            Ok(StoredConfiguration {
                vendor: Some("123".to_owned()),
            })
        );
    }

    #[test]
    fn the_staged_file_does_not_outlive_the_write() {
        let (dir, settings) = in_a_temporary_directory();
        settings.store(&a_plan()).unwrap();
        assert!(!dir.path().join("config.toml.new").exists());
    }
}
