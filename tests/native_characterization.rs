use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::pdf::engine::{EngineError, PdfEngine};
use dual_core_pdf_pipeline::pdf::native_engine::OxidizePdfEngine;
use dual_core_pdf_pipeline::pdf::selector::PdfEngineSelector;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream, StringFormat};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Helper to generate a simple PDF with specific text elements at exact coordinates.
fn create_simple_pdf(path: &Path, strings: &[(&str, f32, f32)]) {
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

    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.0.into()]),
    ];
    for (s, x, y) in strings {
        operations.push(Operation::new(
            "Tm",
            vec![
                1.0.into(),
                0.0.into(),
                0.0.into(),
                1.0.into(),
                (*x).into(),
                (*y).into(),
            ],
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(s.as_bytes().to_vec(), StringFormat::Literal)],
        ));
    }
    operations.push(Operation::new("ET", vec![]));

    let content = Content { operations };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.0.into(), 0.0.into(), 595.0.into(), 842.0.into()],
    });
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
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
fn test_native_engine_apply_change_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("single_edit_in.pdf");
    let output = dir.path().join("single_edit_out.pdf");

    create_simple_pdf(&input, &[("100.00", 50.0, 700.0)]);

    let engine = OxidizePdfEngine::new();
    let blocks = engine.get_text_blocks(&input, 0).unwrap();
    assert_eq!(blocks.len(), 1);
    let target_bbox = blocks[0].bbox;

    engine
        .apply_change(&input, &output, 0, target_bbox, "200.00", "100.00", None)
        .unwrap();

    let out_blocks = engine.get_text_blocks(&output, 0).unwrap();
    assert_eq!(out_blocks.len(), 1);
    assert_eq!(out_blocks[0].text, "200.00");
}

#[test]
#[ignore] // TODO(T2): un-ignore once selector write path uses apply_change_guarded
fn test_native_engine_repeated_value_target() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("repeated_edit_in.pdf");
    let output = dir.path().join("repeated_edit_out.pdf");

    // Two identical strings at different Y coordinates (e.g. transaction amount and running balance)
    create_simple_pdf(&input, &[("100.00", 50.0, 700.0), ("100.00", 50.0, 680.0)]);

    let engine = OxidizePdfEngine::new();
    let blocks = engine.get_text_blocks(&input, 0).unwrap();
    assert_eq!(blocks.len(), 2);

    // Target the first one specifically by its exact bbox
    let target_bbox = blocks[0].bbox;

    engine
        .apply_change(&input, &output, 0, target_bbox, "200.00", "100.00", None)
        .unwrap();

    let out_blocks = engine.get_text_blocks(&output, 0).unwrap();
    assert_eq!(out_blocks.len(), 2);
    // The first one should be changed, the second should remain unchanged
    assert_eq!(out_blocks[0].text, "200.00");
    assert_eq!(out_blocks[1].text, "100.00");
}

#[test]
#[ignore] // TODO(T2): un-ignore once selector write path uses apply_change_guarded
fn test_selector_rejects_non_overlapping_bbox() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("guard_in.pdf");
    let output = dir.path().join("guard_out.pdf");

    create_simple_pdf(&input, &[("Target", 50.0, 700.0)]);

    let primary = Arc::new(OxidizePdfEngine::new());
    let fallback = Arc::new(OxidizePdfEngine::new());
    let config = Arc::new(Mutex::new(Arc::new(AppConfig::default())));
    let selector = PdfEngineSelector::new(primary, fallback, config);

    // Provide a bbox completely outside the bounds of the "Target" text (e.g. y=100.0)
    let bad_bbox = [50.0, 100.0, 100.0, 115.0];

    let result = selector.apply_change(&input, &output, 0, bad_bbox, "Hacked", "Target", None);

    match result {
        Err(EngineError::RowDrifted { .. }) => {} // Expected rejection
        other => panic!("Expected RowDrifted error, got: {:?}", other),
    }
}

#[test]
fn test_native_engine_apply_many_edits_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("multi_edit_in.pdf");
    let output = dir.path().join("multi_edit_out.pdf");

    create_simple_pdf(&input, &[("First", 50.0, 700.0), ("Second", 50.0, 600.0)]);

    let engine = OxidizePdfEngine::new();
    let blocks = engine.get_text_blocks(&input, 0).unwrap();
    assert_eq!(blocks.len(), 2);

    let edits_json = serde_json::json!([
        {
            "page": 0,
            "rect": blocks[0].bbox,
            "old_text": "First",
            "new_text": "Alpha"
        },
        {
            "page": 0,
            "rect": blocks[1].bbox,
            "old_text": "Second",
            "new_text": "Beta"
        }
    ]).to_string();

    engine.apply_many_edits(&input, &output, &edits_json, None).unwrap();

    let out_blocks = engine.get_text_blocks(&output, 0).unwrap();
    assert_eq!(out_blocks.len(), 2);
    assert_eq!(out_blocks[0].text, "Alpha");
    assert_eq!(out_blocks[1].text, "Beta");
}
