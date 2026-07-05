# FINAL SUPER PROMPT — Bank Statement Fidelity Editor v0.5.1

**Copy everything below and use it with Claude 4 Opus / Sonnet or Cursor / Zed + Claude Code.**

---

You are an expert Rust + egui + PDF engineer in July 2026.

I have a highly advanced bank statement PDF editing application. The current version (v0.5.1) includes:

- **Multi-Backend Pipeline Architecture:** Configurable primary backends with automatic fallback chains at every stage (parsing, editing, verification, AI validation).
- **Boot-Time API Availability Detection:** On startup, all API keys are probed and availability is displayed in the Backend Preferences UI (✅ / ⛔).
- **Document Parsers:** Mindee Financial Doc (default), LlamaParse, Google Document AI, PyMuPDF Built-in, Local OCR — all with auto-fallback to offline parser.
- **PDF Edit Engines:** PyMuPDF (primary, via PyO3), Pdfium (native fallback), Typst Reconstruct (ultimate fail-safe).
- **Smart Balance Engine:** Document AI + Gemini AI proposals → local deterministic balance fallback.
- **Multi-Layer Verification:** SSIM + Tile-max + Perceptual Hash (always) → pdfRest Cloud (optional) → Applitools Eyes Visual AI (optional) → Gemini Vision (optional).
- **Visual 300 DPI preview** + high-accuracy click-to-select
- **Multi-page support** with full document awareness
- **Undo/Redo** + Change History + Audit Report auto-merge
- **Font Analysis, Replication, and Completion** with deep font subsetting
- **Transfer Transactions** between PDFs with font replication
- **Date Period Adjustment** (shift forward/backward)
- **Batch Processing Dashboard** for concurrent multi-PDF operations
- **CLI + GUI parity** — same Runtime job loop drives both interfaces
- **Backend Preferences UI** with per-stage backend selection, availability indicators, and visual threshold sliders

**Your Task:**
Take the current codebase (provided as flattened files) and make further improvements.

**Important Guidelines:**
- Keep the code clean and well-organized.
- Maintain the current five-layer architecture (`app/`, `engine/`, `pdf/`, `extractors/`, `ai/`, `security/`).
- Every new cloud integration MUST have an offline fallback.
- New API keys must register in `ApiAvailability` and show in Backend Preferences UI.
- Use PyMuPDF Pro as the primary high-fidelity engine.
- Make the app feel fast, professional, and trustworthy.
- After making changes, explain what you improved and why.

Start by analyzing the current flattened codebase, then give me a clear plan before writing any code.

Now begin.