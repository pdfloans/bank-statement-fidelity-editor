use super::engine::*;
use std::path::Path;
use crate::app::runtime::{PythonJob, PythonJobResult};
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct PyMuPdfEngine {
    job_tx: std::sync::mpsc::Sender<crate::app::runtime::Job>,
}

impl PyMuPdfEngine {
    pub fn new(job_tx: std::sync::mpsc::Sender<crate::app::runtime::Job>) -> Self {
        Self { job_tx }
    }
}

impl PdfEngine for PyMuPdfEngine {
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            supports_redaction: true,
            supports_cjk: true,
            supports_embedded_fonts: true,
            estimated_fidelity: 0.95,
        }
    }

    fn render_page(&self, _path: &Path, _page: usize, _dpi: f32) -> Result<RenderedPage, EngineError> {
        // We use pdfium/mupdf for rendering, PyMuPDF for editing
        Err(EngineError::Unsupported)
    }

    fn get_text_blocks(&self, path: &Path, page: usize) -> Result<Vec<TextBlock>, EngineError> {
        let (tx, rx) = oneshot::channel();
        self.job_tx.send(crate::app::runtime::Job::Python(
            PythonJob::GetTextBlocks { pdf_path: path.to_string_lossy().to_string(), page_num: page },
            tx
        )).map_err(|_| EngineError::ExtractFailed("Worker thread disconnected".into()))?;

        // This blocks the calling thread (usually a spawn_blocking tokio task)
        let res = rx.blocking_recv().map_err(|e| EngineError::ExtractFailed(e.to_string()))?;
        
        match res {
            PythonJobResult::Json(json) => {
                let blocks = serde_json::from_str(&json).map_err(|e| EngineError::ExtractFailed(e.to_string()))?;
                Ok(blocks)
            }
            PythonJobResult::Error(e) => Err(EngineError::ExtractFailed(e)),
            _ => Err(EngineError::ExtractFailed("Unexpected result".into()))
        }
    }

    fn find_text_block_at_click(&self, path: &Path, page: usize, x: f32, y: f32) -> Result<Option<TextBlock>, EngineError> {
        let (tx, rx) = oneshot::channel();
        self.job_tx.send(crate::app::runtime::Job::Python(
            PythonJob::FindTextBlockAtClick { pdf_path: path.to_string_lossy().to_string(), page_num: page, x, y },
            tx
        )).map_err(|_| EngineError::ExtractFailed("Worker thread disconnected".into()))?;

        let res = rx.blocking_recv().map_err(|e| EngineError::ExtractFailed(e.to_string()))?;
        
        match res {
            PythonJobResult::Json(json) => {
                let trimmed = json.trim();
                if trimmed == "null" || trimmed.is_empty() {
                    return Ok(None);
                }
                let block = serde_json::from_str(&json).map_err(|e| EngineError::ExtractFailed(e.to_string()))?;
                Ok(Some(block))
            }
            PythonJobResult::Error(e) => Err(EngineError::ExtractFailed(e)),
            _ => Ok(None)
        }
    }

    fn apply_change(
        &self, 
        input: &Path, 
        output: &Path, 
        page: usize, 
        bbox: [f32; 4], 
        new_text: &str,
        font_path: Option<&Path>
    ) -> Result<ReplaceOutcome, EngineError> {
        let (tx, rx) = oneshot::channel();
        self.job_tx.send(crate::app::runtime::Job::Python(
            PythonJob::ReplaceTextInRect { 
                pdf_path: input.to_string_lossy().to_string(), 
                output_path: output.to_string_lossy().to_string(), 
                page_num: page, 
                rect: bbox, 
                new_text: new_text.to_string(),
                font_path: font_path.map(|p| p.to_string_lossy().to_string()),
            },
            tx
        )).map_err(|_| EngineError::ApplyFailed("Worker thread disconnected".into()))?;

        let res = rx.blocking_recv().map_err(|e| EngineError::ApplyFailed(e.to_string()))?;
        
        match res {
            PythonJobResult::Success => Ok(ReplaceOutcome { success: true, font_used: "unknown".into(), overflow: false }),
            PythonJobResult::ReplacedWithReviewWarning { .. } => Ok(ReplaceOutcome { success: true, font_used: "unknown".into(), overflow: false }),
            PythonJobResult::Error(e) => Err(EngineError::ApplyFailed(e)),
            _ => Err(EngineError::ApplyFailed("Unexpected result".into()))
        }
    }

    fn analyze_layout(&self, path: &Path) -> Result<DocumentLayout, EngineError> {
        let (tx, rx) = oneshot::channel();
        self.job_tx.send(crate::app::runtime::Job::Python(
            PythonJob::AnalyzeDocumentLayout { pdf_path: path.to_string_lossy().to_string() },
            tx
        )).map_err(|_| EngineError::LayoutFailed("Worker thread disconnected".into()))?;

        let res = rx.blocking_recv().map_err(|e| EngineError::LayoutFailed(e.to_string()))?;
        
        match res {
            PythonJobResult::Json(json) => {
                let pages: Vec<crate::engine::layout::PageLayout> = serde_json::from_str(&json).map_err(|e| EngineError::LayoutFailed(e.to_string()))?;
                Ok(DocumentLayout {
                    total_pages: pages.len(),
                    has_consistent_headers: pages.iter().all(|p| p.has_header),
                    has_consistent_footers: pages.iter().all(|p| p.has_footer),
                    pages,
                    overall_style: "Professional Bank Statement".to_string(),
                    layout_confidence: 0.85,
                })
            }
            PythonJobResult::Error(e) => Err(EngineError::LayoutFailed(e)),
            _ => Err(EngineError::LayoutFailed("Unexpected result".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::{Job, PythonJobResult};
    use std::sync::mpsc;

    #[test]
    fn find_text_block_at_click_returns_none_on_null_json() {
        let (job_tx, job_rx) = mpsc::channel();
        let engine = PyMuPdfEngine::new(job_tx);
        
        std::thread::spawn(move || {
            if let Ok(Job::Python(_, reply_tx)) = job_rx.recv() {
                let _ = reply_tx.send(PythonJobResult::Json("null".into()));
            }
        });

        let res = engine.find_text_block_at_click(Path::new("test.pdf"), 0, 0.0, 0.0).unwrap();
        assert!(res.is_none());
    }
}
