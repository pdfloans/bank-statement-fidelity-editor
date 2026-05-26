# Bank Statement Fidelity Editor v0.4.0

The ONLY job of this tool is to modify text and numbers in bank statement PDFs with **100% visual fidelity** (exact kerning, font, size, color, and position). It also features a Smart Balance Engine to automatically reconcile calculations across the entire document.

## Features (v0.4.0)

| Feature | Description |
| :--- | :--- |
| **PdfEngine Selector** | Dual PDF backend: `mupdf` (primary) and PyMuPDF Pro (fallback). Automatically routes around unsupported PDF features. |
| **Hybrid Extraction** | Document AI for semantic parsing merged with local Geometry Extractors (Tesseract, PyMuPDF Heuristics, Bank Templates). |
| **Smart Balance Engine** | Identifies imbalance across multiple pages and uses Google Gemini to propose minimal cascading adjustments. |
| **100% Visual Fidelity** | Targets specific bounding boxes, extracting embedded fonts and generating pixel-perfect text replacements via PyMuPDF Pro. |
| **Verification Math** | 300 DPI visual diffing and strict mathematical verification against the original statement's balance. Optional Adobe pdfRest rendering. |
| **Audit Log** | Append-only compliance log, capable of reconstructing history and producing a JSON export of all manual and AI-assisted changes. |
| **Telemetry** | Structured logging using `tracing` macros and optional OTLP trace export. |

## System Dependencies

This pipeline requires specific system-level dependencies for rendering and geometry extraction. 

| OS | Requirements |
| :--- | :--- |
| **Windows** | Visual Studio 2019 Build Tools (v142) for `mupdf`. Tesseract and Leptonica binaries installed and in PATH. Python 3.10+ |
| **macOS** | `brew install mupdf tesseract leptonica`. Python 3.10+ |
| **Linux (Ubuntu)**| `apt-get install libmupdf-dev tesseract-ocr libleptonica-dev`. Python 3.10+ |

**Python Dependencies:**
```bash
pip install pymupdf pymupdfpro
```

## Configuration

The application is configured exclusively via environment variables (or a `.env` file).

| Variable | Description |
| :--- | :--- |
| `DUAL_CORE_PASSPHRASE` | **Required.** The strong passphrase required to unlock the pipeline. |
| `PYMUPDF_PRO_KEY` | PyMuPDF Pro license key. |
| `GEMINI_API_KEY` | **Required.** API Key for Google Gemini (Smart Balancing). |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to your Google Cloud Service Account JSON key (for Document AI). |
| `DOCUMENT_AI_PROJECT_ID` | GCP Project ID. |
| `DOCUMENT_AI_LOCATION` | GCP Location (e.g., `us`). |
| `DOCUMENT_AI_PROCESSOR_ID` | The ID of the deployed Document AI parser. |
| `PDFREST_API_KEY` | Optional. Key for Adobe pdfRest to enable highest-tier visual verification. |
| `OTEL_EXPORTER_OTLP_ENDPOINT`| Optional. The gRPC endpoint for OTLP traces. |
| `OTEL_SERVICE_NAME` | Optional. Defaults to `dual-core-pdf-pipeline`. |
| `RUST_LOG` | Optional. Logging level (e.g., `info`, `debug`). |

## Architecture

```text
app/          (CLI, GUI, Audit Log, Telemetry, Runtime Coordinator)
engine/       (Smart Balance Math, Transaction Model, Verification, History)
pdf/          (Dual Engine Abstraction: MuPDF / PyMuPDF)
extractors/   (Hybrid Geometry: Tesseract, PyMuPDF Heuristics, Bank Templates)
ai/           (Document AI, Gemini, pdfRest, PyO3 Bridge)
security/     (Software Root of Trust)
```

## Forensics & Watermarking Ceiling

**Disclaimer:** This tool edits text perfectly, but it *cannot* achieve Adobe forensic identity or forge commercial MuPDF signatures. Commercial systems may still detect that a file has been re-saved by an open-source library, and public watermarking limits apply.
