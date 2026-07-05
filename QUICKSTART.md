# Quickstart Guide

Get up and running with the Bank Statement Fidelity Editor v0.5.0 in 7 steps.

## 1. Install System Dependencies

You must install system-level binaries for rendering, OCR geometry extraction, and visual testing.

**Windows:**
- Visual Studio 2019 Build Tools (v142)
- Python 3.10+ (`python --version` to confirm)
- Node.js 18+ (`node --version` to confirm, needed for Applitools)

**macOS:**
```bash
brew install mupdf tesseract leptonica
```

**Ubuntu:**
```bash
apt-get install libmupdf-dev tesseract-ocr libleptonica-dev
```

## 2. Clone and Install Python & Node Packages

Clone the repository and install the Python bindings for PyMuPDF:

```bash
git clone <repository>
cd bank-statement-modifier
pip install pymupdf pymupdfpro fonttools pillow
npm install @applitools/eyes-images   # optional: for Applitools visual AI
```

## 3. Configure Environment

Copy the `.env.example` file to `.env` and fill in your API keys:

```bash
cp .env.example .env
```

### Required Keys
| Variable | Where to Get It |
|---|---|
| `DUAL_CORE_PASSPHRASE` | Choose a strong passphrase (≥16 chars) |
| `GEMINI_API_KEY` | [Google AI Studio](https://aistudio.google.com/app/apikey) |

### Recommended Keys (enable cloud pipelines)
| Variable | Where to Get It | What It Enables |
|---|---|---|
| `MINDEE_API_KEY` | [Mindee Platform](https://platform.mindee.com/) | Default document parser (highest accuracy) |
| `LLAMAPARSE_API_KEY` | [LlamaCloud](https://cloud.llamaindex.ai/) | LLM-based document parser (alternative) |
| `DOCUMENT_AI_*` | [Google Cloud Console](https://console.cloud.google.com/) | Google Document AI parser + admin |
| `PDFREST_API_KEY` | [pdfRest](https://pdfrest.com/) | Adobe-tier cloud rendering for verification |
| `APPLITOOLS_API_KEY` | [Applitools Eyes](https://eyes.applitools.com/) | Visual AI diff testing (additive layer) |
| `PYMUPDF_PRO_KEY` | [PyMuPDF Pro](https://pymupdf.io/try-pro/) | Enhanced font handling in edit engine |

> **Note:** Every cloud backend has an automatic offline fallback. The app works with zero API keys — you just get fewer features. The Backend Preferences panel shows which backends are available (✅) or unavailable (⛔).

## 4. Build the Project

Build the Rust pipeline:

```bash
cargo build --release
```

For development with relaxed passphrase checks:

```bash
cargo build --release --features dev
```

## 5. Launch the GUI

Start the graphical interface:

```bash
./target/release/dual-core-pdf-pipeline gui
```

On startup, the app will:
1. Load your `.env` configuration
2. Probe all API keys for availability
3. Log which backends are available (check the console or Settings → Backend Preferences)

## 6. First Edit Walkthrough

1. **Load a Document:** Enter the path to your PDF in the left panel and click "Load Entire Statement".
2. **Select Text:** Click any text block directly on the rendered PDF canvas. The targeted edit panel will populate with the selected text.
3. **Edit & Apply:** Modify the text in the "Modified" box and click "🎯 Apply Change". The editor ensures 100% visual fidelity.
4. **Smart Balancing:** Click "⚖️ Balance Statement" to run document-wide imbalance analysis. Review and apply the AI-proposed cascading adjustments (or local offline analysis if AI is not configured).
5. **Verify:** Check the "🔍 Verify Edits" tool to confirm the integrity of the math and visual layout against the original document.
6. **Backend Preferences:** Open Settings → Backend Preferences to see all available backends, change parsers, adjust visual thresholds, or switch AI providers.
7. **Advanced Mode:** Toggle "Advanced Mode" in the top menu bar to reveal deeper forensic tools (Deep Font Replication, pdfRest validation, Typst reconstruction).

## 7. Batch Processing

1. Switch to the **Batch Processing** tab in the top menu bar.
2. Drag and drop a folder containing multiple PDFs onto the dashboard.
3. Click **Extract All to JSON** or **Auto-Balance All** to process all statements concurrently in the background.

### Using the CLI

The application offers 100% CLI parity. You can perform the same flows from the terminal:

```bash
# Balance a statement automatically
./target/release/dual-core-pdf-pipeline balance --input examples/sample.pdf --output output.pdf --auto-approve

# Extract JSON data
./target/release/dual-core-pdf-pipeline extract --input examples/sample.pdf --output data.json

# Perform a targeted text edit
./target/release/dual-core-pdf-pipeline text --input examples/sample.pdf --output output.pdf --old "1,000.00" --new "2,000.00" --bbox 10,20,50,40

# Transfer transactions between PDFs
./target/release/dual-core-pdf-pipeline transfer-transactions --source-pdf a.pdf --target-pdf b.pdf -o out.pdf

# Run health check
./target/release/dual-core-pdf-pipeline doctor

# Verify all API keys
./target/release/dual-core-pdf-pipeline verify-api-keys
```
