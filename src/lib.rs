//! Bank Statement Fidelity Editor v1.0.0
//! Public API

pub mod ai;
pub mod app;
pub mod engine;
pub mod error; // Unified error types
pub mod extractors;
pub mod pdf;
pub mod security;

pub use crate::error::{
    AIError, AppError, AuditError, BalanceError, CacheError, ConfigError, DocumentAIError,
    ExtractionError, PdfRestError, TextEditError, VerificationError as AppVerificationError,
};

pub use engine::balance::process_and_reconcile;
pub use engine::verification::{verify_edit, VerificationReport};
