# AGENTS.md

## Project type

Rust desktop/CLI project using Cargo.

The project may include GUI, CLI, Python bridge, PDF processing, AI-service integrations, tests, scripts, and CI configuration.

## Autonomy level

The agent is allowed to operate with high autonomy for development, debugging, repair, and validation tasks.

The agent may:

- inspect the repository
- inspect build/test/runtime logs
- modify source files
- modify project configuration
- modify tests
- modify scripts
- modify CI workflows
- run terminal commands needed for diagnosis and validation
- install or select the required Rust toolchain
- run formatting, linting, build, and test commands
- repeat the diagnose → fix → validate loop until the issue is resolved or blocked

The agent should prefer durable project-level fixes over local temporary workarounds.

## Preferred Rust toolchain

Use Rust 1.88.0 unless a task explicitly requires another version.

If `rust-toolchain.toml` contains:
