# Bank Statement Fidelity Editor v0.4.0

A high-fidelity PDF text editor focused on bank statements: replace text and numbers in-place with the original kerning, font, size, color, and position preserved, then keep all transactions and running balances mathematically consistent.

## What it does

- **Targeted edits.** Click any text on a rendered page to select the exact bounding box; type a new value; the engine performs a redaction-based replacement that re-uses the original glyph metrics.
- **Smart Balance Engine.** Runs Document AI over the full PDF, detects mathematical imbalance, and asks Gemini for the minimum cascading adjustment plan. Only the *last* running balance is auto-corrected by default; everything else stays untouched.
- **Hybrid extraction.** Document AI for semantics, plus geometry providers (per-bank YAML templates, PyMuPDF heuristics, optional Tesseract). Sources are merged with deterministic tiebreak rules.
- **Verification.** Renders original vs. edited at 300 DPI, computes per-pixel delta + perceptual hash; falls back from pdfRest (Adobe-tier) to local pdfium-render automatically if no key is configured.
- **Audit log + change history.** Every edit lands in an append-only log file with a snapshot PDF, plus an in-memory undo/redo stack and an autosaved `audit/history.json` so you can resume after a crash.
- **CLI + GUI parity.** Both interfaces drive the same `Runtime` job loop, so anything you can do in the GUI you can script.

## What it does not do

- It cannot forge Adobe signatures, mimic a commercial MuPDF watermark, or defeat sophisticated forensic detection. Re-saved PDFs may still be flagged as "modified" by tools that read library fingerprints.
- The Smart Balance Engine and Document AI extraction require **GEMINI_API_KEY** + a configured **Document AI** processor. Without those, only manual edit / verify / render work.
- pdfRest is optional. Without `PDFREST_API_KEY` the verifier falls back to local rendering with a warning.

## System dependencies

| OS | Required |
|---|---|
| **Windows** | Visual Studio 2019 Build Tools (v142). Python 3.10+. (Tesseract is optional.) |
| **macOS** | `brew install mupdf tesseract leptonica`. Python 3.10+. |
| **Linux (Ubuntu)** | `apt-get install libmupdf-dev tesseract-ocr libleptonica-dev`. Python 3.10+. |

Python packages: `pip install pymupdf pymupdfpro fonttools pillow`.

## Build

```text
cargo build --release
```

The release binary is `target/release/dual-core-pdf-pipeline`.

## Configuration

All configuration is via environment variables (or a `.env` file).

| Variable | Required? | Description |
|---|---|---|
| `DUAL_CORE_PASSPHRASE` | Yes (≥16 chars) | Software root-of-trust passphrase. |
| `PYMUPDF_PRO_KEY` | Recommended | PyMuPDF Pro license key for the Python actor. |
| `GEMINI_API_KEY` | For Smart Balance | Google Gemini API key. |
| `DOCUMENT_AI_API_KEY` | For Document AI (Beta, preferred) | API key for Document AI v1beta3. Tried first. |
| `GOOGLE_APPLICATION_CREDENTIALS` | For Document AI (legacy fallback) | Path to service-account JSON. Used if API-key auth fails. |
| `DOCUMENT_AI_PROJECT_ID` | For Document AI | GCP project. |
| `DOCUMENT_AI_LOCATION` | For Document AI | e.g. `us`. |
| `DOCUMENT_AI_PROCESSOR_ID` | For Document AI | Processor ID. |
| `PDFREST_API_KEY` | Optional | Enables Adobe-tier visual verification. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Optional | OTLP gRPC endpoint. |
| `OTEL_SERVICE_NAME` | Optional | Defaults to `dual-core-pdf-pipeline`. |
| `RUST_LOG` | Optional | e.g. `info`, `debug`. |
| `LOG_DIR` | Optional | Defaults to `./logs`. |

Run `dual-core-pdf-pipeline doctor` to print a one-shot health check (env vars set, directories writable, runtime worker reachable).

## CLI

```text
dual-core-pdf-pipeline gui                                 # launch the GUI
dual-core-pdf-pipeline doctor                              # config health check
dual-core-pdf-pipeline text -i in.pdf -o out.pdf \
    --old "100.00" --new "150.00" --page 0 --bbox 50,40,90,52
dual-core-pdf-pipeline balance -i in.pdf -o out.pdf [--auto-approve]
dual-core-pdf-pipeline extract -i in.pdf -o transactions.json
dual-core-pdf-pipeline verify --original a.pdf --edited b.pdf -o audit/verify
dual-core-pdf-pipeline render -i in.pdf -o pages -p 0 --dpi 300
dual-core-pdf-pipeline font-complete -i in.pdf --font Helvetica
dual-core-pdf-pipeline export-history --from-log audit/2026.log -o history.json
```

## GUI shortcuts

- `Ctrl+O` open • `Ctrl+Z/Y` undo/redo • `Ctrl+S` export history
- `+` / `-` zoom • `0` reset zoom • `←/→` page nav
- Middle-drag (or Shift+drag) to pan; Ctrl+wheel to zoom

## Architecture

```text
app/          CLI, GUI, runtime, audit log, telemetry, config
engine/       Balance math, transaction model, verification, history, layout, text editor
pdf/          Engine trait + selector (mupdf/pdfium primary, PyMuPDF fallback)
extractors/   Geometry providers (templates, PyMuPDF heuristic, Tesseract) + hybrid merger
ai/           Document AI, Gemini, pdfRest clients + PyO3 bridge
security/     Software root-of-trust
```

All long-running work goes through the `Runtime` job loop. The GUI never blocks. Python work is funnelled into a single dedicated actor thread to avoid PyO3 cross-thread issues. Panics inside the actor are caught and surfaced as structured errors instead of crashing the process.

## Forensics & watermarking caveats

This tool edits text perfectly but cannot achieve commercial-tool forensic identity. Public watermarking limits apply. See [the original disclaimer](#what-it-does-not-do).
