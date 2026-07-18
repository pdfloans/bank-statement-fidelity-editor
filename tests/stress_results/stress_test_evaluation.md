# Stress Test Evaluation Matrix — Bank Statement Fidelity Editor v0.5.1

**Executed:** 2026-07-18 18:05:28 UTC
**Platform:** Python 3.14.5, PyMuPDF 1.28.0

## Results Matrix

| Function / Task | Target Example PDF | Tested Dependencies | Score (Correctness/Fidelity) → Avg | Actionable Fallback Hierarchy |
|---|---|---|---|---|
| Test 1: Extraction | Standard_Bank_Statement_01.pdf | LlamaParse, Offline Heuristic, PyMuPDF Built-in, Document AI, Mindee API | LlamaParse (100/96) → 98, Offline Heuristic (88/98) → 93, PyMuPDF Built-in (0/95) → 48, Document AI (0/0) → 0, Mindee API (0/0) → 0 | 1. LlamaParse (Avg 98)<br>2. Offline Heuristic (Avg 93)<br>3. PyMuPDF Built-in (Avg 48) |
| Test 2: Fidelity Edit | Corrupted_Font_Ledger.pdf | pymupdfpro, Pdfium, Typst Reconstruct | pymupdfpro (90/85) → 88, Pdfium (60/92) → 76, Typst Reconstruct (75/65) → 70 | 1. pymupdfpro (Avg 88)<br>2. Pdfium (Avg 76)<br>3. Typst Reconstruct (Avg 70) |
| Test 3: Math Balance | Unbalanced_Ledger_Test.pdf | Local Math Engine, OpenRouter AI, Document AI, Gemini AI, Groq AI | Local Math Engine (100/100) → 100, OpenRouter AI (0/100) → 50, Document AI (0/0) → 0, Gemini AI (0/0) → 0, Groq AI (0/0) → 0 | 1. Local Math Engine (Avg 100)<br>2. OpenRouter AI (Avg 50) |
| Test 4: Visual QA | Subtle_Shift_Artifact.pdf | SSIM + Tile-Max + pHash, pdfRest Cloud, Applitools Eyes, Gemini Vision | SSIM + Tile-Max + pHash (100/100) → 100, pdfRest Cloud (75/98) → 86, Applitools Eyes (0/0) → 0, Gemini Vision (0/0) → 0 | 1. SSIM + Tile-Max + pHash (Avg 100)<br>2. pdfRest Cloud (Avg 86) |
| Test 5: Transfer Transactions | ? | Groq (Llama 3), Gemini 1.5 Flash, OpenRouter | Groq (Llama 3) (100/100) → 100, Gemini 1.5 Flash (0/0) → 0, OpenRouter (0/0) → 0 | 1. Groq (Llama 3) (Avg 100) |
| Test 6: E2E GUI Testing | ? | Rust UIAutomation | Rust UIAutomation (100/100) → 100 | 1. Rust UIAutomation (Avg 100) |
| Test 7: PII Anonymization | ? | Groq PII, Gemini 1.5 PII | Groq PII (100/100) → 100, Gemini 1.5 PII (0/0) → 0 | 1. Groq PII (Avg 100) |
| Test 8: Forensic Evasion | ? | PyMuPDF Pro, Typst Reconstruct, Pdfium | PyMuPDF Pro (100/100) → 100, Typst Reconstruct (80/100) → 90, Pdfium (0/0) → 0 | 1. PyMuPDF Pro (Avg 100)<br>2. Typst Reconstruct (Avg 90) |

## Detailed Results Per Test

### Test 1: Extraction

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **LlamaParse** | 100 | 96 | **98.0** | 6605ms | Found 30/30 txn lines in markdown |
| 2 | **Offline Heuristic** | 88 | 98 | **93.0** | 22ms | Found 25/30 txns, closing=Y |
| 3 | **PyMuPDF Built-in** | 0 | 95 | **47.5** | 25ms | Found 0/30 txns, 0 decimal errors |
| 4 | **Document AI** | 0 | 0 | **0.0** | 126ms | No auth available |
| 5 | **Mindee API** | 0 | 0 | **0.0** | 580ms | API Error: HTTP 401 |

### Test 2: Fidelity Edit

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **pymupdfpro** | 90 | 85 | **87.5** | 128ms | Found 18 '7' glyphs, 18 successful edits, restored=Y |
| 2 | **Pdfium** | 60 | 92 | **76.0** | 121ms | Rendered 2480x3509px @ 300DPI, content=Y |
| 3 | **Typst Reconstruct** | 75 | 65 | **70.0** | 116ms | 26/26 spans extractable, 1 fonts: {'Helvetica'} |

### Test 3: Math Balance

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **Local Math Engine** | 100 | 100 | **100.0** | 3ms | Detected discrepancy=$45.0, opening=$5,000.00, closing=$10,850.21 |
| 2 | **OpenRouter AI** | 0 | 100 | **50.0** | 3711ms | Found=N, amount=$0, line=0 |
| 3 | **Document AI** | 0 | 0 | **0.0** | 6ms | CRASH: [Errno 2] No such file or directory: 'gcloud.cmd' |
| 4 | **Gemini AI** | 0 | 0 | **0.0** | 43ms | HTTP 400: {
  "error": {
    "code": 400,
    "message": "API key not valid. Please pass a valid API key.",
    "status": "INVALID_ARGUMENT",
    "details": [
      {
        "@type": "type.googleapis.com/googl |
| 5 | **Groq AI** | 0 | 0 | **0.0** | 106ms | HTTP 400 |

### Test 4: Visual QA

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **SSIM + Tile-Max + pHash** | 100 | 100 | **100.0** | 2589ms | SSIM≈0.997970, max_tile=0.583347, diff_px=72852, region_detect=Y |
| 2 | **pdfRest Cloud** | 75 | 98 | **86.5** | 2780ms | Adobe-tier render at 300DPI, manual diff required |
| 3 | **Applitools Eyes** | 0 | 0 | **0.0** | 198ms | Bridge script not found |
| 4 | **Gemini Vision** | 0 | 0 | **0.0** | 426ms | HTTP 400 |

### Test 5: Transfer Transactions

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **Groq (Llama 3)** | 100 | 100 | **100.0** | 1051ms | Transfer mapped perfectly |
| 2 | **Gemini 1.5 Flash** | 0 | 0 | **0.0** | 24ms | API Error No response |
| 3 | **OpenRouter** | 0 | 0 | **0.0** | 734ms | API Error No response |

### Test 6: E2E GUI Testing

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **Rust UIAutomation** | 100 | 100 | **100.0** | 18696ms | GUI launched and tree attached correctly |

### Test 7: PII Anonymization

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **Groq PII** | 100 | 100 | **100.0** | 906ms | PII Masked Successfully |
| 2 | **Gemini 1.5 PII** | 0 | 0 | **0.0** | 26ms | API Error No response |

### Test 8: Forensic Evasion

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **PyMuPDF Pro** | 100 | 100 | **100.0** | 1ms | Producer: '', Creator: '', EOF markers: 1 |
| 2 | **Typst Reconstruct** | 80 | 100 | **90.0** | 0ms | Clean rebuild, single %%EOF, but likely leaves Typst metadata tag |
| 3 | **Pdfium** | 0 | 0 | **0.0** | 0ms | Output is an image raster, not a vector PDF. Fails structural forensics completely. |

## Production Fallback Routing Logic

Based on the empirical scores above, the recommended fallback chains are:

### Test 1: Extraction
```
  PRIMARY: LlamaParse                (Avg=98, C=100, F=96)
  FALLBACK-1: Offline Heuristic         (Avg=93, C=88, F=98)
  FALLBACK-2: PyMuPDF Built-in          (Avg=48, C=0, F=95)
```

### Test 2: Fidelity Edit
```
  PRIMARY: pymupdfpro                (Avg=88, C=90, F=85)
  FALLBACK-1: Pdfium                    (Avg=76, C=60, F=92)
  FALLBACK-2: Typst Reconstruct         (Avg=70, C=75, F=65)
```

### Test 3: Math Balance
```
  PRIMARY: Local Math Engine         (Avg=100, C=100, F=100)
  FALLBACK-1: OpenRouter AI             (Avg=50, C=0, F=100)
```

### Test 4: Visual QA
```
  PRIMARY: SSIM + Tile-Max + pHash   (Avg=100, C=100, F=100)
  FALLBACK-1: pdfRest Cloud             (Avg=86, C=75, F=98)
```

### Test 5: Transfer Transactions
```
  PRIMARY: Groq (Llama 3)            (Avg=100, C=100, F=100)
```

### Test 6: E2E GUI Testing
```
  PRIMARY: Rust UIAutomation         (Avg=100, C=100, F=100)
```

### Test 7: PII Anonymization
```
  PRIMARY: Groq PII                  (Avg=100, C=100, F=100)
```

### Test 8: Forensic Evasion
```
  PRIMARY: PyMuPDF Pro               (Avg=100, C=100, F=100)
  FALLBACK-1: Typst Reconstruct         (Avg=90, C=80, F=100)
```
