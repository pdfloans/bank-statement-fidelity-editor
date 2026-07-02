use crate::ai::document_ai::BankStatement;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum TypstEngineError {
    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Typst Compilation Error: {0}")]
    Typst(String),
    #[error("Font Subsetting Error: {0}")]
    Subsetting(String),
}

#[derive(Default)]
pub struct TypstEngine;

impl TypstEngine {
    pub fn new() -> Self {
        Self
    }

    /// Reconstructs a bank statement as a brand new PDF using Typst.
    pub async fn reconstruct_pdf(
        &self,
        statement: &BankStatement,
        output_path: &Path,
    ) -> Result<(), TypstEngineError> {
        let typ_markup = self.generate_markup(statement);

        let temp_dir = std::env::temp_dir().join("typst_reconstruct");
        std::fs::create_dir_all(&temp_dir)?;

        let typ_path = temp_dir.join("statement.typ");
        std::fs::write(&typ_path, &typ_markup)?;

        // Font subsetting using pure Rust `subsetter`
        tracing::info!(
            "[typst_engine] Subsetting fonts for reconstructed document ({} transactions)",
            statement.transactions.len()
        );

        let font_path = std::path::PathBuf::from("assets/bank_font.ttf");
        if font_path.exists() {
            let font_data = std::fs::read(&font_path)?;
            if let Ok(face) = ttf_parser::Face::parse(&font_data, 0) {
                let mut glyphs = vec![];
                for ch in typ_markup.chars() {
                    if let Some(glyph_id) = face.glyph_index(ch) {
                        glyphs.push(glyph_id.0 as u16);
                    }
                }
                let mapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);

                if let Ok(subset) = subsetter::subset(&font_data, 0, &mapper) {
                    let subset_path = temp_dir.join("bank_font_subset.ttf");
                    std::fs::write(&subset_path, subset)?;
                    tracing::info!("[typst_engine] Font successfully subsetted and saved to {:?}", subset_path);
                } else {
                    tracing::warn!("[typst_engine] Subsetter failed, relying on default font embedding");
                }
            }
        } else {
            tracing::warn!("[typst_engine] Source font not found at {:?}", font_path);
        }

        // Note: Full programmatic Typst compilation requires implementing the `World` trait
        // which provides fonts, files, and standard library primitives.
        // For now, we will shell out to the typst CLI if available, or just save the `.typ` file.
        // We added `typst` to Cargo.toml so we can theoretically compile it in-process, but
        // bootstrapping the default fonts and `World` is complex.

        let out = std::process::Command::new("typst")
            .arg("compile")
            .arg(&typ_path)
            .arg(output_path)
            .output();

        match out {
            Ok(output) if output.status.success() => {
                tracing::info!("[typst_engine] Successfully compiled PDF via Typst");
                Ok(())
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(TypstEngineError::Typst(err.to_string()))
            }
            Err(e) => {
                // Typst CLI not found, fallback to just writing the file
                tracing::warn!(
                    "[typst_engine] Typst CLI not found, saving .typ file only: {}",
                    e
                );
                std::fs::copy(&typ_path, output_path)?;
                Ok(())
            }
        }
    }

    fn generate_markup(&self, stmt: &BankStatement) -> String {
        let mut out = String::new();
        out.push_str("#set page(margin: 1in)\n");
        out.push_str("#set text(font: \"Helvetica\", size: 10pt)\n\n");
        out.push_str("= Bank Statement\n\n");

        if let Some(ref acc) = stmt.account_number {
            out.push_str(&format!("*Account Number:* {}\n\n", acc));
        }

        out.push_str(&format!("*Opening Balance:* ${}\n\n", stmt.opening_balance));

        out.push_str("#table(\n");
        out.push_str("  columns: (1fr, 3fr, 1fr, 1fr),\n");
        out.push_str("  [**Date**], [**Description**], [**Debit**], [**Credit**],\n");

        for tx in &stmt.transactions {
            let date = tx.date.clone();
            let desc = tx.raw_text.replace("[", "\\[").replace("]", "\\]");
            let debit = tx.debit.map(|d| format!("${}", d)).unwrap_or_default();
            let credit = tx.credit.map(|c| format!("${}", c)).unwrap_or_default();

            out.push_str(&format!(
                "  [{}], [{}], [{}], [{}],\n",
                date, desc, debit, credit
            ));
        }
        out.push_str(")\n\n");

        out.push_str(&format!("*Closing Balance:* ${}\n\n", stmt.closing_balance));

        out
    }
}
