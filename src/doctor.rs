//! Read-only installation and data diagnostics.

use std::{fmt, fs, path::Path};

use crate::{
    config::{self, AppPaths},
    storage::{LATEST_VERSION, SqliteStore},
};

/// Outcome of one diagnostic check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    /// Check succeeded.
    Pass,
    /// Non-blocking condition worth reporting.
    Warning,
    /// User, configuration, or data problem.
    Fail,
}

impl fmt::Display for CheckStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pass => "PASS",
            Self::Warning => "WARN",
            Self::Fail => "FAIL",
        })
    }
}

/// One named doctor result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctorCheck {
    /// Short check name.
    pub name: &'static str,
    /// Pass, warning, or failure.
    pub status: CheckStatus,
    /// Concise result and suggested action where relevant.
    pub detail: String,
}

/// Complete read-only doctor report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctorReport {
    /// Checks in display order.
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// Run path, configuration, and database diagnostics without creating files
    /// or changing domain data.
    #[must_use]
    pub fn run(paths: &AppPaths) -> Self {
        let mut report = Self::default();
        report.check_path(
            "config path",
            paths.config_file(),
            "optional file is absent; built-in defaults will be used",
        );
        report.check_path(
            "data path",
            paths.data_dir(),
            "directory does not exist yet; startup will create it",
        );
        report.check_path(
            "state path",
            paths.state_dir(),
            "directory does not exist yet; startup will create it",
        );
        report.check_configuration(paths.config_file());
        report.check_database(paths.database_file());
        report
    }

    /// Return the documented process exit code: zero when no check failed,
    /// otherwise one.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        u8::from(
            self.checks
                .iter()
                .any(|check| check.status == CheckStatus::Fail),
        )
    }

    fn check_path(&mut self, name: &'static str, path: &Path, missing_detail: &'static str) {
        let (status, detail) = match fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() || metadata.is_file() => {
                (CheckStatus::Pass, path.display().to_string())
            }
            Ok(_) => (
                CheckStatus::Fail,
                format!("{} is not a regular file or directory", path.display()),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                CheckStatus::Pass,
                format!("{}: {missing_detail}", path.display()),
            ),
            Err(error) => (
                CheckStatus::Fail,
                format!("cannot inspect {}: {error}", path.display()),
            ),
        };
        self.checks.push(DoctorCheck {
            name,
            status,
            detail,
        });
    }

    fn check_configuration(&mut self, path: &Path) {
        match config::load(path) {
            Ok(loaded) => {
                self.checks.push(DoctorCheck {
                    name: "configuration",
                    status: CheckStatus::Pass,
                    detail: if path.exists() {
                        format!("{} is valid", path.display())
                    } else {
                        "using built-in defaults".to_owned()
                    },
                });
                self.checks
                    .extend(loaded.warnings.into_iter().map(|warning| DoctorCheck {
                        name: "configuration",
                        status: CheckStatus::Warning,
                        detail: warning,
                    }));
            }
            Err(error) => self.checks.push(DoctorCheck {
                name: "configuration",
                status: CheckStatus::Fail,
                detail: format!("{error}; fix the file and run `chore doctor` again"),
            }),
        }
    }

    fn check_database(&mut self, path: &Path) {
        if !path.exists() {
            self.checks.push(DoctorCheck {
                name: "database",
                status: CheckStatus::Pass,
                detail: format!(
                    "{} does not exist yet; startup will create it",
                    path.display()
                ),
            });
            return;
        }

        match SqliteStore::inspect_read_only(path) {
            Ok(diagnostics) => {
                self.checks.push(DoctorCheck {
                    name: "schema",
                    status: if diagnostics.schema_version == LATEST_VERSION {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                    detail: format!(
                        "version {} (supported {})",
                        diagnostics.schema_version, LATEST_VERSION
                    ),
                });
                self.checks.push(DoctorCheck {
                    name: "foreign keys",
                    status: if diagnostics.foreign_keys {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                    detail: if diagnostics.foreign_keys {
                        "enabled".to_owned()
                    } else {
                        "disabled on diagnostic connection".to_owned()
                    },
                });
                let integrity_ok = diagnostics.integrity == ["ok"];
                self.checks.push(DoctorCheck {
                    name: "integrity",
                    status: if integrity_ok {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                    detail: if integrity_ok {
                        "ok".to_owned()
                    } else {
                        format!(
                            "{}; keep the database unchanged and restore from a known-good backup",
                            diagnostics.integrity.join("; ")
                        )
                    },
                });
            }
            Err(error) => self.checks.push(DoctorCheck {
                name: "database",
                status: CheckStatus::Fail,
                detail: format!(
                    "cannot safely inspect {}: {error}; keep the file unchanged and check permissions or restore a backup",
                    path.display()
                ),
            }),
        }
    }
}

impl fmt::Display for DoctorReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "STATUS  CHECK           DETAIL")?;
        for check in &self.checks {
            writeln!(
                formatter,
                "{:<6}  {:<14}  {}",
                check.status, check.name, check.detail
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::tempdir_in;

    use super::*;

    fn fixture() -> (tempfile::TempDir, AppPaths) {
        let directory =
            tempdir_in(std::env::current_dir().expect("working directory should exist"))
                .expect("temporary directory should exist");
        let root = directory.path();
        let paths = AppPaths::test_paths(
            root.join("config/config.toml"),
            root.join("data"),
            root.join("state"),
        );
        (directory, paths)
    }

    fn failure_detail(report: &DoctorReport) -> String {
        report
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Fail)
            .map(|check| check.detail.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn fresh_install_is_healthy_and_remains_unmodified() {
        let (_directory, paths) = fixture();

        let report = DoctorReport::run(&paths);

        assert_eq!(report.exit_code(), 0);
        assert!(!paths.config_file().exists());
        assert!(!paths.database_file().exists());
        assert!(!paths.state_dir().exists());
    }

    #[test]
    fn valid_database_is_checked_without_changing_domain_or_schema_data() {
        let (_directory, paths) = fixture();
        paths
            .prepare_runtime_dirs()
            .expect("runtime directories should be created");
        drop(SqliteStore::open(paths.database_file()).expect("database should open"));
        let before = fs::metadata(paths.database_file())
            .expect("database metadata should load")
            .len();

        let report = DoctorReport::run(&paths);

        assert_eq!(report.exit_code(), 0);
        assert!(report.checks.iter().any(|check| {
            check.name == "integrity" && check.status == CheckStatus::Pass && check.detail == "ok"
        }));
        assert_eq!(
            fs::metadata(paths.database_file())
                .expect("database metadata should load")
                .len(),
            before
        );
        let connection = Connection::open_with_flags(
            paths.database_file(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("database should reopen read-only");
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("migration count should load"),
            1
        );
    }

    #[test]
    fn invalid_configuration_is_actionable_and_fails_doctor() {
        let (_directory, paths) = fixture();
        fs::create_dir_all(
            paths
                .config_file()
                .parent()
                .expect("config parent should exist"),
        )
        .expect("config directory should be created");
        fs::write(paths.config_file(), "date_format = \"american\"\n")
            .expect("fixture should write");

        let report = DoctorReport::run(&paths);
        let detail = failure_detail(&report);

        assert_eq!(report.exit_code(), 1);
        assert!(detail.contains(paths.config_file().to_string_lossy().as_ref()));
        assert!(detail.contains("iso"));
        assert!(detail.contains("locale"));
    }

    #[test]
    fn unknown_configuration_key_is_only_a_warning() {
        let (_directory, paths) = fixture();
        fs::create_dir_all(
            paths
                .config_file()
                .parent()
                .expect("config parent should exist"),
        )
        .expect("config directory should be created");
        fs::write(paths.config_file(), "future_option = true\n").expect("fixture should write");

        let report = DoctorReport::run(&paths);

        assert_eq!(report.exit_code(), 0);
        assert!(report.checks.iter().any(|check| {
            check.status == CheckStatus::Warning && check.detail.contains("future_option")
        }));
    }

    #[test]
    fn newer_and_corrupt_databases_fail_without_replacement() {
        let (_directory, paths) = fixture();
        fs::create_dir_all(paths.data_dir()).expect("data directory should be created");
        let connection = Connection::open(paths.database_file()).expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL); \
                 INSERT INTO schema_migrations VALUES (99, 'future');",
            )
            .expect("future fixture should write");
        drop(connection);

        let newer = DoctorReport::run(&paths);
        assert_eq!(newer.exit_code(), 1);
        assert!(failure_detail(&newer).contains("newer than supported"));

        fs::write(paths.database_file(), b"not a sqlite database")
            .expect("corrupt fixture should write");
        let corrupt = DoctorReport::run(&paths);
        assert_eq!(corrupt.exit_code(), 1);
        assert!(failure_detail(&corrupt).contains("keep the file unchanged"));
        assert_eq!(
            fs::read(paths.database_file()).expect("fixture should remain readable"),
            b"not a sqlite database"
        );
    }
}
