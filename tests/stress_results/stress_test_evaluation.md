# Stress Test Evaluation Matrix — Bank Statement Fidelity Editor v0.5.1

**Executed:** 2026-07-13 10:45:49 UTC
**Platform:** Python 3.14.3, PyMuPDF 1.27.2.3

## Results Matrix

| Function / Task | Target Example PDF | Tested Dependencies | Score (Correctness/Fidelity) → Avg | Actionable Fallback Hierarchy |
|---|---|---|---|---|
| Test 1: Extraction | Standard_Bank_Statement_01.pdf | LlamaParse, Offline Heuristic, PyMuPDF Built-in, Document AI, Mindee API | LlamaParse (100/96) → 98, Offline Heuristic (88/98) → 93, PyMuPDF Built-in (0/95) → 48, Document AI (0/0) → 0, Mindee API (0/0) → 0 | 1. LlamaParse (Avg 98)<br>2. Offline Heuristic (Avg 93)<br>3. PyMuPDF Built-in (Avg 48) |
| Test 2: Fidelity Edit | Corrupted_Font_Ledger.pdf | pymupdfpro, Pdfium, Typst Reconstruct | pymupdfpro (90/85) → 88, Pdfium (60/92) → 76, Typst Reconstruct (75/65) → 70 | 1. pymupdfpro (Avg 88)<br>2. Pdfium (Avg 76)<br>3. Typst Reconstruct (Avg 70) |
| Test 3: Math Balance | Unbalanced_Ledger_Test.pdf | Local Math Engine, Groq AI, OpenRouter AI, Gemini AI, Document AI | Local Math Engine (85/100) → 92, Groq AI (0/0) → 0, OpenRouter AI (0/0) → 0, Gemini AI (0/0) → 0, Document AI (0/0) → 0 | 1. Local Math Engine (Avg 92) |
| Test 4: Visual QA | Subtle_Shift_Artifact.pdf | SSIM + Tile-Max + pHash, pdfRest Cloud, Applitools Eyes, Gemini Vision | SSIM + Tile-Max + pHash (100/100) → 100, pdfRest Cloud (75/98) → 86, Applitools Eyes (40/100) → 70, Gemini Vision (0/0) → 0 | 1. SSIM + Tile-Max + pHash (Avg 100)<br>2. pdfRest Cloud (Avg 86)<br>3. Applitools Eyes (Avg 70) |

## Detailed Results Per Test

### Test 1: Extraction

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **LlamaParse** | 100 | 96 | **98.0** | 10472ms | Found 30/30 txn lines in markdown |
| 2 | **Offline Heuristic** | 88 | 98 | **93.0** | 35ms | Found 25/30 txns, closing=Y |
| 3 | **PyMuPDF Built-in** | 0 | 95 | **47.5** | 49ms | Found 0/30 txns, 0 decimal errors |
| 4 | **Document AI** | 0 | 0 | **0.0** | 3173ms | HTTP 401: {
  "error": {
    "code": 401,
    "message": "API keys are not supported by this API. Expected OAuth2 access token or other authentication credentials that assert a principal. See https://cloud.goog |
| 5 | **Mindee API** | 0 | 0 | **0.0** | 5064ms | API Error: HTTP 422 - Invalid fields in form :: 422-009 - One or more fields failed validation. |

### Test 2: Fidelity Edit

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **pymupdfpro** | 90 | 85 | **87.5** | 117ms | Found 18 '7' glyphs, 18 successful edits, restored=Y |
| 2 | **Pdfium** | 60 | 92 | **76.0** | 573ms | Rendered 2480x3509px @ 300DPI, content=Y |
| 3 | **Typst Reconstruct** | 75 | 65 | **70.0** | 43ms | 26/26 spans extractable, 1 fonts: {'Helvetica'} |

### Test 3: Math Balance

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **Local Math Engine** | 85 | 100 | **92.5** | 22ms | Detected discrepancy=$45.0, opening=$5,000.00, closing=$10,850.21 |
| 2 | **Groq AI** | 0 | 0 | **0.0** | 0ms | API key not configured |
| 3 | **OpenRouter AI** | 0 | 0 | **0.0** | 0ms | API key not configured |
| 4 | **Gemini AI** | 0 | 0 | **0.0** | 2093ms | HTTP 400: {
  "error": {
    "code": 400,
    "message": "API key not valid. Please pass a valid API key.",
    "status": "INVALID_ARGUMENT",
    "details": [
      {
        "@type": "type.googleapis.com/googl |
| 5 | **Document AI** | 0 | 0 | **0.0** | 3030ms | HTTP 401 |

### Test 4: Visual QA

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **SSIM + Tile-Max + pHash** | 100 | 100 | **100.0** | 30237ms | SSIM≈0.997970, max_tile=0.583347, diff_px=72852, region_detect=Y |
| 2 | **pdfRest Cloud** | 75 | 98 | **86.5** | 8405ms | Adobe-tier render at 300DPI, manual diff required |
| 3 | **Applitools Eyes** | 40 | 100 | **70.0** | 6560ms | Visual AI diff=MISSED, output_len=111 |
| 4 | **Gemini Vision** | 0 | 0 | **0.0** | 7110ms | HTTP 400 |

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
  PRIMARY: Local Math Engine         (Avg=92, C=85, F=100)
```

### Test 4: Visual QA
```
  PRIMARY: SSIM + Tile-Max + pHash   (Avg=100, C=100, F=100)
  FALLBACK-1: pdfRest Cloud             (Avg=86, C=75, F=98)
  FALLBACK-2: Applitools Eyes           (Avg=70, C=40, F=100)
```
