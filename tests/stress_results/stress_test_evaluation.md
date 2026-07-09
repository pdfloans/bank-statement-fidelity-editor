# Stress Test Evaluation Matrix — Bank Statement Fidelity Editor v0.5.1

**Executed:** 2026-07-09 16:33:55 UTC
**Platform:** Python 3.14.3, PyMuPDF 1.27.2.3

## Results Matrix

| Function / Task | Target Example PDF | Tested Dependencies | Score (Correctness/Fidelity) → Avg | Actionable Fallback Hierarchy |
|---|---|---|---|---|
| Test 1: Extraction | Standard_Bank_Statement_01.pdf | LlamaParse, Offline Heuristic, PyMuPDF Built-in, Mindee API, Document AI | LlamaParse (100/96) → 98, Offline Heuristic (88/98) → 93, PyMuPDF Built-in (0/95) → 48, Mindee API (0/0) → 0, Document AI (0/0) → 0 | 1. LlamaParse (Avg 98)<br>2. Offline Heuristic (Avg 93)<br>3. PyMuPDF Built-in (Avg 48) |
| Test 2: Fidelity Edit | Corrupted_Font_Ledger.pdf | pymupdfpro, Pdfium, Typst Reconstruct | pymupdfpro (90/85) → 88, Pdfium (60/92) → 76, Typst Reconstruct (75/65) → 70 | 1. pymupdfpro (Avg 88)<br>2. Pdfium (Avg 76)<br>3. Typst Reconstruct (Avg 70) |
| Test 3: Math Balance | Unbalanced_Ledger_Test.pdf | Local Math Engine, Gemini AI, Document AI | Local Math Engine (85/100) → 92, Gemini AI (0/0) → 0, Document AI (0/0) → 0 | 1. Local Math Engine (Avg 92) |
| Test 4: Visual QA | Subtle_Shift_Artifact.pdf | SSIM + Tile-Max + pHash, Applitools Eyes, pdfRest Cloud, Gemini Vision | SSIM + Tile-Max + pHash (100/100) → 100, Applitools Eyes (40/100) → 70, pdfRest Cloud (0/0) → 0, Gemini Vision (0/0) → 0 | 1. SSIM + Tile-Max + pHash (Avg 100)<br>2. Applitools Eyes (Avg 70) |

## Detailed Results Per Test

### Test 1: Extraction

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **LlamaParse** | 100 | 96 | **98.0** | 27569ms | Found 30/30 txn lines in markdown |
| 2 | **Offline Heuristic** | 88 | 98 | **93.0** | 45ms | Found 25/30 txns, closing=Y |
| 3 | **PyMuPDF Built-in** | 0 | 95 | **47.5** | 63ms | Found 0/30 txns, 0 decimal errors |
| 4 | **Mindee API** | 0 | 0 | **0.0** | 4666ms | HTTP 401: {"api_request": {"error": {"code": "Unauthorized", "details": "The token provided is for the v2 API. Please check the documentation here: https://docs.mindee.com/integrations/", "message": "Authorizat |
| 5 | **Document AI** | 0 | 0 | **0.0** | 9050ms | HTTP 401: {
  "error": {
    "code": 401,
    "message": "API keys are not supported by this API. Expected OAuth2 access token or other authentication credentials that assert a principal. See https://cloud.goog |

### Test 2: Fidelity Edit

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **pymupdfpro** | 90 | 85 | **87.5** | 97ms | Found 18 '7' glyphs, 18 successful edits, restored=Y |
| 2 | **Pdfium** | 60 | 92 | **76.0** | 351ms | Rendered 2480x3509px @ 300DPI, content=Y |
| 3 | **Typst Reconstruct** | 75 | 65 | **70.0** | 49ms | 26/26 spans extractable, 1 fonts: {'Helvetica'} |

### Test 3: Math Balance

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **Local Math Engine** | 85 | 100 | **92.5** | 6ms | Detected discrepancy=$45.0, opening=$5,000.00, closing=$10,850.21 |
| 2 | **Gemini AI** | 0 | 0 | **0.0** | 2347ms | HTTP 400: {
  "error": {
    "code": 400,
    "message": "API key not valid. Please pass a valid API key.",
    "status": "INVALID_ARGUMENT",
    "details": [
      {
        "@type": "type.googleapis.com/googl |
| 3 | **Document AI** | 0 | 0 | **0.0** | 2513ms | HTTP 401 |

### Test 4: Visual QA

| Rank | Tool | Correctness | Fidelity | Avg | Latency | Details |
|---|---|---|---|---|---|---|
| 1 | **SSIM + Tile-Max + pHash** | 100 | 100 | **100.0** | 9545ms | SSIM≈0.997970, max_tile=0.583347, diff_px=72852, region_detect=Y |
| 2 | **Applitools Eyes** | 40 | 100 | **70.0** | 2106ms | Visual AI diff=MISSED, output_len=111 |
| 3 | **pdfRest Cloud** | 0 | 0 | **0.0** | 4920ms | HTTP 404: { "error":"Invalid endpoint. Please refer to documentation at https://pdfrest.com/documentation/" } |
| 4 | **Gemini Vision** | 0 | 0 | **0.0** | 4772ms | HTTP 400 |

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
  FALLBACK-1: Applitools Eyes           (Avg=70, C=40, F=100)
```
