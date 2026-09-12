//! Terminal lifecycle and top-level rendering.

pub mod model;
pub mod screens;
pub mod widgets;

use std::io::{self, stdout};

use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::Alignment,
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

trait TerminalControl {
    fn enter(&mut self) -> io::Result<()>;
    fn restore(&mut self) -> io::Result<()>;
}

#[derive(Default)]
struct CrosstermControl;

impl TerminalControl for CrosstermControl {
    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;

        if let Err(error) = execute!(stdout(), EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(error);
        }

        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        restore_terminal()
    }
}

struct TerminalGuard<C: TerminalControl> {
    control: C,
    active: bool,
}

impl<C: TerminalControl> TerminalGuard<C> {
    fn enter(mut control: C) -> io::Result<Self> {
        control.enter()?;
        Ok(Self {
            control,
            active: true,
        })
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.active {
            self.active = false;
            self.control.restore()?;
        }
        Ok(())
    }
}

impl<C: TerminalControl> Drop for TerminalGuard<C> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Restore the process terminal. The operation is intentionally idempotent so
/// both the panic hook and the lifecycle guard can call it safely.
///
/// # Errors
///
/// Returns an I/O error when raw mode or alternate-screen cleanup fails.
pub fn restore_terminal() -> io::Result<()> {
    let raw_mode_result = disable_raw_mode();
    let screen_result = execute!(stdout(), LeaveAlternateScreen, Show);

    raw_mode_result.and(screen_result)
}

/// Enter the alternate screen, render a bootstrap frame, and restore the
/// terminal before returning.
///
/// # Errors
///
/// Returns an I/O error when terminal setup or rendering fails. Cleanup still
/// runs through the lifecycle guard.
pub fn run_placeholder() -> io::Result<()> {
    let _guard = TerminalGuard::enter(CrosstermControl)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    terminal.draw(|frame| {
        let message = Paragraph::new(vec![
            Line::from("ChoreTUI"),
            Line::from(""),
            Line::from("Bootstrap complete — Weekly Board coming next."),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Weekly Chores "),
        );

        frame.render_widget(message, frame.area());
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::{TerminalControl, TerminalGuard};

    struct FakeControl {
        enter_count: Arc<AtomicUsize>,
        restore_count: Arc<AtomicUsize>,
    }

    impl TerminalControl for FakeControl {
        fn enter(&mut self) -> io::Result<()> {
            self.enter_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn restore(&mut self) -> io::Result<()> {
            self.restore_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn guard_restores_once_on_normal_drop() {
        let enter_count = Arc::new(AtomicUsize::new(0));
        let restore_count = Arc::new(AtomicUsize::new(0));

        {
            let _guard = TerminalGuard::enter(FakeControl {
                enter_count: Arc::clone(&enter_count),
                restore_count: Arc::clone(&restore_count),
            })
            .expect("fake terminal should enter");
        }

        assert_eq!(enter_count.load(Ordering::SeqCst), 1);
        assert_eq!(restore_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn guard_restores_during_panic_unwind() {
        let restore_count = Arc::new(AtomicUsize::new(0));
        let observed_count = Arc::clone(&restore_count);

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = TerminalGuard::enter(FakeControl {
                enter_count: Arc::new(AtomicUsize::new(0)),
                restore_count,
            })
            .expect("fake terminal should enter");
            panic!("controlled panic");
        }));

        assert!(result.is_err());
        assert_eq!(observed_count.load(Ordering::SeqCst), 1);
    }
}
