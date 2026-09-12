//! Tracing setup and panic-time terminal recovery.

use std::{panic, path::Path, sync::Once};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, util::SubscriberInitExt};

static PANIC_HOOK: Once = Once::new();

/// Install a process-wide tracing subscriber.
///
/// # Errors
///
/// Returns an error when another process-wide subscriber is already installed.
pub fn init() -> Result<(), tracing_subscriber::util::TryInitError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .finish()
        .try_init()
}

/// Install a process-wide subscriber writing only to a daily rolling local log.
///
/// The returned guard must remain alive until process shutdown so buffered
/// records are flushed.
///
/// # Errors
///
/// Returns an error when another process-wide subscriber is already installed.
pub fn init_file(
    state_directory: &Path,
) -> Result<WorkerGuard, tracing_subscriber::util::TryInitError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let appender = tracing_appender::rolling::daily(state_directory, "choretui.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true)
        .with_writer(writer)
        .finish()
        .try_init()?;
    Ok(guard)
}

/// Ensure the terminal is restored before the normal panic report is printed.
pub fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = crate::tui::restore_terminal();
            previous(info);
        }));
    });
}
