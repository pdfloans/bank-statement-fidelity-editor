# Tests

These tests define the strict business logic invariants of the bank statement modification engine.

## Test Categories

### Unit Tests (`cargo test --lib`)
Core business logic tests embedded in source modules:
- `balance.rs` — Balance math invariants (rejecting dual debit/credit, negative balances, cascading reconciliation).
- `offline_parser.rs` — Amount extraction, date parsing, row classification.
- `number_format.rs` — Locale-aware number formatting and rounding.
- `verification.rs` — SSIM scoring, tile-max diff, perceptual hash, Applitools graceful degradation.
- `font_analysis.rs` — Font metric extraction and matching.
- `model.rs` — Transaction model serialization and invariants.
- `config.rs` — Configuration loading, API availability detection, mode enum defaults.

### Integration Tests (`tests/`)
- `chaos_fallback.rs` — Verifies `PdfEngineSelector` gracefully handles complete engine failures and falls through the fallback chain (PyMuPDF → Pdfium → error, not panic).

### Characterization Tests
**CRITICAL:** These tests must continue to pass through every subsequent ticket. No work is permitted to alter the tested invariants (e.g. rejecting both Debit and Credit on the same line, rejecting negative balances, etc.).

## Running Tests

```bash
# All tests
cargo test

# With all features (including OCR)
cargo test --all-features

# Specific module
cargo test --lib balance

# Integration tests only
cargo test --test chaos_fallback
```

## Validation Suite

Full validation (run before PR):

```bash
cargo fmt
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
