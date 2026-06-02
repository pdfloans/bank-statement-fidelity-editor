pub mod engine;
pub mod mupdf_engine;
pub mod pymupdf_engine;
pub mod selector;

pub use engine::*;
pub use mupdf_engine::MuPdfEngine;
pub use pymupdf_engine::PyMuPdfEngine;
pub use selector::PdfEngineSelector;

pub fn get_pdfium_instance() -> pdfium_render::prelude::Pdfium {
    use pdfium_render::prelude::*;
    // Fallback: look in the executable's directory first because if we are in a Mac .app bundle,
    // the working directory is /, but the library is next to the executable.
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop(); // get directory
        let lib_name = "libpdfium.dylib";
        let lib_path = exe.join(lib_name);
        if lib_path.exists() {
            if let Ok(bindings) = Pdfium::bind_to_library(lib_path.to_string_lossy().as_ref()) {
                return Pdfium::new(bindings);
            }
        }
    }

    // Default behavior (searches DYLD_LIBRARY_PATH, cwd, etc)
    Pdfium::default()
}
