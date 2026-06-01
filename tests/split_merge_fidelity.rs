use dual_core_pdf_pipeline::engine::pdf_split_merge::{split_pdf, merge_pdfs};
use lopdf::Document;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn test_split_merge_cycle() {
    let src_path = PathBuf::from("examples/sample.pdf");
    if !src_path.exists() {
        eprintln!("Skipping test: examples/sample.pdf not found");
        return;
    }

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
    
    // Verify that some resources (e.g. Fonts) are still there.
    // We check the first page for Font references.
    let page_id = merged_doc.get_pages().get(&1).cloned().expect("Page 1 missing");
    let page_dict = merged_doc.get_object(page_id).and_then(|obj| obj.as_dict()).expect("Page 1 not a dict");
    
    if let Ok(resources) = page_dict.get(b"Resources").and_then(|obj| obj.as_dict()) {
        if let Ok(fonts) = resources.get(b"Font").and_then(|obj| obj.as_dict()) {
            assert!(!fonts.is_empty(), "Merged PDF lost font references on Page 1");
        }
    }
}

#[test]
fn test_split_merge_fidelity_multi_page() {
    let src_path = PathBuf::from("examples/sample.pdf");
    if !src_path.exists() {
        return;
    }

    let out_dir = tempdir().expect("Failed to create temp dir");
    
    // Split into 2-page segments (if original has enough pages)
    let segments = split_pdf(&src_path, out_dir.path(), 2).expect("Split failed");
    
    let output_path = out_dir.path().join("merged_2.pdf");
    let segment_paths: Vec<PathBuf> = segments.into_iter().map(|s| s.path).collect();
    
    let merged_pages = merge_pdfs(&segment_paths, &output_path).expect("Merge failed");
    
    let original_doc = Document::load(&src_path).unwrap();
    assert_eq!(merged_pages, original_doc.get_pages().len());
}
