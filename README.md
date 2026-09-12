# ChoreTUI

ChoreTUI is a local, keyboard-driven terminal application for recurring weekly chores. The product and architecture baseline lives in [the development specification](docs/specification.md).

The repository is currently in its bootstrap phase. The binary enters a safely managed alternate terminal screen, renders a placeholder frame, and exits; chore functionality is implemented by the follow-up issues linked from the specification.

## Prerequisites

- Current stable Rust toolchain with `rustfmt` and Clippy

The checked-in `rust-toolchain.toml` selects the expected toolchain and components when using rustup.

## Build and run

```sh
cargo build --locked
cargo run --locked -- --help
cargo run --locked -- --version
cargo run --locked
```

## Quality checks

Run the same gates as CI before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
```

Install the repository hooks once and run them explicitly when needed:

```sh
python -m pip install pre-commit==4.6.2
pre-commit install
pre-commit run --all-files
```

The pre-commit configuration runs formatting, strict Clippy, and tests with the
locked dependency graph. CI executes the same hooks on Linux and the complete
quality suite on Linux, macOS, and Windows.

## Source boundaries

- `src/domain` contains UI- and storage-independent domain code.
- `src/recurrence` contains pure recurrence calculations.
- `src/app` coordinates use cases through domain ports.
- `src/storage` implements persistence adapters.
- `src/config` owns configuration and local path resolution.
- `src/tui` owns terminal lifecycle, screen state, and rendering.

The `chore` binary is a thin process boundary around the library.
