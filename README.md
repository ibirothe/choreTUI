# ChoreTUI

ChoreTUI is a local, keyboard-driven terminal application for recurring chores. It presents an ISO-week board, stores data in SQLite, works offline, and keeps historical occurrence snapshots stable when chore definitions change.

## Install

Install a current stable Rust toolchain with `rustfmt` and Clippy, then build or install the locked project:

```sh
cargo build --release --locked
cargo install --path . --locked
```

The binary is named `chore`.

## Usage

```text
chore              Open the interactive Weekly Board
chore doctor       Check paths, configuration, database access, schema,
                   foreign keys, and SQLite integrity without changing data
chore --help       Show command-line help
chore --version    Show the installed version
```

The Weekly Board is the startup screen. Press `?` for contextual, scrollable help from any main screen.

### Weekly Board keys

| Key | Action |
|---|---|
| Arrows or `h`/`j`/`k`/`l` | Navigate days and chores |
| `Space` | Toggle pending/completed |
| `a` / `e` | Add / edit a chore |
| `d` / `D` | Disable / soft-delete a chore |
| `c` | Open the Chore List |
| `[` / `]`, `PageUp` / `PageDown` | Previous / next ISO week |
| `t` | Current week |
| `?` | Contextual help |
| `q` | Quit |

The editor uses `Tab`/`BackTab` for focus, arrows for choices, `Space` for checkboxes, `Ctrl+S` to save, and `Esc` to cancel. The Chore List uses `/` to filter, `Space` to enable or disable, `D` to soft-delete, and `x` to include deleted chores.

## Configuration

The optional `config.toml` supports:

```toml
confirm_delete = true
show_completed = true
date_format = "iso"
```

Unknown keys produce a warning. Invalid syntax or values stop startup rather than silently changing behavior. Override locations with `CHORETUI_CONFIG` and `CHORETUI_DATA_DIR`.

Default locations follow platform conventions:

| Data | Linux example | Override |
|---|---|---|
| Configuration | `~/.config/choretui/config.toml` | `CHORETUI_CONFIG` |
| Database | `~/.local/share/choretui/choretui.db` | `CHORETUI_DATA_DIR` |
| Diagnostics | platform state directory, `choretui/choretui.log` | platform state directory |

Use `chore doctor` to print the exact resolved paths and non-destructively diagnose configuration or database problems.

## Backup and recovery

Close ChoreTUI before copying `choretui.db`; also copy any adjacent `-wal` and `-shm` files if they exist. Keep the original database unchanged when `doctor` reports corruption, a newer schema, or an unreadable path. Restore from a verified backup to a separate location first—ChoreTUI never silently recreates or overwrites an unreadable database.

## MVP limits

- Single local user and local SQLite storage; no sync or accounts.
- ISO weeks always start Monday.
- Deleted chores are retained for history and are read-only.
- No hard delete, history editing, occurrence move/skip, import/export, analytics, or notifications.
- Recurrence supports weekly/every-N-weeks, every-N-days, and monthly day-of-month schedules.

## Development and quality checks

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
```

Install and run the repository hooks with:

```sh
python -m pip install pre-commit==4.6.2
pre-commit install
pre-commit run --all-files
```

CI runs pre-commit, strict formatting/Clippy/tests/release builds on Linux, macOS, and Windows, plus dependency advisory and license checks. The detailed product and architecture contract is in [docs/specification.md](docs/specification.md).
