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

use crate::core::port::{Settings, Stored, Unavailable};

/// The configuration, kept as TOML at a path fixed when this is built.
pub struct FileSettings {
    path: PathBuf,
}

/// The file, as TOML sees it.
///
/// It stands apart from the port's type because the key spelling and the
/// absent-field rules belong to the format, not to what is being stored.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(skip_serializing_if = "Option::is_none")]
    vendor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    #[serde(rename = "usage-limit", skip_serializing_if = "Option::is_none")]
    usage_limit: Option<u64>,
}

impl FileSettings {
    /// Takes the path it is given. This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileSettings { path }
    }

    /// The path `docs/cli.md` names, or nothing when there is nowhere for it.
    pub fn in_config_home() -> Option<Self> {
        path_of(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME")).map(FileSettings::at)
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

impl Settings for FileSettings {
    fn load(&self) -> Result<Stored, Unavailable> {
        let written = match fs::read_to_string(&self.path) {
            Ok(written) => written,
            // Nothing stored yet. Any other read failure is worth saying.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Stored::default()),
            Err(e) => return Err(self.failing(e)),
        };

        let document: Document = toml::from_str(&written).map_err(|e| self.failing(e))?;
        Ok(Stored {
            vendor: document.vendor,
            plan: document.plan,
            usage_limit: document.usage_limit,
        })
    }

    /// Writes beside the file and renames it into place.
    ///
    /// Opening the file truncates it first, so a process that stops after that
    /// leaves an empty one behind, and we would be the ones writing the file
    /// `load` then refuses. A rename within one filesystem is atomic, so a
    /// reader sees the old contents or the new ones and nothing between.
    fn store(&self, stored: &Stored) -> Result<(), Unavailable> {
        let document = Document {
            vendor: stored.vendor.clone(),
            plan: stored.plan.clone(),
            usage_limit: stored.usage_limit,
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

    fn in_a_temporary_directory() -> (TempDir, FileSettings) {
        let dir = TempDir::new().unwrap();
        let settings = FileSettings::at(dir.path().join("config.toml"));
        (dir, settings)
    }

    fn a_plan() -> Stored {
        Stored {
            plan: Some("max-20x".to_owned()),
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
        assert_eq!(settings.load(), Ok(Stored::default()));
    }

    #[test]
    fn what_was_written_is_there_for_the_next_process_to_read() {
        let (dir, settings) = in_a_temporary_directory();
        settings.store(&a_plan()).unwrap();

        // A second reader over the same path is what a restarted core is.
        let restarted = FileSettings::at(dir.path().join("config.toml"));
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
        fs::write(dir.path().join("config.toml"), "plan = \"max-40x\"\n").unwrap();
        assert_eq!(
            settings.load(),
            Ok(Stored {
                plan: Some("max-40x".to_owned()),
                ..Default::default()
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
