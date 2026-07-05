# Contributing

We welcome contributions to the Bank Statement Fidelity Editor!

## Development Setup

1. Follow the [QUICKSTART.md](QUICKSTART.md) guide to set up your environment.
2. Copy `.env.example` to `.env` and configure at minimum `DUAL_CORE_PASSPHRASE` and `GEMINI_API_KEY`.
3. Build and run tests:

```bash
cargo build
cargo test
```

## Code Quality

### Linting & Formatting

All code must pass clippy and rustfmt before merge:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
```

### Mutation Testing

To ensure high-quality code and robust logic, we highly recommend using mutation testing locally before submitting a PR:

```bash
cargo install cargo-mutants
cargo mutants
```

This verifies that the test suite actually catches bugs in the business logic (especially in `src/engine/balance.rs`, `src/engine/verification.rs`, and `src/engine/offline_parser.rs`).

### Full Validation

Run the complete validation suite before submitting:

```bash
cargo fmt
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Architecture Guidelines

- **Fallback chains:** Every new cloud integration must have an offline fallback. Never leave a pipeline stage with a single point of failure.
- **API availability:** New API keys must be added to `ApiAvailability` in `src/app/config.rs` and checked at boot time.
- **Backend preferences:** New backends should be added to the appropriate enum (`AiProviderMode`, `DocumentParserMode`, `VerificationMode`, `PdfEngineMode`) and surfaced in the Backend Preferences UI in `src/app/modals.rs`.
- **Error handling:** Prefer typed errors with context. No silent failures or unchecked unwraps in production paths.
- **Secrets:** Never log, print, or commit API key values. Use `.env.example` for templates.

## Files

- **`.env.example`** — Template with all configurable keys (safe to commit)
- **`.env`** — Your local secrets (gitignored, never commit)
- **`AGENTS.md`** — Agent development rules and autonomy boundaries
- **`QUICKSTART.md`** — Setup guide for new developers
- **`CHANGELOG.md`** — Release history
