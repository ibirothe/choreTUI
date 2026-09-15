use std::process::ExitCode;

use anyhow::{Context, Result};
use choretui::{
    cli::{Cli, Command},
    config::{self, AppPaths},
    diagnostics,
    doctor::DoctorReport,
    storage::SqliteStore,
};

fn main() -> ExitCode {
    let cli = Cli::parse_env();
    diagnostics::install_panic_hook();

    let paths = match AppPaths::resolve() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("chore: {error}; set CHORETUI_CONFIG and CHORETUI_DATA_DIR explicitly");
            return ExitCode::FAILURE;
        }
    };

    if cli.command == Some(Command::Doctor) {
        let report = DoctorReport::run(&paths);
        print!("{report}");
        return ExitCode::from(report.exit_code());
    }

    match run_interactive(&paths) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chore: {error:#}");
            eprintln!("Run `chore doctor` for non-destructive diagnostics.");
            ExitCode::FAILURE
        }
    }
}

fn run_interactive(paths: &AppPaths) -> Result<()> {
    paths.prepare_runtime_dirs().with_context(|| {
        format!(
            "could not prepare application directories for {}",
            paths.database_file().display()
        )
    })?;
    let loaded = config::load(paths.config_file())?;
    let _log_guard = diagnostics::init_file(paths.state_dir())
        .context("could not initialize rolling diagnostics log")?;
    for warning in &loaded.warnings {
        eprintln!("warning: {warning}");
        tracing::warn!(message = %warning, "configuration warning");
    }
    tracing::debug!(
        database = %paths.database_file().display(),
        config = %paths.config_file().display(),
        "resolved application paths"
    );
    let store = SqliteStore::open(paths.database_file()).with_context(|| {
        format!(
            "could not open database {} without risking data loss",
            paths.database_file().display()
        )
    })?;
    choretui::app::run(store, &loaded.config).context("interactive application failed")
}
