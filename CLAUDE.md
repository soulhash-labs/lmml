# CLAUDE.md

## What this project is
lmml — a Rust TUI for managing llama.cpp. Full plan in `docs/lmml-plan.md`.
Read that plan before doing anything. It defines the crate structure, API shapes,
and all conventions to follow.

## Workspace bootstrap
The project is a Cargo workspace. If `Cargo.toml` doesn't exist yet, create it:

```toml
[workspace]
members = [
    "crates/lmml-detect",
    "crates/lmml-compat",
    "crates/lmml-build",
    "crates/lmml-models",
    "crates/lmml-server",
    "crates/lmml-state",
    "crates/lmml-tui",
]
resolver = "2"
```

## Build and test
```sh
cargo build -p <crate>
cargo test -p <crate>
cargo clippy -p <crate>
cargo fmt
```

## Milestone order
Build in milestone order from the plan. Do not skip ahead.
Complete all tests for a crate before moving to the next.

## Never do
- Do not add code to a crate that belongs in another crate.
- Do not use `unwrap()` or `expect()` outside tests.
- Do not hardcode llama.cpp flag names outside `lmml-compat`.
- Do not watch log lines to detect server readiness — poll `/health`.
