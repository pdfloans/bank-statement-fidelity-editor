# Contributing

We welcome contributions to the Bank Statement Fidelity Editor!

## Development Tools

To ensure high-quality code and robust logic, we highly recommend using mutation testing locally before submitting a PR.

```bash
cargo install cargo-mutants
cargo mutants
```

This verifies that the test suite actually catches bugs in the business logic (especially in `src/engine/balance.rs` and `src/engine/verification.rs`).
