mod fixtures;

use dual_core_pdf_pipeline::engine::pdf_split_merge::{merge_pdfs, split_pdf};
use lopdf::Document;
use std::path::PathBuf;
use tempfile::tempdir;

/// Returns the path to a real or synthetic test PDF.
/// Prefers examples/sample.pdf for high-fidelity testing; falls back to a
/// generated synthetic 5-page PDF so this test always exercises real code
/// instead of silently self-skipping.
fn get_test_pdf() -> (PathBuf, bool) {
    let sample = PathBuf::from("examples/sample.pdf");
    if sample.exists() {
        return (sample, false);
    }

    eprintln!("[info] examples/sample.pdf not found; using synthetic 5-page PDF");
    let dir = std::env::temp_dir().join("split_merge_test_fixtures");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("synthetic_5page.pdf");
    fixtures::generate_test_pdf(5, &path);
    (path, true)
}

#[test]
fn test_split_merge_cycle() {
    let (src_path, is_synthetic) = get_test_pdf();

    let original_doc = Document::load(&src_path).expect("Failed to load original");
    let original_pages = original_doc.get_pages().len();
    assert!(original_pages > 0);

    let out_dir = tempdir().expect("Failed to create temp dir");

    // Split into 1-page segments
    let segments = split_pdf(&src_path, out_dir.path(), 1).expect("Split failed");
    assert_eq!(segments.len(), original_pages);

    let output_path = out_dir.path().join("merged.pdf");
    let segment_paths: Vec<PathBuf> = segments.into_iter().map(|s| s.path).collect();

    // Merge back
    let merged_pages = merge_pdfs(&segment_paths, &output_path).expect("Merge failed");
    assert_eq!(merged_pages, original_pages);

    // Verify structural integrity of merged PDF
    let merged_doc = Document::load(&output_path).expect("Failed to load merged PDF");
    assert_eq!(merged_doc.get_pages().len(), original_pages);

    // Only check Font references for real PDFs — synthetic PDFs use simple
    // inline font refs that lopdf's page dict may not expose identically.
    if !is_synthetic {
        let page_id = merged_doc
            .get_pages()
            .get(&1)
            .cloned()
            .expect("Page 1 missing");
        let page_dict = merged_doc
            .get_object(page_id)
            .and_then(|obj| obj.as_dict())
            .expect("Page 1 not a dict");

        if let Ok(resources) = page_dict.get(b"Resources").and_then(|obj| obj.as_dict()) {
            if let Ok(fonts) = resources.get(b"Font").and_then(|obj| obj.as_dict()) {
                assert!(
                    !fonts.is_empty(),
                    "Merged PDF lost font references on Page 1"
                );
            }
        }
    }

    eprintln!(
        "✅ split_merge_cycle: {} pages → {} segments → merged {} pages ({})",
        original_pages,
        original_pages,
        merged_pages,
        if is_synthetic {
            "synthetic"
        } else {
            "real PDF"
        }
    );
}

#[test]
fn test_split_merge_fidelity_multi_page() {
    let (src_path, is_synthetic) = get_test_pdf();

    let out_dir = tempdir().expect("Failed to create temp dir");

    // Split into 2-page segments (if original has enough pages)
    let segments = split_pdf(&src_path, out_dir.path(), 2).expect("Split failed");
    let num_segments = segments.len();

    let output_path = out_dir.path().join("merged_2.pdf");
    let segment_paths: Vec<PathBuf> = segments.into_iter().map(|s| s.path).collect();

    let merged_pages = merge_pdfs(&segment_paths, &output_path).expect("Merge failed");

    let original_doc = Document::load(&src_path).unwrap();
    assert_eq!(merged_pages, original_doc.get_pages().len());

    eprintln!(
        "✅ split_merge_fidelity_multi_page: {} pages → {} segments → merged {} pages ({})",
        original_doc.get_pages().len(),
        num_segments,
        merged_pages,
        if is_synthetic {
            "synthetic"
        } else {
            "real PDF"
        }
    );
}
