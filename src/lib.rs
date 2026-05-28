//! Bank Statement Fidelity Editor v0.4.0
//! Public API

pub mod ai;
pub mod app;
pub mod engine;
pub mod extractors;
pub mod pdf;
pub mod security;

pub use engine::balance::process_and_reconcile;
pub use engine::verification::{verify_edit, VerificationError, VerificationReport};
