#[derive(Debug, Clone)]
pub enum PythonJob {
    Ping,
    GetTextBlocks {
        pdf_path: String,
        page_num: usize,
    },
    ReplaceTextInRect {
        pdf_path: String,
        output_path: String,
        page_num: usize,
        rect: [f32; 4],
        new_text: String,
        font_path: Option<String>,
    },
    FindTextBlockAtClick {
        pdf_path: String,
        page_num: usize,
        x: f32,
        y: f32,
    },
    GetAllTransactions {
        pdf_path: String,
    },
    AnalyzeDocumentLayout {
        pdf_path: String,
    },
    CompleteFontWithAdaption {
        pdf_path: String,
        font_name: String,
    },
    DeepFontReplication {
        pdf_path: String,
        font_name: String,
        output_dir: String,
    },
    /// Stage 3 / Item #14: apply N edits in one open/save pass.
    /// `edits_json` is a JSON array of `{page, rect, new_text, fill_color?}`.
    ApplyManyEdits {
        pdf_path: String,
        output_path: String,
        edits_json: String,
        font_path: Option<String>,
    },
    /// Stage 3 / Item #16: split a PDF into chunks ≤30 pages so Document AI
    /// can parse documents above its single-request page cap.
    ChunkPdfForDocai {
        pdf_path: String,
        output_dir: String,
        max_pages_per_chunk: usize,
    },
    /// Stage 8.5: per-font usage + coverage analysis. Returns the JSON
    /// shape produced by `pymupdf_pro_integration.analyze_fonts`.
    AnalyzeFonts {
        pdf_path: String,
    },
    /// Stage 11: targeted font cascade. Runs composite synthesis →
    /// subset extension → Gemini Vision donor identification on the
    /// supplied `missing_chars`. Returns the JSON dict produced by
    /// `replicate_font_for_chars`.
    ReplicateFontForMissingChars {
        pdf_path: String,
        font_name: String,
        missing_chars_csv: String,
        output_dir: String,
    },
    /// Clone (duplicate) pages within a PDF to create capacity for more
    /// transactions. Each entry in `page_indices` is a source page to clone;
    /// clones are inserted immediately after the original. Does NOT require
    /// PyMuPDF Pro — page-level operations use the free tier.
    ClonePages {
        pdf_path: String,
        output_path: String,
        page_indices: Vec<usize>,
    },
    /// Remove pages from a PDF (excess capacity). Pages are deleted in
    /// descending order so indices don't shift. Does NOT require PyMuPDF Pro.
    RemovePages {
        pdf_path: String,
        output_path: String,
        page_indices: Vec<usize>,
    },
}

#[derive(Debug)]
pub enum PythonJobResult {
    Pong,
    Json(String),
    ReplacedWithReviewWarning { reason: String },
    Success,
    Error(String),
}
