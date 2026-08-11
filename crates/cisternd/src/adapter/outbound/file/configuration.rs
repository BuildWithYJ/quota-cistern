//! The configuration file.
//!
//! The only place that knows the path and the file format.
//! Neither reaches the core.

use std::{env, path::PathBuf};

use crate::core::port::outbound::{ConfigurationStore, Unavailable};

use super::kept::Kept;

/// The configuration, kept as TOML at a path fixed when this is built.
pub struct FileConfiguration {
    kept: Kept,
}

/// The text a user would have typed for what TOML holds.
///
/// A string keeps its contents; everything else is rendered as the file writes it.
/// A number, a boolean, and a table all reach the core as something it can read and refuse.
fn as_text(value: toml::Value) -> String {
    match value {
        toml::Value::String(text) => text,
        other => other.to_string(),
    }
}

/// What the file is called where `docs/cli.md` says it is kept.
const NAMED: &str = "config.toml";

impl FileConfiguration {
    /// Takes the path it is given.
    /// This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileConfiguration {
            kept: Kept::at(path),
        }
    }

    /// The path `docs/cli.md` names, or nothing when there is nowhere for it.
    pub fn in_config_home() -> Option<Self> {
        Kept::in_config_home(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"), NAMED)
            .map(FileConfiguration::at)
    }
}

impl ConfigurationStore for FileConfiguration {
    /// Every key the file holds, whether or not the core has heard of it.
    ///
    /// A table gives its keys back in the order they were written, which is the order
    /// `config get` prints them in.
    fn load(&self) -> Result<Vec<(String, String)>, Unavailable> {
        // Nothing stored yet is an empty configuration rather than a failure.
        let Some(written) = self.kept.read()? else {
            return Ok(Vec::new());
        };

        let document: toml::Table = toml::from_str(&written).map_err(|e| self.kept.failing(e))?;
        Ok(document
            .into_iter()
            .map(|(key, value)| (key, as_text(value)))
            .collect())
    }

    fn store(&self, stored: &[(String, String)]) -> Result<(), Unavailable> {
        let document: toml::Table = stored
            .iter()
            .map(|(key, value)| (key.clone(), toml::Value::String(value.clone())))
            .collect();
        let written = toml::to_string(&document).map_err(|e| self.kept.failing(e))?;
        self.kept.write(&written)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn in_a_temporary_directory() -> (TempDir, FileConfiguration) {
        let dir = TempDir::new().unwrap();
        let settings = FileConfiguration::at(dir.path().join("config.toml"));
        (dir, settings)
    }

    fn a_setting() -> Vec<(String, String)> {
        vec![("vendor".to_owned(), "claude".to_owned())]
    }

    /// Where the file goes is `kept`'s; which file it is belongs here.
    #[test]
    fn the_file_is_the_one_the_specification_names() {
        let path = Kept::in_config_home(Some(std::ffi::OsString::from("/x/.config")), None, NAMED)
            .unwrap();
        assert_eq!(path, PathBuf::from("/x/.config/cistern/config.toml"));
    }

    #[test]
    fn nothing_stored_reads_as_nothing_set() {
        let (_dir, settings) = in_a_temporary_directory();
        assert_eq!(settings.load(), Ok(Vec::new()));
    }

    #[test]
    fn what_was_written_is_there_for_the_next_process_to_read() {
        let (dir, settings) = in_a_temporary_directory();
        settings.store(&a_setting()).unwrap();

        // A second reader over the same path is what a restarted core is.
        let restarted = FileConfiguration::at(dir.path().join("config.toml"));
        assert_eq!(restarted.load(), Ok(a_setting()));
    }

    #[test]
    fn a_file_that_is_not_toml_fails_rather_than_reading_as_empty() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "this is not toml").unwrap();
        assert!(settings.load().is_err());
    }

    /// A key section 2.5 does not name is the core's to refuse, so it arrives.
    /// Refusing it here would answer with a code the specification gives to something else.
    #[test]
    fn a_key_the_specification_does_not_have_still_reads() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "colour = \"red\"\n").unwrap();
        assert_eq!(
            settings.load(),
            Ok(vec![("colour".to_owned(), "red".to_owned())])
        );
    }

    /// A value no key takes is the core's to refuse, so the file is read.
    #[test]
    fn a_value_no_key_takes_still_reads() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "vendor = \"codex\"\n").unwrap();
        assert_eq!(
            settings.load(),
            Ok(vec![("vendor".to_owned(), "codex".to_owned())])
        );
    }

    /// A file can hold a TOML type the key does not take.
    /// Refusing that is the core's, so it has to arrive rather than fail on the way.
    #[test]
    fn a_value_of_another_type_reads_as_the_text_it_was_written_as() {
        let (dir, settings) = in_a_temporary_directory();
        fs::write(dir.path().join("config.toml"), "vendor = 123\n").unwrap();
        assert_eq!(
            settings.load(),
            Ok(vec![("vendor".to_owned(), "123".to_owned())])
        );
    }

    #[test]
    fn the_staged_file_does_not_outlive_the_write() {
        let (dir, settings) = in_a_temporary_directory();
        settings.store(&a_setting()).unwrap();
        assert!(!dir.path().join("config.toml.new").exists());
    }
}
