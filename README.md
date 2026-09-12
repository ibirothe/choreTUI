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

CI runs these checks on Linux, macOS, and Windows.

## Source boundaries

- `src/domain` contains UI- and storage-independent domain code.
- `src/recurrence` contains pure recurrence calculations.
- `src/app` coordinates use cases through domain ports.
- `src/storage` implements persistence adapters.
- `src/config` owns configuration and local path resolution.
- `src/tui` owns terminal lifecycle, screen state, and rendering.

The `chore` binary is a thin process boundary around the library.
