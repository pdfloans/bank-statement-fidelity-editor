# Bank Statement Fidelity Editor v1.0.0

A high-fidelity PDF text editor focused on bank statements: replace text and numbers in-place with the original kerning, font, size, color, and position preserved, then keep all transactions and running balances mathematically consistent.

## What it does

- **Targeted edits.** Click any text on a rendered page to select the exact bounding box; type a new value; the engine performs a redaction-based replacement that re-uses the original glyph metrics.
- **Multi-stage workflow.** Parse → Edit → Balance Preview → Confirm & Render → Visual Validate → Final Math Check, with autosaved drafts so you can pause and resume mid-edit. See [Workflow](#workflow) below.
- **Multi-backend pipeline with auto-fallback.** Every stage has a configurable primary backend and automatic fallback. If an API key is missing or a cloud service fails, the pipeline gracefully degrades to the next-best offline option. See [Backend Preferences](#backend-preferences).
- **Boot-time API detection.** On startup the app probes every configured API key and displays availability status (✅ / ⛔) in the Backend Preferences panel. Unavailable backends are labelled with an explanation and signup URL.
- **Batch Processing Dashboard.** Drag and drop a folder of PDFs to queue up asynchronous extraction or smart auto-balancing across dozens of statements at once.
- **Progressive Disclosure.** Complex verification settings and forensic modes are tucked behind an "Advanced Mode" toggle, keeping the default UI clean and focused.
- **Smart Balance Engine.** Parses the full PDF, detects mathematical imbalance, and asks Gemini for the minimum cascading adjustment plan. Only the *last* running balance is auto-corrected by default; everything else stays untouched. Falls back to local deterministic balance analysis when AI is unavailable.
- **Hybrid extraction.** Multiple parsers (Mindee, LlamaParse, Document AI, PyMuPDF, Local OCR) plus geometry providers (per-bank YAML templates, PyMuPDF heuristics). Sources are merged with deterministic tiebreak rules.
- **Multi-layer verification.** Renders original vs. edited at 300 DPI, computes per-pixel delta + perceptual hash + SSIM; optionally adds pdfRest cloud rendering and Applitools Eyes visual AI. Configurable thresholds and retry counts via sliders.
- **Audit log + change history.** Every edit lands in an append-only log file with a snapshot PDF, plus an in-memory undo/redo stack and an autosaved `audit/history.json` so you can resume after a crash. The final step automatically merges the Audit JSON Report as a new page onto the final output PDF.
- **CLI + GUI parity.** Both interfaces drive the same `Runtime` job loop, so anything you can do in the GUI you can script.

## What it does not do

- It cannot forge Adobe signatures, mimic a commercial MuPDF watermark, or defeat sophisticated forensic detection. Re-saved PDFs may still be flagged as "modified" by tools that read library fingerprints.
- Without any API keys at all, only manual edit / verify / render with local offline parsing works. The more keys you configure, the more pipeline stages light up.
- pdfRest, Applitools, and AI vision are additive verification layers — they enhance fidelity checking but are not required.

## System dependencies

| OS | Required |
|---|---|
| **Windows** | Visual Studio 2019 Build Tools (v142). Python 3.10+. Node.js 18+ (for Applitools). |
| **macOS** | `brew install mupdf tesseract leptonica`. Python 3.10+. Node.js 18+. |
| **Linux (Ubuntu)** | `apt-get install libmupdf-dev tesseract-ocr libleptonica-dev`. Python 3.10+. Node.js 18+. |

Python packages: `pip install pymupdf pymupdfpro fonttools pillow`.  
Node packages (optional): `npm install @applitools/eyes-images` (for visual AI verification).

## Build

```text
cargo build --release
```

The release binary is `target/release/dual-core-pdf-pipeline`.

## Configuration

All configuration is via environment variables (or a `.env` file). Copy `.env.example` to `.env` to get started.

### Required Keys

| Variable | Description |
|---|---|
| `DUAL_CORE_PASSPHRASE` | Software root-of-trust passphrase (≥16 chars). Alternatively, create a `.pipeline_key` file. |

### AI & Parsing Keys

| Variable | Used By | Fallback If Missing |
|---|---|---|
| `GEMINI_API_KEY` | Smart Balance, AI Completeness, Vision Validation | → Manual-only mode (local balance engine) |
| `GEMINI_AUTH_MODE` | Auth method: `api_key` (default) or `vertex` (enterprise SA/ADC) | Defaults to `api_key` |
| `MINDEE_API_KEY` | Mindee Financial Document API | → offline parser (PyMuPDF built-in) |
| `LLAMAPARSE_API_KEY` | **Default parser** — LlamaParse LLM-based parser | → offline parser |
| `DOCUMENT_AI_PROJECT_ID` | Google Document AI parser | → offline parser |
| `DOCUMENT_AI_LOCATION` | e.g. `us` | |
| `DOCUMENT_AI_PROCESSOR_ID` | Processor ID | |
| `DOCUMENT_AI_API_KEY` | Document AI v1beta3 API key (preferred auth) | → SA/ADC auth |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to service-account JSON (legacy fallback) | → ADC auto-detection |

### PDF Engine & Verification Keys

| Variable | Used By | Fallback If Missing |
|---|---|---|
| `PYMUPDF_PRO_KEY` | PyMuPDF Pro (enhanced font handling) | → PyMuPDF free tier |
| `PDFREST_API_KEY` | Adobe-tier cloud rendering for verification | → local Pdfium |
| `APPLITOOLS_API_KEY` | Applitools Eyes visual AI testing | → graceful skip (SSIM-only) |

### Optional / Telemetry

| Variable | Description |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP gRPC endpoint (default: `http://localhost:4317`) |
| `OTEL_SERVICE_NAME` | Defaults to `dual-core-pdf-pipeline` |
| `RUST_LOG` | e.g. `info`, `debug` |
| `LOG_DIR` | Defaults to `./logs` |

Run `dual-core-pdf-pipeline doctor` to print a one-shot health check (env vars set, directories writable, runtime worker reachable).

## Backend Preferences

The **Backend Preferences** panel (Settings → Backend Preferences) lets you choose which backend to use for each pipeline stage. Options that require a missing API key are marked with ⛔ and show a hover tooltip explaining what's needed.

### PDF Engine
| Mode | Description | Requires |
|---|---|---|
| **Auto** (default) | PyMuPDF first, falls back to Pdfium | Python + pymupdf |
| Dual Concurrent | Both engines in parallel | Python + pymupdf |
| Force Native (Pdfium) | Pdfium only | Always available |
| Force PyMuPDF | PyMuPDF only | Python + pymupdf |
| Reconstruct (Typst) | Rebuild from scratch via Typst | Always available |

### AI Provider
| Mode | Description | Requires |
|---|---|---|
| **Manual Only** (default) | No AI calls | Nothing |
| Gemini (API Key) | AI Studio key | `GEMINI_API_KEY` |
| Gemini (Vertex AI) | Enterprise SA/ADC | Service account + ADC |
| Groq (Llama 3) | Fast math reasoning | `GROQ_API_KEY` |
| OpenRouter (DeepSeek) | Double-check reasoning | `OPENROUTER_API_KEY` |

### Document Parser
| Mode | Description | Requires | Fallback |
|---|---|---|---|
| Mindee | Cloud ML parsing | `MINDEE_API_KEY` | → offline parser |
| **LlamaParse** (default) | LLM-based parsing | `LLAMAPARSE_API_KEY` | → offline parser |
| PyMuPDF Built-in ✅ | Local text extraction | Always available | — |
| Local OCR ✅ | Pure Rust OCR | `--features ocr` | — |
| Document AI | Google ML parsing | GCP credentials | → offline parser |

### Verification Renderer
| Mode | Description | Requires | Fallback |
|---|---|---|---|
| **Local Pdfium** ✅ (default) | Local rendering | Always available | — |
| pdfRest (Cloud) | Adobe-tier rendering | `PDFREST_API_KEY` | → Local Pdfium |
| Applitools Eyes (Additive) | Visual AI testing | `APPLITOOLS_API_KEY` | → Skips if missing |

### Visual Validation Thresholds
| Setting | Default | Range | Description |
|---|---|---|---|
| Visual Diff Threshold | 0.02 | 0.005–0.10 | Tile-max diff ceiling. Lower = stricter. |
| Max Visual Retries | 5 | 1–10 | Retry attempts with progressive mask widening. |

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

### Stress Testing
cargo test --test au_transfer_stress -- --ignored --nocapture
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

1. **① Parse + AI validate.** The selected document parser extracts every transaction; if an AI provider is configured, Gemini double-checks for missed rows. If the cloud parser fails, the pipeline auto-falls back to the offline parser. Result: a `ParseValidation` with a completeness score (0..1) and a list of any rows the deterministic geometry extractor saw but the parser missed.
2. **Edit.** The inline edit table (powered by `egui_extras::TableBuilder`) shows every parsed row with editable Date / Description / Debit / Credit / Balance columns. Numeric fields turn red when the typed text isn't parseable. Click "↶" on any row to revert every queued edit on that row at once.
3. **② Balance Out Preview.** Recomputes every running balance with the user's edits applied and shows the per-row diff plus the final imbalance. Translucent yellow boxes appear on the canvas over each `will_change` cell — hover for a `<old> → <new>` tooltip.
4. **③ Confirm and Render.** Applies edits to the PDF using the selected engine mode. If the primary engine fails, the pipeline falls back through Pdfium → Typst Reconstruct as ultimate fail-safe. Drops any "redundant" edits whose typed value already matches what the cascade would produce.
5. **Visual validate.** Renders the edited PDF, compares to the original page-by-page (only changed pages), retries up to `max_visual_attempts` with growing tolerance. If pdfRest is configured, adds a cloud rendering layer. If Applitools is configured, adds AI visual comparison. Both gracefully degrade if unavailable.
6. **Final math check.** Re-parses the rendered PDF and verifies all running balances are still consistent.

### Batch Processing Flow
The **Batch Processing** tab allows for bulk operations across multiple PDFs:
1. **Load Folder:** Drag and drop a folder containing multiple bank statements.
2. **Bulk Extraction:** Click "Extract All to JSON" to concurrently extract transactional data from all files.
3. **Bulk Auto-Balance:** Click "Auto-Balance All" to invoke the Smart Balance Engine on all files.

### Drafts

The whole session (parse, queued edits, stage) autosaves to
`audit/workflow.json` every 1.5s as you edit. **File → Resume workflow
draft** restores it; **File → Discard workflow draft** clears it. The
draft is hashed against the source PDF so the GUI can warn you if you
re-open the draft against a modified file. On a successful workflow
completion the draft is automatically removed.

## Architecture

```text
app/          CLI, GUI, runtime, audit log, telemetry, config, API availability detection
engine/       Balance math, transaction model, verification (multi-layer), history, layout,
              typst reconstruction, font analysis/replication/shaping, offline parser
pdf/          Engine trait + selector (PyMuPDF primary, Pdfium fallback, OxidizePdf)
extractors/   Geometry providers (per-bank templates, PyMuPDF heuristic) + hybrid merger
ai/           Document AI, Gemini, Mindee, LlamaParse, pdfRest, Applitools bridge,
              PyO3 Python bridge
security/     Software root-of-trust, ChaCha20-Poly1305 encryption
```

All long-running work goes through the `Runtime` job loop. The GUI never blocks. Python work is funnelled into a single dedicated actor thread to avoid PyO3 cross-thread issues. Panics inside the actor are caught and surfaced as structured errors instead of crashing the process.

### Fallback Hierarchy

Every pipeline stage is designed with explicit fallback chains:

```
PDF Engine:    PyMuPDF → Pdfium → Typst Reconstruct (ultimate)
Parsers:       Mindee/LlamaParse/DocAI → offline_parser (PyMuPDF built-in)
AI Balance:    Gemini → local balance::process_and_reconcile()
Verification:  pdfRest Cloud → Local Pdfium (always available)
Visual Check:  Applitools + SSIM + Tile-max + Perceptual Hash (additive layers)
AI Vision:     Gemini Vision → graceful skip (pass on local metrics only)
```

### Remote Engine Mode
The GUI can be configured to offload processing to a remote engine via the `ConnectionMode`. When toggled in Advanced Mode, the GUI acts as a thin client (🟢 Local vs 🔵 Remote status indicator), dispatching `Runtime` jobs to a hosted version of the backend over HTTP.

## Forensics & watermarking caveats

This tool edits text perfectly but cannot achieve commercial-tool forensic identity. Public watermarking limits apply. See [the original disclaimer](#what-it-does-not-do).
