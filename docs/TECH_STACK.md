# Technology Stack Breakdown

This document provides a detailed overview of the specific technologies, frameworks, and libraries used to build the Bank Statement Fidelity Editor (`dual-core-pdf-pipeline`).

## Core Languages

- **Rust (Edition 2021):** The primary language for the application logic, GUI, and high-performance asynchronous orchestration. Rust guarantees memory safety, fast execution, and strict concurrency control.
- **Python (3.10+):** Used selectively for deep integrations with proprietary or C-based PDF rendering and manipulation libraries (like PyMuPDF Pro) that don't have native Rust equivalents.

## GUI Framework (Native Desktop)

- **`egui` (0.28):** A highly performant, immediate-mode GUI framework built purely in Rust. Chosen for its simplicity, speed, and seamless integration with Rust data structures, making it perfect for rapid data-driven UI development.
- **`eframe` (0.28):** The official framework wrapper around `egui` to run it as a native desktop application (handling the OS windowing, WebGL/WGPU rendering contexts).

## PDF Processing Engines

The application employs a "hybrid" PDF processing strategy, leaning on different libraries depending on the exact requirements of the task.

- **`lopdf` (0.34):** A pure-Rust library used for low-level PDF dictionary manipulation. It is primarily used for the `Split & Merge` engine (Subsystem A), as it can extract pages and merge them back together without altering visual fidelity or dropping fonts.
- **`pdfium-render` (0.8):** Rust bindings to Google's open-source `pdfium` C++ library (the same engine used in Google Chrome). Used for rendering PDFs to bitmaps for visual verification and click-targeting on the canvas.
- **`PyMuPDF` / `pymupdfpro` (via `pyo3`):** Called from Rust via Python bindings. This is the only engine capable of high-fidelity, per-segment redaction and text insertion while accurately reusing the exact embedded font dictionaries and glyph metrics (Subsystem B).

## AI and Machine Learning Integration

- **Google Cloud Document AI (v1beta3):** The primary engine for semantic extraction. It is responsible for parsing raw PDF layouts into structured transactions (dates, descriptions, debits, credits, running balances).
- **Google Gemini / Vertex AI:** Used as the "Smart Balance Engine". Gemini is fed the deterministic imbalances detected in the statement and tasked with generating minimal cascading adjustment plans to resolve math errors.
- **pdfRest API:** Integrated as an advanced AI client and orchestration layer for Adobe-tier visual verification and document analysis.
- **Tesseract OCR (Optional):** Used locally as a fallback geometry provider for text bounding boxes if native PDF text extraction fails.

## Concurrency and Asynchronous Runtime

- **`tokio` (1.x):** The industry-standard async runtime for Rust. It handles all network requests (Document AI, Gemini), file I/O, and job dispatching. 
- **MPSC Channels:** The application uses multi-producer, single-consumer channels to bridge the synchronous, immediate-mode GUI (`egui`) with the asynchronous background tasks (`tokio`).

## Python Interoperability (FFI)

- **`pyo3` (0.22):** The Rust/Python bridge. It embeds a Python interpreter directly inside the Rust process, allowing Rust to execute Python scripts (like the PyMuPDF modification scripts) with near-zero overhead. Work is constrained to a single dedicated actor thread to avoid Python Global Interpreter Lock (GIL) deadlocks.

## Observability and Logging

- **`tracing` / `tracing-subscriber`:** Structured, event-driven logging framework for Rust.
- **`opentelemetry` (0.21):** OpenTelemetry SDK for distributed tracing, allowing the application to export metrics and traces to an OTLP-compatible endpoint for deep debugging.

## Serialization and State Management

- **`serde` (1.0) / `serde_json`:** Used ubiquitously for serializing and deserializing API requests, JSON outputs, and internal message passing.
- **`confy` (0.6):** A configuration management library used to persist the `AppSettings` (like dark mode, advanced mode, etc.) to the user's local application data directory.

## Security and Fault Tolerance

- **`chacha20poly1305`:** Used for strong encryption of the local Document AI cache and other sensitive artifacts at rest.
- **Enterprise Fault Tolerance:** Handled inside the `tokio` asynchronous runtime with exponential backoffs, automatic retry middlewares via `reqwest-retry`, and strict cryptographic software root-of-trust (via SHA-256 and `.pipeline_key`).
