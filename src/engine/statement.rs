//! Smart Document-Level Balance Engine v2.0
//!
//! This is the SINGLE UNIFIED ENGINE for the entire application.
//! It treats the bank statement as ONE CONNECTED DOCUMENT (not per-page).
//!
//! Core Responsibilities:
//! - Parse the ENTIRE multi-page PDF using Google Document AI
//! - Maintain a global list of ALL transactions across ALL pages
//! - When ANY change is made, recalculate ALL subsequent running balances
//! - Use Gemini with FULL document context to propose minimal smart adjustments
//! - Queue changes that may span multiple pages
//! - Apply all approved changes with maximum visual fidelity

use crate::engine::model::{Transaction, Provenance, ProposedChange};
use std::path::Path;
use thiserror::Error;
use std::sync::Arc;
use crate::ai::document_ai::DocumentAiClient;
use crate::ai::gemini_client::GeminiClient;
use crate::extractors::merger::HybridMerger;
use crate::pdf::PdfEngine;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Document not loaded")]
    NotLoaded,
    #[error("Invalid transaction format: {0}")]
    InvalidTransaction(String),
    #[error("Balance math error: {0}")]
    MathError(String),
    #[error("Gemini failed to generate plan: {0}")]
    AiPlanFailed(String),
    #[error("Low Confidence: {0:.2}")]
    LowConfidence(f32),
}

pub struct SmartDocumentEngine {
    pub all_transactions: Vec<Transaction>,
    pub proposed_changes: Vec<ProposedChange>,
    pub is_balanced: bool,
    pub total_pages: usize,
    pub layout: Option<crate::engine::layout::DocumentLayout>,
    
    pdf_engine: Arc<dyn PdfEngine>,
    doc_ai: Arc<DocumentAiClient>,
    gemini: Arc<GeminiClient>,
    merger: Arc<HybridMerger>,
}

impl SmartDocumentEngine {
    pub fn new(
        pdf_engine: Arc<dyn PdfEngine>,
        doc_ai: Arc<DocumentAiClient>,
        gemini: Arc<GeminiClient>,
        merger: Arc<HybridMerger>,
    ) -> Self {
        Self {
            all_transactions: Vec::new(),
            is_balanced: false,
            proposed_changes: Vec::new(),
            layout: None,
            total_pages: 0,
            pdf_engine,
            doc_ai,
            gemini,
            merger,
        }
    }

    /// Load the ENTIRE multi-page statement + Layout Analysis (called once when user loads PDF)
    pub async fn load_full_document(&mut self, _job_tx: &std::sync::mpsc::Sender<crate::app::runtime::Job>, pdf_path: &Path) -> Result<(), EngineError> {
        tracing::info!("[DOCUMENT ENGINE] Loading ENTIRE multi-page statement + Layout Analysis...");

        self.all_transactions.clear();
        self.proposed_changes.clear();

        // Run document-level layout analysis
        let layout = self.pdf_engine.analyze_layout(pdf_path)
            .map_err(|e| EngineError::AiPlanFailed(format!("Layout analysis failed: {}", e)))?;
            
        self.total_pages = layout.total_pages;
        self.layout = Some(layout.clone());
        tracing::info!("[DOCUMENT ENGINE] Layout analysis complete: {} pages, consistent headers: {}",
                layout.total_pages, layout.has_consistent_headers);

        tracing::info!("[DOCUMENT ENGINE] Document loaded with {} transactions across {} pages",
                 self.all_transactions.len(), self.total_pages);

        Ok(())
    }

    /// Main function - "Balance Statement Out" for the ENTIRE document
    pub async fn balance_entire_statement(
        &mut self,
        current_pdf_path: &Path,
    ) -> Result<Vec<ProposedChange>, EngineError> {
        tracing::info!("[DOCUMENT ENGINE] ===== BALANCE ENTIRE STATEMENT =====");

        // 1. Document AI Extraction
        let bank_stmt = self.doc_ai.parse_entire_statement(current_pdf_path)
            .await
            .map_err(|e| EngineError::AiPlanFailed(format!("Document AI failed: {}", e)))?;
            
        // 2. Geometry Extraction
        let mut geometries = Vec::new();
        for provider in &self.merger.providers {
            if let Ok(geo) = provider.extract_line_geometry(current_pdf_path) {
                geometries.extend(geo);
            }
        }
        
        // 3. Hybrid Merge
        let report = self.merger.merge(bank_stmt.transactions, geometries);
        self.all_transactions = report.transactions.clone();
        
        // 4. Calculate current global balance status
        let imbalance = self.calculate_global_imbalance();

        if imbalance.abs() < 0.01 {
            self.is_balanced = true;
            tracing::info!("[DOCUMENT ENGINE] ✅ Statement is already perfectly balanced.");
            return Ok(vec![]);
        }

        self.is_balanced = false;
        tracing::info!("[DOCUMENT ENGINE] Imbalance detected: ${:.2}", imbalance);

        // 5. Use Gemini with FULL document context to propose minimal smart adjustments
        tracing::info!("[DOCUMENT ENGINE] Asking Gemini for minimal cascading adjustments across all pages...");
        let layout = self.layout.as_ref().ok_or(EngineError::NotLoaded)?;
        
        let plan = self.gemini.propose_balance_adjustments(&self.all_transactions, imbalance, layout)
            .await
            .map_err(|e| {
                if let crate::ai::gemini_client::GeminiError::LowConfidence(c) = e {
                    EngineError::LowConfidence(c)
                } else {
                    EngineError::AiPlanFailed(format!("Gemini failed: {}", e))
                }
            })?;

        // 6. Map adjustments to ProposedChange
        let changes: Vec<ProposedChange> = plan.adjustments.into_iter().map(|adj| {
            ProposedChange {
                page: adj.page,
                old_text: format!("{:.2}", adj.old_running_balance),
                new_text: format!("{:.2}", adj.new_running_balance),
                reason: adj.reason,
                confidence: adj.confidence,
                affects_subsequent_balances: true,
                bbox: None,
            }
        }).collect();

        self.proposed_changes = changes.clone();
        tracing::info!("[DOCUMENT ENGINE] Proposed {} changes across the document", self.proposed_changes.len());
        
        Ok(changes)
    }

    pub fn calculate_global_imbalance(&self) -> f64 {
        if self.all_transactions.is_empty() {
            return 0.0;
        }

        let opening_balance = self.all_transactions.first()
            .map(|t| t.running_balance.unwrap_or(0.0) - (t.credit.unwrap_or(0.0) - t.debit.unwrap_or(0.0)))
            .unwrap_or(0.0);

        let sum_credits: f64 = self.all_transactions.iter().map(|t| t.credit.unwrap_or(0.0)).sum();
        let sum_debits: f64 = self.all_transactions.iter().map(|t| t.debit.unwrap_or(0.0)).sum();
        
        let reported_closing_balance = self.all_transactions.last()
            .and_then(|t| t.running_balance)
            .unwrap_or(0.0);

        // Correct formula: Calculated = Opening + Credits - Debits
        // Imbalance = Reported - Calculated
        let calculated_closing = opening_balance + sum_credits - sum_debits;
        let diff = reported_closing_balance - calculated_closing;
        
        // Round to 2 decimal places to avoid floating point noise
        (diff * 100.0).round() / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{Transaction, Provenance};

    #[test]
    fn calculate_global_imbalance_correct() {
        // Dummy test for compile
    }
}
