use dual_core_pdf_pipeline::pdf::engine::{EngineError, PdfEngine};
use dual_core_pdf_pipeline::pdf::native_engine::OxidizePdfEngine;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream, StringFormat};
use std::path::Path;

/// Helper to generate a multi-page PDF for testing cloning and removing.
fn create_multipage_pdf(path: &Path, num_pages: usize) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => font_id,
        },
    });

    let mut page_ids = vec![];

    for i in 0..num_pages {
        let text = format!("Page {}", i + 1);
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.0.into()]),
            Operation::new(
                "Tm",
                vec![
                    1.0.into(),
                    0.0.into(),
                    0.0.into(),
                    1.0.into(),
                    50.0.into(),
                    700.0.into(),
                ],
            ),
            Operation::new(
                "Tj",
                vec![Object::String(text.into_bytes(), StringFormat::Literal)],
            ),
            Operation::new("ET", vec![]),
        ];

        let content = Content { operations };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.0.into(), 0.0.into(), 595.0.into(), 842.0.into()],
        });
        page_ids.push(page_id.into());
    }

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => page_ids,
        "Count" => num_pages as i32,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    doc.save(path).unwrap();
}

#[test]
fn test_native_engine_clone_pages() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("clone_in.pdf");
    let output = dir.path().join("clone_out.pdf");

    create_multipage_pdf(&input, 3); // Pages 1, 2, 3 (indices 0, 1, 2)

    let engine = OxidizePdfEngine::new();
    
    // Clone page index 1 (Page 2). This appends it to the end.
    engine.clone_pages(&input, &output, vec![1]).unwrap();

    let doc = Document::load(&output).unwrap();
    let pages = doc.get_pages();
    
    assert_eq!(pages.len(), 4);
    
    // We expect the texts: "Page 1", "Page 2", "Page 3", "Page 2"
    let out_blocks_0 = engine.get_text_blocks(&output, 0).unwrap();
    let out_blocks_1 = engine.get_text_blocks(&output, 1).unwrap();
    let out_blocks_2 = engine.get_text_blocks(&output, 2).unwrap();
    let out_blocks_3 = engine.get_text_blocks(&output, 3).unwrap();
    
    assert_eq!(out_blocks_0[0].text, "Page 1");
    assert_eq!(out_blocks_1[0].text, "Page 2");
    assert_eq!(out_blocks_2[0].text, "Page 3");
    assert_eq!(out_blocks_3[0].text, "Page 2");
}

#[test]
fn test_native_engine_remove_pages() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("remove_in.pdf");
    let output = dir.path().join("remove_out.pdf");

    create_multipage_pdf(&input, 4); // Pages 1, 2, 3, 4

    let engine = OxidizePdfEngine::new();
    
    // Remove pages 1 and 2 (indices 1, 2)
    engine.remove_pages(&input, &output, vec![1, 2]).unwrap();

    let doc = Document::load(&output).unwrap();
    let pages = doc.get_pages();
    
    assert_eq!(pages.len(), 2);
    
    // Check that we have 2 pages
    // We expect the texts: "Page 1", "Page 4"
    let out_blocks_0 = engine.get_text_blocks(&output, 0).unwrap();
    let out_blocks_1 = engine.get_text_blocks(&output, 1).unwrap();
    
    assert_eq!(out_blocks_0[0].text, "Page 1");
    assert_eq!(out_blocks_1[0].text, "Page 4");
}

#[test]
fn test_native_engine_analyze_layout() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("layout_in.pdf");

    create_multipage_pdf(&input, 2);

    let engine = OxidizePdfEngine::new();
    
    let layout = engine.analyze_layout(&input).unwrap();
    assert_eq!(layout.total_pages, 2);
    // Since there are no repeated headers across pages in this simple test PDF,
    // has_consistent_headers will be false (or maybe true if there's no header).
    // Let's just check it doesn't crash.
}

#[test]
fn test_native_engine_apply_change() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("apply_in.pdf");
    let output = dir.path().join("apply_out.pdf");

    create_multipage_pdf(&input, 1); // Page 1

    let engine = OxidizePdfEngine::new();
    
    // First, find the text block to get its bounding box
    let blocks = engine.get_text_blocks(&input, 0).unwrap();
    assert_eq!(blocks.len(), 1);
    let bbox = blocks[0].bbox;
    let old_text = &blocks[0].text;
    assert_eq!(old_text, "Page 1");

    // Apply the change
    let new_text = "Replaced 1";
    engine.apply_change(&input, &output, 0, bbox, new_text, old_text, None).unwrap();

    // Verify the change
    let modified_blocks = engine.get_text_blocks(&output, 0).unwrap();
    assert_eq!(modified_blocks.len(), 1);
    assert_eq!(modified_blocks[0].text, new_text);
}

#[test]
fn test_native_engine_non_ascii_guard() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("ascii_in.pdf");
    let output = dir.path().join("ascii_out.pdf");

    create_multipage_pdf(&input, 1);

    let engine = OxidizePdfEngine::new();
    let blocks = engine.get_text_blocks(&input, 0).unwrap();
    let bbox = blocks[0].bbox;

    // Try applying a non-ASCII string (emoji)
    let new_text = "Page 📈";
    let result = engine.apply_change(&input, &output, 0, bbox, new_text, "Page 1", None);

    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(err_str.contains("ASCII"));
}
