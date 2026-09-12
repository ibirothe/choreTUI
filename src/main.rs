use std::process::ExitCode;

use choretui::{cli::Cli, diagnostics};

fn main() -> ExitCode {
    let _cli = Cli::parse_env();

    if let Err(error) = diagnostics::init() {
        eprintln!("warning: diagnostics could not be initialized: {error}");
    }
    diagnostics::install_panic_hook();

    match choretui::app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chore: {error}");
            ExitCode::FAILURE
        }
    }
}
