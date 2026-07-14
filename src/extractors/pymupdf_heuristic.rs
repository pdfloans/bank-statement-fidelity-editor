use super::geometry::*;
use crate::pdf::PdfEngine;
use regex::Regex;
use std::path::Path;
use std::sync::Arc;

pub struct PyMuPdfHeuristicProvider {
    pub engine: Arc<dyn PdfEngine>,
}

impl GeometryProvider for PyMuPdfHeuristicProvider {
    fn extract_line_geometry(&self, pdf_path: &Path) -> Result<Vec<LineGeometry>, ExtractorError> {
        let mut geometries = Vec::new();

        // We do a simplified mock of PyMuPdf's layout analysis using the get_text_blocks from engine
        // Assuming engine.analyze_layout() gets pages
        let layout = self
            .engine
            .analyze_layout(pdf_path)
            .map_err(|e| ExtractorError::ExtractionFailed(e.to_string()))?;

        static DATE_PATTERN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r"(?i)(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec|\d{1,2}/\d{1,2})")
                .unwrap()
        });
        static AMOUNT_PATTERN: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new(r"-?\$?[\d,]+\.\d{2}").unwrap());

        for page in 0..layout.total_pages {
            let blocks = self
                .engine
                .get_text_blocks(pdf_path, page)
                .unwrap_or_default();

            // Simple row clustering based on y-coordinate
            let mut current_y = None;
            let mut current_row = Vec::new();
            let mut row_idx = 0;

            for block in blocks {
                let y_center = (block.bbox[1] + block.bbox[3]) / 2.0;
                if current_y.is_none() {
                    current_y = Some(y_center);
                    current_row.push(block);
                } else if let Some(y) = current_y {
                    if (y_center - y).abs() < 5.0 {
                        current_row.push(block);
                    } else {
                        // Process row
                        process_row(
                            &current_row,
                            page,
                            row_idx,
                            &mut geometries,
                            &DATE_PATTERN,
                            &AMOUNT_PATTERN,
                        );
                        current_row.clear();
                        current_row.push(block);
                        current_y = Some(y_center);
                        row_idx += 1;
                    }
                }
            }
            if !current_row.is_empty() {
                process_row(
                    &current_row,
                    page,
                    row_idx,
                    &mut geometries,
                    &DATE_PATTERN,
                    &AMOUNT_PATTERN,
                );
            }
        }

        Ok(geometries)
    }
}

fn process_row(
    row: &[crate::pdf::TextBlock],
    page: usize,
    line_on_page: usize,
    geometries: &mut Vec<LineGeometry>,
    date_pattern: &Regex,
    amount_pattern: &Regex,
) {
    if row.is_empty() {
        return;
    }

    let mut text = String::new();
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    for block in row {
        text.push_str(&block.text);
        text.push(' ');
        min_x = min_x.min(block.bbox[0]);
        min_y = min_y.min(block.bbox[1]);
        max_x = max_x.max(block.bbox[2]);
        max_y = max_y.max(block.bbox[3]);
    }

    let text = text.trim().to_string();

    // Naive filter for transaction rows
    if date_pattern.is_match(&text) && amount_pattern.is_match(&text) {
        geometries.push(LineGeometry {
            page,
            line_on_page,
            text,
            bbox: [min_x, min_y, max_x, max_y],
            confidence: 0.8,
            source: GeometrySource::TextLayer,
        });
    }
}
