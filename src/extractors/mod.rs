pub mod geometry;
pub mod merger;
pub mod ocrs_engine;
// pymupdf_heuristic removed — replaced by native oxidize-pdf extraction
// tesseract removed — replaced by ocrs (Phase 5)
pub mod templates;

pub use geometry::*;
pub use merger::*;
pub use ocrs_engine::OcrsEngine;
pub use templates::{learn_template, parsers, BankTemplate, BankTemplateProvider};
