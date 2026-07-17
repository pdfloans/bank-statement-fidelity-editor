use crate::ai::document_ai::BankStatement;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum TypstEngineError {
    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Typst Compilation Error: {0}")]
    Typst(String),
}

#[derive(Default)]
pub struct TypstEngine;

impl TypstEngine {
    pub fn new() -> Self {
        Self
    }

    /// Reconstructs a bank statement as a brand new PDF using Typst in-process.
    pub async fn reconstruct_pdf(
        &self,
        statement: &BankStatement,
        output_path: &Path,
    ) -> Result<(), TypstEngineError> {
        tracing::info!("[typst_engine] Starting in-process PDF reconstruction");
        let markup = self.generate_markup(statement);

        // We use spawn_blocking because typst compilation is CPU intensive
        let out_path = output_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let world = ReconstructWorld::new(markup);

            match typst::compile(&world).output {
                Ok(document) => {
                    // Generate PDF
                    let pdf_bytes = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
                        .map_err(|e| {
                            TypstEngineError::Typst(format!("PDF generation failed: {:?}", e))
                        })?;

                    std::fs::write(&out_path, pdf_bytes).map_err(TypstEngineError::Io)?;
                    tracing::info!("[typst_engine] Successfully compiled PDF in-process");
                    Ok(())
                }
                Err(diags) => {
                    let errs = diags
                        .into_iter()
                        .map(|d| d.message.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    tracing::error!("[typst_engine] Typst compilation failed: {}", errs);
                    Err(TypstEngineError::Typst(errs))
                }
            }
        })
        .await
        .map_err(|e| TypstEngineError::Typst(format!("Panic in typst thread: {}", e)))?
    }

    fn generate_markup(&self, stmt: &BankStatement) -> String {
        let mut out = String::new();
        out.push_str("#set page(margin: 1in)\n");
        // We embed Inter, so let's use it
        out.push_str("#set text(font: \"Inter\", size: 10pt)\n\n");
        out.push_str("= Bank Statement\n\n");

        if let Some(ref acc) = stmt.account_number {
            out.push_str(&format!("*Account Number:* {}\n\n", acc));
        }

        out.push_str(&format!(
            "*Opening Balance:* \\${}\n\n",
            stmt.opening_balance
        ));

        out.push_str("#table(\n");
        out.push_str("  columns: (1fr, 3fr, 1fr, 1fr),\n");
        out.push_str("  [**Date**], [**Description**], [**Debit**], [**Credit**],\n");

        for tx in &stmt.transactions {
            let date = tx.date.clone();
            let desc = tx.raw_text.replace("[", "\\[").replace("]", "\\]");
            let debit = tx.debit.map(|d| format!("\\${}", d)).unwrap_or_default();
            let credit = tx.credit.map(|c| format!("\\${}", c)).unwrap_or_default();

            out.push_str(&format!(
                "  [{}], [{}], [{}], [{}],\n",
                date, desc, debit, credit
            ));
        }
        out.push_str(")\n\n");

        out.push_str(&format!(
            "*Closing Balance:* \\${}\n\n",
            stmt.closing_balance
        ));

        out
    }
}

// Minimal Typst World for in-process compilation
use typst::diag::{FileError, FileResult};
use typst::foundations::Datetime;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::World;

struct ReconstructWorld {
    library: typst::utils::LazyHash<typst::Library>,
    book: typst::utils::LazyHash<FontBook>,
    fonts: Vec<Font>,
    source: Source,
}

impl ReconstructWorld {
    fn new(source_text: String) -> Self {
        let font_data = include_bytes!("../../assets/Inter-Regular.ttf");
        let font = Font::new(typst::foundations::Bytes::new(font_data.to_vec()), 0)
            .expect("Failed to parse Inter-Regular");

        let font_bold_data = include_bytes!("../../assets/Inter-Bold.ttf");
        let font_bold = Font::new(typst::foundations::Bytes::new(font_bold_data.to_vec()), 0)
            .expect("Failed to parse Inter-Bold");

        let fonts = vec![font, font_bold];
        let book = typst::utils::LazyHash::new(FontBook::from_fonts(&fonts));
        use typst::LibraryExt;
        let library = typst::utils::LazyHash::new(typst::Library::builder().build());
        let source = Source::new(FileId::new(None, VirtualPath::new("main.typ")), source_text);

        Self {
            library,
            book,
            fonts,
            source,
        }
    }
}

impl World for ReconstructWorld {
    fn library(&self) -> &typst::utils::LazyHash<typst::Library> {
        &self.library
    }
    fn book(&self) -> &typst::utils::LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(
                id.vpath().as_rootless_path().to_path_buf(),
            ))
        }
    }

    fn file(&self, id: FileId) -> FileResult<typst::foundations::Bytes> {
        Err(FileError::NotFound(
            id.vpath().as_rootless_path().to_path_buf(),
        ))
    }

    fn font(&self, id: usize) -> Option<Font> {
        self.fonts.get(id).cloned()
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}
