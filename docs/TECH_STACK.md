# Technology Stack Breakdown

This document provides a detailed overview of the specific technologies, frameworks, and libraries used to build the Bank Statement Fidelity Editor (`dual-core-pdf-pipeline` v0.5.1).

## Core Languages

- **Rust (Edition 2021):** The primary language for the application logic, GUI, and high-performance asynchronous orchestration. Rust guarantees memory safety, fast execution, and strict concurrency control.
- **Python (3.10+):** Used selectively for deep integrations with proprietary or C-based PDF rendering and manipulation libraries (like PyMuPDF Pro) that don't have native Rust equivalents. Runs inside a dedicated PyO3 actor thread.
- **Node.js (18+):** Used for the Applitools Eyes visual AI bridge (`src/ai/applitools_bridge.js`). Invoked as a child process from Rust for visual diff testing.

## GUI Framework (Native Desktop)

- **`egui` (0.28):** A highly performant, immediate-mode GUI framework built purely in Rust. Chosen for its simplicity, speed, and seamless integration with Rust data structures, making it perfect for rapid data-driven UI development.
- **`eframe` (0.28):** The official framework wrapper around `egui` to run it as a native desktop application (handling the OS windowing, WebGL/WGPU rendering contexts).
- **`egui_extras`:** Extended widgets including `TableBuilder` for the editable transaction table, and image loading for rendered PDF pages.

## PDF Processing Engines

The application employs a multi-engine strategy with automatic fallback, managed by `PdfEngineSelector`:

- **`PyMuPDF` / `pymupdfpro` (via `pyo3`):** The primary engine. Called from Rust via Python bindings. Capable of high-fidelity, per-segment redaction and text insertion while accurately reusing the exact embedded font dictionaries and glyph metrics. The Pro tier adds enhanced font handling (gated by `PYMUPDF_PRO_KEY`).
- **`pdfium-render` (0.8.x):** Rust bindings to Google's open-source `pdfium` C++ library (the same engine used in Google Chrome). Used as the fallback engine for rendering, text extraction, and editing when PyMuPDF is unavailable.
- **`lopdf` (0.34):** A pure-Rust library for low-level PDF dictionary manipulation. Used for the Split & Merge engine, extracting pages and merging them back together without altering visual fidelity or dropping fonts.
- **Typst Engine (`subsetter` + `typst`):** Ultimate fail-safe reconstruction engine. When both PyMuPDF and Pdfium fail to apply edits, this engine rebuilds the PDF from scratch using Typst typesetting with font subsetting. Always available as the last resort.

### Engine Mode Hierarchy

```
PdfEngineSelector:
  Auto (default) → PyMuPDF primary, Pdfium fallback
  DualConcurrent  → Both engines in parallel, prefer PyMuPDF
  NativeOnly      → Pdfium only
  PyMuPdfOnly     → PyMuPDF only
  TypstReconstruct → Full document rebuild from transaction data
```

## Document Parsers (Multi-Backend)

Each parser auto-falls back to the offline parser if its API key is missing or the call fails:

- **Mindee Financial Document API (default):** Cloud-based ML parser with per-field bounding boxes. Requires `MINDEE_API_KEY`. Best balance of accuracy, ease of setup, and cost.
- **Google Cloud Document AI (v1beta3):** Highest accuracy on trained layouts. Supports custom-trained processor versions with in-app admin (train, deploy, undeploy). Requires GCP credentials.
- **LlamaParse:** LLM-based document parser via LlamaCloud. Requires `LLAMAPARSE_API_KEY`.
- **PyMuPDF Built-in:** Local text extraction using pymupdf's native layout parser. No external dependencies. Always available.
- **Local OCR (`ocrs` + `rten`):** Pure Rust OCR for scanned documents. Requires `--features ocr` at compile time.
- **Offline Parser (`offline_parser.rs`):** Deterministic heuristic parser using regex patterns and structural analysis. The universal fallback for all cloud parsers.

## AI and Machine Learning Integration

- **Google Gemini / Vertex AI:** Multi-purpose AI engine used for:
  - Smart Balance Engine — generates minimal cascading adjustment plans to resolve math errors.
  - Completeness Validation — checks for missed transaction rows.
  - Vision Validation — compares rendered PDF pages for visual fidelity.
  - Auth modes: API Key (simple) or Vertex AI (enterprise, SA/ADC).
  - Fallback: graceful skip with score=0.7 (pipeline continues without AI).
- **Applitools Eyes (via Node.js bridge):** Visual AI testing layer. Compares original vs. edited rendered pages using Applitools' proprietary visual AI. Requires `APPLITOOLS_API_KEY` + Node.js. Additive layer — gracefully skips if unavailable.
- **pdfRest API:** Adobe-tier cloud PDF rendering for high-fidelity visual verification. Requires `PDFREST_API_KEY`. Falls back to local Pdfium rendering.

## Verification Pipeline (Multi-Layer)

The verification system uses additive layers — each layer runs independently:

1. **SSIM (Structural Similarity Index):** Compares rendered page images pixel-by-pixel.
2. **Tile-Max Diff:** Divides pages into tiles and finds the maximum local difference (catches localized drift that whole-page averages hide).
3. **Perceptual Hash (`image_hasher`):** Hash-based similarity check for structural integrity.
4. **pdfRest Cloud Rendering (optional):** Adobe-tier rendering for comparison against local renders.
5. **Applitools Eyes (optional):** AI visual comparison via Node.js bridge.
6. **Gemini Vision AI (optional):** AI-based visual fidelity analysis.

Configurable thresholds: `visual_diff_threshold` (default 0.02) and `max_visual_attempts` (default 5).

## Geometry Extraction (Hybrid)

- **Bank Templates (YAML):** Per-bank column layouts for deterministic text extraction. Ships with templates for AU and US banks.
- **PyMuPDF Heuristic:** Statistical column boundary detection from embedded text runs.
- **Hybrid Merger:** Merges results from multiple geometry providers with deterministic tiebreak rules.

## Concurrency and Asynchronous Runtime

- **`tokio` (1.x):** The industry-standard async runtime for Rust. Handles all network requests, file I/O, job dispatching, and timeout management.
- **MPSC Channels:** Multi-producer, single-consumer channels bridge the synchronous, immediate-mode GUI (`egui`) with the asynchronous background tasks (`tokio`).
- **API Semaphore:** Limits concurrent cloud API calls (default 3) to prevent rate limiting.
- **Cancellation Registry:** Per-job cancellation tokens for responsive UI cancellation.

## Python Interoperability (FFI)

- **`pyo3` (0.24):** The Rust/Python bridge. Embeds a Python interpreter directly inside the Rust process, allowing Rust to execute Python scripts with near-zero overhead. Work is constrained to a single dedicated actor thread to avoid Python GIL deadlocks. Panics inside the actor are caught and surfaced as structured errors.

## Node.js Interoperability

- **Child Process:** The Applitools bridge (`src/ai/applitools_bridge.js`) is invoked as a `node` child process from Rust. Communication is via stdout JSON lines (`APPLITOOLS_RESULT:{...}`). Failure detection is graceful — if Node.js or the bridge script is unavailable, the verification pipeline continues with local-only metrics.

## Observability and Logging

- **`tracing` / `tracing-subscriber`:** Structured, event-driven logging framework for Rust with daily file rotation.
- **`opentelemetry` (0.21):** OpenTelemetry SDK for distributed tracing, allowing the application to export metrics and traces to an OTLP-compatible endpoint for deep debugging.
- **Boot-Time API Summary:** `ApiAvailability::log_summary()` logs the availability status of all backends on startup for instant diagnostics.

## Serialization and State Management

- **`serde` (1.0) / `serde_json`:** Used ubiquitously for serializing and deserializing API requests, JSON outputs, and internal message passing.
- **`confy` (0.6):** Configuration management for persisting `AppSettings` (dark mode, advanced mode, backend preferences, visual thresholds) to the user's local application data directory.
- **`dotenvy`:** Loads `.env` file on startup. Hot-reloadable via `Job::ReloadConfig`.

## Security and Fault Tolerance

- **`chacha20poly1305`:** Strong encryption of the local Document AI cache and other sensitive artifacts at rest.
- **Enterprise Fault Tolerance:** Exponential backoffs, automatic retry middleware via `reqwest-retry`, strict cryptographic software root-of-trust (via SHA-256 and `.pipeline_key`).
- **Zero-Trust API Detection:** Every API key is validated at boot time. Missing keys auto-exclude their backend from the fallback chain and disable the corresponding UI toggle with an explanatory message.

## Font Engineering

- **`ttf-parser`:** Fast, zero-allocation TrueType/OpenType font parsing for font analysis and metric extraction.
- **`subsetter`:** Font subsetting for Typst reconstruction — embeds only the glyphs needed by each page.
- **Font Replication:** Deep font analysis extracts embedded font programs and re-embeds them with matched weight/width metrics for cross-statement font transfer.
