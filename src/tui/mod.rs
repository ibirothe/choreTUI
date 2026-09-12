//! Terminal lifecycle and top-level rendering.

pub mod model;
pub mod screens;
pub mod widgets;

use std::io::{self, stdout};

use crossterm::{
    cursor::{Hide, Show},
    event::{Event, read},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use self::model::{BoardCommand, BoardLayout, BoardState, input_for_key};

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

/// Run the Weekly Board event loop and restore the terminal before returning.
///
/// # Errors
///
/// Returns an I/O error when terminal setup or rendering fails. Cleanup still
/// runs through the lifecycle guard.
pub fn run(mut state: BoardState) -> io::Result<()> {
    let _guard = TerminalGuard::enter(CrosstermControl)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    loop {
        terminal.draw(|frame| screens::board::render(frame, frame.area(), &mut state))?;
        let Event::Key(key) = read()? else {
            continue;
        };
        let Some(input) = input_for_key(key) else {
            continue;
        };
        let area = terminal.size()?;
        let layout = BoardLayout::for_size(area.width, area.height);
        match state.handle_input(input, layout) {
            Ok(Some(BoardCommand::Quit)) => break,
            Ok(Some(BoardCommand::LoadWeek(week))) => state.replace_week(week, Vec::new()),
            Ok(Some(BoardCommand::Help)) => state.set_status(Some(
                "Board: arrows/Vim navigate; [/] weeks; Space toggles; q quits".to_owned(),
            )),
            Ok(Some(_)) => {
                state.set_status(Some("Action ready for the application layer".to_owned()));
            }
            Ok(None) => state.set_status(None),
            Err(error) => state.set_status(Some(error.to_string())),
        }
    }

    terminal.show_cursor()
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
