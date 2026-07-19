use dual_core_pdf_pipeline::engine::font_shaping::calculate_exact_width;
use std::path::PathBuf;

fn get_test_font_path() -> PathBuf {
    // Assuming the test is run from the workspace root
    PathBuf::from("assets/Inter-Regular.ttf")
}

#[test]
fn test_calculate_exact_width() {
    let font_path = get_test_font_path();
    
    // Skip the test if the asset isn't present in the environment
    if !font_path.exists() {
        println!("Test skipped: assets/Inter-Regular.ttf not found");
        return;
    }

    let text = "Hello, World!";
    let font_size = 12.0;
    
    // Measure the width
    let width = calculate_exact_width(&font_path, text, font_size).expect("Failed to shape font");
    
    // Width should be positive and reasonable for 12pt text
    assert!(width > 0.0);
    assert!(width < 200.0); // "Hello, World!" at 12pt is usually around 70-90 pts wide
    
    // Ensure consistent behavior (a specific text should have the same deterministic width)
    let width_again = calculate_exact_width(&font_path, text, font_size).unwrap();
    assert_eq!(width, width_again);

    // Empty text should be 0 width
    let empty_width = calculate_exact_width(&font_path, "", font_size).unwrap();
    assert_eq!(empty_width, 0.0);
}
