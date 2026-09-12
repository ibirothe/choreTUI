//! Tracing setup and panic-time terminal recovery.

use std::{panic, sync::Once};

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
