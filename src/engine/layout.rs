//! Document-Level Layout Analysis
//!
//! Analyzes the ENTIRE multi-page bank statement for structural and visual layout.
//! This helps the Smart Balance Engine make smarter decisions and preserve fidelity.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLayout {
    pub page_number: usize,
    pub has_header: bool,
    pub has_footer: bool,
    pub has_page_number: bool,
    pub table_columns: usize,
    pub main_text_style: String,
    pub dominant_font: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentLayout {
    pub total_pages: usize,
    pub pages: Vec<PageLayout>,
    pub has_consistent_headers: bool,
    pub has_consistent_footers: bool,
    pub overall_style: String,
    pub layout_confidence: f32,
}

impl Default for DocumentLayout {
    fn default() -> Self {
        Self {
            total_pages: 0,
            pages: vec![],
            has_consistent_headers: false,
            has_consistent_footers: false,
            overall_style: "Unknown".to_string(),
            layout_confidence: 0.0,
        }
    }
}

use crate::app::runtime::{Job, PythonJob};
use std::sync::mpsc::Sender;
use tokio::sync::oneshot;

/// Analyze the ENTIRE document layout using PyMuPDF Pro
pub fn analyze_document_layout(
    job_tx: &Sender<Job>,
    pdf_path: &Path,
) -> Result<oneshot::Receiver<crate::app::runtime::PythonJobResult>, String> {
    tracing::info!("[LAYOUT ANALYZER] Starting document-level layout analysis...");

    let (reply_tx, reply_rx) = oneshot::channel();
    job_tx
        .send(Job::Python(
            PythonJob::AnalyzeDocumentLayout {
                pdf_path: pdf_path.to_string_lossy().to_string(),
            },
            reply_tx,
        ))
        .map_err(|e| e.to_string())?;

    Ok(reply_rx)
}
