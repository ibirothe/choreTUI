//! Configuration loading and platform path resolution.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::Deserialize;
use thiserror::Error;

/// Environment variable overriding the configuration file path.
pub const CONFIG_ENV: &str = "CHORETUI_CONFIG";
/// Environment variable overriding the application data directory.
pub const DATA_DIR_ENV: &str = "CHORETUI_DATA_DIR";

/// Fully resolved local paths used by `ChoreTUI`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    config_file: PathBuf,
    data_dir: PathBuf,
    database_file: PathBuf,
    state_dir: PathBuf,
    log_file: PathBuf,
}

impl AppPaths {
    /// Resolve paths from platform conventions and supported overrides.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::Unavailable`] when the platform has no usable
    /// project directories.
    pub fn resolve() -> Result<Self, PathError> {
        let project = ProjectDirs::from("", "", "choretui").ok_or(PathError::Unavailable)?;
        let roots = PathRoots {
            config: project.config_dir().to_owned(),
            data: project.data_dir().to_owned(),
            state: project
                .state_dir()
                .unwrap_or_else(|| project.data_local_dir())
                .to_owned(),
        };
        let overrides = EnvOverrides {
            config_file: std::env::var_os(CONFIG_ENV),
            data_dir: std::env::var_os(DATA_DIR_ENV),
        };
        Ok(Self::from_roots(roots, overrides))
    }

    fn from_roots(roots: PathRoots, overrides: EnvOverrides) -> Self {
        let config_file = overrides
            .config_file
            .map_or_else(|| roots.config.join("config.toml"), PathBuf::from);
        let data_dir = overrides.data_dir.map(PathBuf::from).unwrap_or(roots.data);
        Self {
            database_file: data_dir.join("choretui.db"),
            log_file: roots.state.join("choretui.log"),
            config_file,
            data_dir,
            state_dir: roots.state,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_paths(config_file: PathBuf, data_dir: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            database_file: data_dir.join("choretui.db"),
            log_file: state_dir.join("choretui.log"),
            config_file,
            data_dir,
            state_dir,
        }
    }

    /// Create data and state directories without creating a configuration
    /// file.
    ///
    /// # Errors
    ///
    /// Returns the path and OS error when a directory cannot be prepared.
    pub fn prepare_runtime_dirs(&self) -> Result<(), PathError> {
        prepare_directory(&self.data_dir)?;
        prepare_directory(&self.state_dir)
    }

    /// Return the optional configuration file path.
    #[must_use]
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    /// Return the application data directory.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Return the SQLite database file path.
    #[must_use]
    pub fn database_file(&self) -> &Path {
        &self.database_file
    }

    /// Return the application state/log directory.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Return the logical rolling log path.
    #[must_use]
    pub fn log_file(&self) -> &Path {
        &self.log_file
    }
}

#[derive(Debug)]
struct PathRoots {
    config: PathBuf,
    data: PathBuf,
    state: PathBuf,
}

#[derive(Debug, Default)]
struct EnvOverrides {
    config_file: Option<OsString>,
    data_dir: Option<OsString>,
}

/// Failure while resolving or preparing application paths.
#[derive(Debug, Error)]
pub enum PathError {
    /// Standard application directories are unavailable.
    #[error("platform application directories are unavailable")]
    Unavailable,
    /// A runtime directory could not be created or secured.
    #[error("cannot prepare directory {}: {source}", path.display())]
    Prepare {
        /// Affected path.
        path: PathBuf,
        /// Underlying OS failure.
        #[source]
        source: io::Error,
    },
}

fn prepare_directory(path: &Path) -> Result<(), PathError> {
    fs::create_dir_all(path).map_err(|source| PathError::Prepare {
        path: path.to_owned(),
        source,
    })?;
    #[cfg(unix)]
    restrict_permissions(path)?;

    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), PathError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
        PathError::Prepare {
            path: path.to_owned(),
            source,
        }
    })
}

/// Secondary date-label presentation.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    /// Canonical `YYYY-MM-DD` dates.
    #[default]
    Iso,
    /// Operating-system locale formatting.
    Locale,
}

/// Typed user configuration with stable defaults.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Require confirmation before soft deletion.
    pub confirm_delete: bool,
    /// Include completed occurrences on the board.
    pub show_completed: bool,
    /// Secondary date-label presentation.
    pub date_format: DateFormat,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            confirm_delete: true,
            show_completed: true,
            date_format: DateFormat::Iso,
        }
    }
}

/// A successfully loaded configuration and its non-blocking warnings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedConfig {
    /// Parsed values, including defaults for missing keys.
    pub config: Config,
    /// Unknown-key warnings to display once during startup.
    pub warnings: Vec<String>,
}

/// Failure reading or parsing a present configuration file.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The file exists but cannot be read.
    #[error("cannot read configuration {}: {source}", path.display())]
    Read {
        /// Configuration path.
        path: PathBuf,
        /// Underlying OS failure.
        #[source]
        source: io::Error,
    },
    /// TOML syntax or a typed value is invalid.
    #[error("invalid configuration {}: {source}", path.display())]
    Parse {
        /// Configuration path.
        path: PathBuf,
        /// TOML diagnostic with source span when available.
        #[source]
        source: toml::de::Error,
    },
}

/// Load an optional TOML file. A missing file returns defaults and is not
/// created.
///
/// # Errors
///
/// Returns an error for unreadable files, invalid syntax, or invalid typed
/// values.
pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadedConfig {
                config: Config::default(),
                warnings: Vec::new(),
            });
        }
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };

    let mut warnings = Vec::new();
    let deserializer =
        toml::de::Deserializer::parse(&source).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
    let config = serde_ignored::deserialize(deserializer, |unknown| {
        warnings.push(format!("unknown configuration key `{unknown}`"));
    })
    .map_err(|source| ConfigError::Parse {
        path: path.to_owned(),
        source,
    })?;
    Ok(LoadedConfig { config, warnings })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir_in;

    use super::*;

    fn temporary_directory() -> tempfile::TempDir {
        tempdir_in(std::env::current_dir().expect("working directory should exist"))
            .expect("temporary directory should exist")
    }

    #[test]
    fn missing_configuration_uses_defaults_without_creating_a_file() {
        let directory = temporary_directory();
        let path = directory.path().join("missing.toml");

        assert_eq!(
            load(&path).expect("defaults should load").config,
            Config::default()
        );
        assert!(!path.exists());
    }

    #[test]
    fn valid_values_and_unknown_key_warning_are_reported() {
        let directory = temporary_directory();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "confirm_delete = false\nshow_completed = false\ndate_format = \"locale\"\nfuture_key = 1\n",
        )
        .expect("fixture should write");

        let loaded = load(&path).expect("configuration should load");
        assert_eq!(
            loaded.config,
            Config {
                confirm_delete: false,
                show_completed: false,
                date_format: DateFormat::Locale,
            }
        );
        assert_eq!(loaded.warnings, ["unknown configuration key `future_key`"]);
    }

    #[test]
    fn invalid_value_reports_path_and_source_location() {
        let directory = temporary_directory();
        let path = directory.path().join("broken.toml");
        fs::write(&path, "confirm_delete = \"yes\"\n").expect("fixture should write");

        let message = load(&path)
            .expect_err("invalid value should fail")
            .to_string();
        assert!(message.contains(path.to_string_lossy().as_ref()));
        assert!(message.contains("boolean"));
        assert!(message.contains("line 1"));
    }

    #[test]
    fn path_overrides_are_isolated_and_do_not_change_state_location() {
        let directory = temporary_directory();
        let root = directory.path();
        let roots = PathRoots {
            config: root.join("xdg/config/choretui"),
            data: root.join("xdg/data/choretui"),
            state: root.join("xdg/state/choretui"),
        };
        let paths = AppPaths::from_roots(
            roots,
            EnvOverrides {
                config_file: Some(root.join("override/settings.toml").into_os_string()),
                data_dir: Some(root.join("override/data").into_os_string()),
            },
        );

        assert_eq!(paths.config_file(), root.join("override/settings.toml"));
        assert_eq!(
            paths.database_file(),
            root.join("override/data/choretui.db")
        );
        assert_eq!(paths.state_dir(), root.join("xdg/state/choretui"));
    }
}
