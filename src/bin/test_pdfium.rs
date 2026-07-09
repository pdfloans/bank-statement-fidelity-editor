use std::sync::Arc;
use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::pdf::native_engine::pdfium_resolver;

fn main() {
    dotenvy::dotenv().ok();
    println!("Testing Pdfium Resolver with Auto-Download...");
    match pdfium_resolver::resolve() {
        Ok(path) => println!("Success! Found pdfium at: {:?}", path),
        Err(e) => println!("Error: {}", e),
    }
}
