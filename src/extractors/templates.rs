use std::path::Path;
use std::fs;
use serde::{Deserialize, Serialize};
use super::geometry::*;
use regex::Regex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankTemplate {
    pub id: String,
    pub header_signatures: Vec<String>,
    pub date_format: String,
    pub amount_regex: String,
    pub column_x_ranges: std::collections::HashMap<String, [f32; 2]>,
}

pub struct BankTemplateProvider {
    pub templates: Vec<BankTemplate>,
    pub engine: std::sync::Arc<dyn crate::pdf::PdfEngine>,
}

impl BankTemplateProvider {
    pub fn new(template_dir: &Path, engine: std::sync::Arc<dyn crate::pdf::PdfEngine>) -> Self {
        let mut templates = Vec::new();
        if let Ok(entries) = fs::read_dir(template_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|s| s.to_str()) == Some("yaml") {
                    if let Ok(content) = fs::read_to_string(entry.path()) {
                        if let Ok(template) = serde_yaml::from_str::<BankTemplate>(&content) {
                            templates.push(template);
                        }
                    }
                }
            }
        }
        Self { templates, engine }
    }
}

impl GeometryProvider for BankTemplateProvider {
    fn extract_line_geometry(&self, pdf_path: &Path) -> Result<Vec<LineGeometry>, ExtractorError> {
        let mut geometries = Vec::new();
        
        // 1. Get layout to know total pages
        let layout = self.engine.analyze_layout(pdf_path)
            .map_err(|e| ExtractorError::ExtractionFailed(e.to_string()))?;

        for page in 0..layout.total_pages {
            let blocks = self.engine.get_text_blocks(pdf_path, page).unwrap_or_default();
            let page_text = blocks.iter().map(|b| b.text.clone()).collect::<Vec<_>>().join(" ");

            // 2. Identify template
            for template in &self.templates {
                let matches_all = template.header_signatures.iter().all(|sig| page_text.contains(sig));
                
                if matches_all {
                    tracing::info!("Matched template '{}' on page {}", template.id, page);
                    
                    // 3. Simple row-based extraction using template-defined columns
                    // This is a placeholder for actual column-based slicing
                    for (i, block) in blocks.iter().enumerate() {
                        geometries.push(LineGeometry {
                            page,
                            line_on_page: i,
                            text: block.text.clone(),
                            bbox: block.bbox,
                            confidence: 1.0,
                            source: GeometrySource::BankTemplate { template_id: template.id.clone() },
                        });
                    }
                }
            }
        }

        Ok(geometries)
    }
}
