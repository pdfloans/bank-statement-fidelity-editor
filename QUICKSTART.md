# Quickstart Guide

Get up and running with the Bank Statement Fidelity Editor v0.4.0 in 6 steps.

## 1. Install System Dependencies

You must install system-level binaries for rendering and OCR geometry extraction.

**Windows:**
Install Visual Studio 2019 Build Tools (v142) and Tesseract/Leptonica binaries.
Python 3.10+ required.

**macOS:**
```bash
brew install mupdf tesseract leptonica
```

**Ubuntu:**
```bash
apt-get install libmupdf-dev tesseract-ocr libleptonica-dev
```

## 2. Clone and Install Python Packages

Clone the repository and install the Python bindings for PyMuPDF Pro:

```bash
git clone <repository>
cd bank-statement-modifier
pip install pymupdf pymupdfpro
```

## 3. Configure Environment

Copy the `.env.example` file to `.env` and fill in the required keys:

```bash
cp .env.example .env
```
Ensure you provide `DUAL_CORE_PASSPHRASE` and your `GEMINI_API_KEY`.

## 4. Build the Project

Build the Rust pipeline. The `dev` feature allows you to bypass strict passphrase length checks for local testing.

```bash
cargo build --release --features dev
```

## 5. Launch the GUI

Start the graphical interface:

```bash
./target/release/dual-core-pdf-pipeline gui
```

## 6. First Edit Walkthrough

1. **Load a Document:** Enter the path to your PDF in the left panel and click "Load Entire Statement".
2. **Select Text:** Click any text block directly on the rendered PDF canvas. The targeted edit panel will populate with the selected text.
3. **Edit & Apply:** Modify the text in the "Modified" box and click "🎯 Apply Change". The editor ensures 100% visual fidelity.
4. **Smart Balancing:** Click "⚖️ Balance Statement" to run the document-wide imbalance analysis. Review and apply the AI-proposed cascading adjustments.
5. **Verify:** Check the "🔍 Verify Edits" tool to confirm the integrity of the math and visual layout against the original document.

### Using the CLI

The application offers 100% CLI parity. You can perform the same flows from the terminal:

```bash
# Balance a statement automatically
./target/release/dual-core-pdf-pipeline balance --input examples/sample.pdf --output output.pdf --auto-approve

# Extract JSON data
./target/release/dual-core-pdf-pipeline extract --input examples/sample.pdf --output data.json

# Perform a targeted text edit
./target/release/dual-core-pdf-pipeline text --input examples/sample.pdf --output output.pdf --old "1,000.00" --new "2,000.00" --bbox 10,20,50,40
```
