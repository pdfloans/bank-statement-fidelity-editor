# Bank Statement Fidelity Editor v0.4.0

A high-fidelity PDF text editor focused on bank statements: replace text and numbers in-place with the original kerning, font, size, color, and position preserved, then keep all transactions and running balances mathematically consistent.

## What it does

- **Targeted edits.** Click any text on a rendered page to select the exact bounding box; type a new value; the engine performs a redaction-based replacement that re-uses the original glyph metrics.
- **Multi-stage workflow.** Parse → Edit → Balance Preview → Confirm & Render → Visual Validate → Final Math Check, with autosaved drafts so you can pause and resume mid-edit. See [Workflow](#workflow) below.
- **Batch Processing Dashboard.** Drag and drop a folder of PDFs to queue up asynchronous extraction or smart auto-balancing across dozens of statements at once.
- **Progressive Disclosure.** Complex verification settings and forensic modes are tucked behind an "Advanced Mode" toggle, keeping the default UI clean and focused.
- **Smart Balance Engine.** Runs Document AI over the full PDF, detects mathematical imbalance, and asks Gemini for the minimum cascading adjustment plan. Only the *last* running balance is auto-corrected by default; everything else stays untouched.
- **Hybrid extraction.** Document AI for semantics, plus geometry providers (per-bank YAML templates, PyMuPDF heuristics, optional Tesseract). Sources are merged with deterministic tiebreak rules.
- **Verification.** Renders original vs. edited at 300 DPI, computes per-pixel delta + perceptual hash; falls back from pdfRest (Adobe-tier) to local pdfium-render automatically if no key is configured.
- **Audit log + change history.** Every edit lands in an append-only log file with a snapshot PDF, plus an in-memory undo/redo stack and an autosaved `audit/history.json` so you can resume after a crash. The final step of the pipeline automatically merges the generated Audit JSON Report directly as a new page onto the final output PDF.
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
| `DUAL_CORE_PASSPHRASE` | Yes (≥16 chars) | Software root-of-trust passphrase. Alternatively, create a `.pipeline_key` file. |
| `PYMUPDF_PRO_KEY` | Recommended | PyMuPDF Pro license key for the Python actor. |
| `GEMINI_API_KEY` | For Smart Balance | Google Gemini API key. |
| `GEMINI_AUTH_MODE` | Optional | `ApiKey` (default) or `vertex` (Google Cloud Service Account/ADC). |
| `DOCUMENT_AI_API_KEY` | For Document AI (Beta, preferred) | API key for Document AI v1beta3. Tried first. |
| `GOOGLE_APPLICATION_CREDENTIALS` | For Document AI (legacy fallback) | Path to service-account JSON. Used if API-key auth fails. |
| `DOCUMENT_AI_PROJECT_ID` | For Document AI | GCP project. |
| `DOCUMENT_AI_LOCATION` | For Document AI | e.g. `us`. |
| `DOCUMENT_AI_PROCESSOR_ID` | For Document AI | Processor ID. |
| `DOCUMENT_AI_GCS_URI` | For Batch AI Outputs | e.g. `gs://my-bucket/outputs/`. |
| `PDFREST_API_KEY` | Optional | Enables Adobe-tier visual verification and AI orchestration. |
| `LIPI_API_KEY` | Optional | Used for legacy Lipi processing. |
| `WEBHOOK_URL` | Optional | URL to ping on job completion. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Optional | OTLP gRPC endpoint. |
| `OTEL_SERVICE_NAME` | Optional | Defaults to `dual-core-pdf-pipeline`. |
| `RUST_LOG` | Optional | e.g. `info`, `debug`. |
| `LOG_DIR` | Optional | Defaults to `./logs`. |

Run `dual-core-pdf-pipeline doctor` to print a one-shot health check (env vars set, directories writable, runtime worker reachable).

## CLI

```text
dual-core-pdf-pipeline gui                                 # launch the GUI
dual-core-pdf-pipeline serve                               # run headless server (binds 0.0.0.0:$PORT)
dual-core-pdf-pipeline doctor                              # config health check
dual-core-pdf-pipeline verify-api-keys                     # verify all API keys
dual-core-pdf-pipeline text -i in.pdf -o out.pdf \
    --old "100.00" --new "150.00" --page 0 --bbox 50,40,90,52
dual-core-pdf-pipeline balance -i in.pdf -o out.pdf [--auto-approve]
dual-core-pdf-pipeline auto-balance -i in.pdf -o out.pdf   # Smart balance with auto-approve
dual-core-pdf-pipeline extract -i in.pdf -o data.json
dual-core-pdf-pipeline verify --original a.pdf --edited b.pdf --output-dir audit/verify [--use-pdfrest]
dual-core-pdf-pipeline render -i in.pdf -o pages -p 0 --dpi 300
dual-core-pdf-pipeline font-complete -i in.pdf --font Helvetica
dual-core-pdf-pipeline analyze-fonts -i in.pdf
dual-core-pdf-pipeline ai-fix-visual -i in.pdf -p 0
dual-core-pdf-pipeline docai-train                         # Train a new Document AI processor version
dual-core-pdf-pipeline fontcache-init                      # Bootstrap font cache
dual-core-pdf-pipeline transfer-transactions --source-pdf a.pdf --target-pdf b.pdf -o out.pdf
dual-core-pdf-pipeline adjust-dates -i in.pdf -o out.pdf --mode shift-forward-1-month
dual-core-pdf-pipeline run-transfer-tests --statements a.pdf,b.pdf
dual-core-pdf-pipeline export-history --from-log audit/2026.log -o history.json
```

## GUI shortcuts

- `Ctrl+O` open • `Ctrl+Z/Y` undo/redo • `Ctrl+S` export history
- `+` / `-` zoom • `0` reset zoom • `←/→` page nav
- Middle-drag (or Shift+drag) to pan; Ctrl+wheel to zoom

## Workflow

The application supports two primary flows: **Single Statement** and **Batch Processing**.

### Single Statement Flow
The right-hand "Workflow" panel walks the user through six stages. Each
stage is gated by an explicit button click — the app never silently
moves to the next step.

1. **① Parse + AI validate.** Document AI extracts every transaction; Gemini
   double-checks for missed rows. Result: a `ParseValidation` with a
   completeness score (0..1) and a list of any rows the deterministic
   geometry extractor saw but Document AI missed.
2. **Edit.** The inline edit table (powered by `egui_extras::TableBuilder`)
   shows every parsed row with editable Date / Description / Debit /
   Credit / Balance columns. Numeric fields turn red when the typed text
   isn't parseable. Click "↶" on any row to revert every queued edit on
   that row at once.
3. **② Balance Out Preview.** Recomputes every running balance with the
   user's edits applied and shows the per-row diff plus the final
   imbalance. Translucent yellow boxes appear on the canvas over each
   `will_change` cell — hover for a `<old> → <new>` tooltip.
4. **③ Confirm and Render.** Applies edits to the PDF using the binary-level
   redact-and-overlay path. Drops any "redundant" edits whose typed value
   already matches what the cascade would produce.
5. **Visual validate.** Renders the edited PDF, compares to the original
   page-by-page (only the changed pages — full-document re-renders are
   avoided), retries up to 5 attempts with growing tolerance.
6. **Final math check.** Re-parses the rendered PDF through Document AI
   and verifies all running balances are still consistent.

### Batch Processing Flow
The **Batch Processing** tab allows for bulk operations across multiple PDFs:
1. **Load Folder:** Drag and drop a folder containing multiple bank statements.
2. **Bulk Extraction:** Click "Extract All to JSON" to concurrently extract transactional data from all files.
3. **Bulk Auto-Balance:** Click "Auto-Balance All" to invoke the Smart Balance Engine on all files. This will automatically approve the AI's proposed cascade and output `_balanced.pdf` files next to the originals.

### Drafts

The whole session (parse, queued edits, stage) autosaves to
`audit/workflow.json` every 1.5s as you edit. **File → Resume workflow
draft** restores it; **File → Discard workflow draft** clears it. The
draft is hashed against the source PDF so the GUI can warn you if you
re-open the draft against a modified file. On a successful workflow
completion the draft is automatically removed.

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

### Remote Engine Mode
The GUI can be configured to offload processing to a remote engine via the `ConnectionMode`. When toggled in Advanced Mode, the GUI acts as a thin client (🟢 Local vs 🔵 Remote status indicator), dispatching `Runtime` jobs to a hosted version of the backend (e.g. on Railway) over HTTP.

## Forensics & watermarking caveats

This tool edits text perfectly but cannot achieve commercial-tool forensic identity. Public watermarking limits apply. See [the original disclaimer](#what-it-does-not-do).
