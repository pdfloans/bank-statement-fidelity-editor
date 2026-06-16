# FINAL SUPER PROMPT — Bank Statement Fidelity Editor v0.5.0

**Copy everything below and use it with Claude 4 Opus / Sonnet or Cursor / Zed + Claude Code.**

---

You are an expert Rust + egui + PDF engineer in May 2026.

I have a highly advanced bank statement PDF editing application. The current version (v0.3.1) already includes:

- Document-level Smart Balance Engine (Document AI + Gemini)
- Visual 300 DPI preview + high-accuracy click-to-select
- Multi-page support with full document awareness
- Undo/Redo + Change History
- Font Completion with Lipi.ai + Smart Adaption Fallback
- Professional Audit Report foundation

**Your Task:**
Take the current codebase (provided as flattened files) and make it **production-ready, beautiful, and extremely intelligent**.

**Core Requirements for v0.5.0:**

1. **Make all real API calls fully working** (Google Document AI, Gemini, Lipi.ai) with proper authentication and error handling.
2. **Improve the GUI significantly** — make it feel like a professional desktop tool (better spacing, modern design, loading spinners, progress indicators, beautiful proposed changes UI).
3. **Enhance the Smart Balance Engine** — make Gemini's proposals even smarter and more minimal.
4. **Add final polish**:
   - Better error messages everywhere
   - Progress bars for long operations
   - Keyboard shortcuts for everything important
   - Side-by-side before/after viewer
   - Export corrected statement + audit report as one PDF

**Important Guidelines:**
- Keep the code clean and well-organized.
- Maintain the current architecture (statement_engine as the brain).
- Use PyMuPDF Pro as the primary high-fidelity engine.
- Make the app feel fast, professional, and trustworthy.
- After making changes, explain what you improved and why.

Start by analyzing the current flattened codebase, then give me a clear plan before writing any code.

Now begin.