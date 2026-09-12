//! Command-line parsing.

use clap::Parser;

/// Manage recurring chores from a keyboard-driven terminal interface.
#[derive(Debug, Parser)]
#[command(name = "chore", version, about)]
pub struct Cli {}

impl Cli {
    /// Parse command-line arguments from the process environment.
    #[must_use]
    pub fn parse_env() -> Self {
        Self::parse()
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn clap_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }
}
