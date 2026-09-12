//! Command-line parsing.

use clap::{Parser, Subcommand};

/// Manage recurring chores from a keyboard-driven terminal interface.
#[derive(Debug, Parser)]
#[command(name = "chore", version, about)]
pub struct Cli {
    /// Optional diagnostic command; no command opens the TUI.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Supported non-interactive commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Validate paths, configuration, and SQLite without changing domain data.
    Doctor,
}

impl Cli {
    /// Parse command-line arguments from the process environment.
    #[must_use]
    pub fn parse_env() -> Self {
        Self::parse()
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser, error::ErrorKind};

    use super::{Cli, Command};

    #[test]
    fn clap_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn doctor_subcommand_and_usage_errors_follow_cli_contract() {
        assert_eq!(
            Cli::try_parse_from(["chore", "doctor"])
                .expect("doctor should parse")
                .command,
            Some(Command::Doctor)
        );
        assert_eq!(
            Cli::try_parse_from(["chore", "unknown"])
                .expect_err("unknown command should fail")
                .kind(),
            ErrorKind::InvalidSubcommand
        );
    }
}
