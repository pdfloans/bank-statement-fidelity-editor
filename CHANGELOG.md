# Changelog

All notable changes to this project will be documented in this file.

## [v0.4.0] - 2026-05-26

### Added
- **Smart Balance Engine**: Added the cloud-required Smart Balance Engine pipeline (Document AI -> Hybrid Merging -> Balance Math -> Gemini Proposals).
- **Hybrid Extraction Layer**: Implemented three geometry providers (`Tesseract`, `PyMuPDF Heuristics`, `Bank Templates`) and a `HybridMerger` to map semantic transactions to spatial bounding boxes.
- **PdfEngine Trait Abstraction**: Abstracted all PDF manipulation behind a `PdfEngine` trait with a primary `MuPdfEngine` and fallback `PyMuPdfEngine`.
- **Document AI JWT**: Real RS256 JWT generation and token exchange for Google Cloud Document AI.
- **Structured Telemetry**: Replaced print debugging with process-wide `tracing` macros, daily file rotation, and an optional OpenTelemetry (OTLP) gRPC exporter.
- **CLI Parity**: Full feature parity for the CLI, including new `balance --auto-approve` and `extract` subcommands with typed JSON outputs.
- **Verification Math**: Hardened verification tests to compute mathematical drift correctly against the original unmodified PDF's expected balance.
- **5 Bank Templates**: Shipped default templates for Chase, Bank of America, Wells Fargo, Citibank, and Capital One.

### Changed
- **Unified Engine**: The `SmartDocumentEngine` now serves as the central unified engine for all operations, treating multi-page statements as one connected document.
- **History Simplification**: Consolidated the `ChangeHistory` into a single process-wide source of truth orchestrated by the Tokio runtime. Fixed the auto-undo GUI bug.
- **PyO3 Architecture**: The PyO3 bridge now relies on a non-panicking, dedicated actor thread to isolate Python interpreter state.
- **Gemini Rest Client**: Updated to the stable `v1beta` Google Gemini REST API using exact `camelCase` fields and typed object schemas for `responseSchema`.
- **Refactoring**: Moved from a three-layer architecture to a mature five-layer architecture (`app/`, `engine/`, `pdf/`, `extractors/`, `ai/`, `security/`).

### Removed
- **Legacy Stubs**: Removed placeholder parsing and fake manual heuristics in Python integration and text_editor components.
