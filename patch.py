import sys

with open('src/app/gui.rs', 'r', encoding='utf-8') as f:
    gui = f.read()

# Fix 1: Add fields to struct MyApp
struct_old = "edit_pymupdf_pro_key: String,"
struct_new = """edit_pymupdf_pro_key: String,
    edit_mindee_api_key: String,
    edit_llamaparse_api_key: String,
    edit_pdfrest_api_key: String,
    edit_applitools_api_key: String,"""
gui = gui.replace(struct_old, struct_new)

# Fix 2: Initialize fields in MyApp::new
init_old = 'edit_pymupdf_pro_key: std::env::var("PYMUPDF_PRO_KEY").unwrap_or_default(),'
init_new = """edit_pymupdf_pro_key: std::env::var("PYMUPDF_PRO_KEY").unwrap_or_default(),
            edit_mindee_api_key: std::env::var("MINDEE_API_KEY").unwrap_or_default(),
            edit_llamaparse_api_key: std::env::var("LLAMAPARSE_API_KEY").unwrap_or_default(),
            edit_pdfrest_api_key: std::env::var("PDFREST_API_KEY").unwrap_or_default(),
            edit_applitools_api_key: std::env::var("APPLITOOLS_API_KEY").unwrap_or_default(),"""
gui = gui.replace(init_old, init_new)

# Fix 3: Add to pairs in save_credentials
pairs_old = """(
                "PYMUPDF_PRO_KEY",
                self.edit_pymupdf_pro_key.trim().to_string(),
            ),"""
pairs_new = """(
                "PYMUPDF_PRO_KEY",
                self.edit_pymupdf_pro_key.trim().to_string(),
            ),
            (
                "MINDEE_API_KEY",
                self.edit_mindee_api_key.trim().to_string(),
            ),
            (
                "LLAMAPARSE_API_KEY",
                self.edit_llamaparse_api_key.trim().to_string(),
            ),
            (
                "PDFREST_API_KEY",
                self.edit_pdfrest_api_key.trim().to_string(),
            ),
            (
                "APPLITOOLS_API_KEY",
                self.edit_applitools_api_key.trim().to_string(),
            ),"""
gui = gui.replace(pairs_old, pairs_new)

# Fix 4: Fix suffixes in api_availability fields
gui = gui.replace('self.api_availability.document_ai_configured', 'self.api_availability.document_ai')
gui = gui.replace('self.api_availability.gemini_configured', 'self.api_availability.gemini_api_key')
gui = gui.replace('self.api_availability.pro_editing_available', 'self.api_availability.pymupdf_pro')
gui = gui.replace('self.api_availability.mindee_configured', 'self.api_availability.mindee')
gui = gui.replace('self.api_availability.llamaparse_configured', 'self.api_availability.llamaparse')
gui = gui.replace('self.api_availability.pdfrest_configured', 'self.api_availability.pdfrest')
gui = gui.replace('self.api_availability.applitools_configured', 'self.api_availability.applitools')
gui = gui.replace('self.api_availability.offline_parser_available', 'self.api_availability.offline_parser')

with open('src/app/gui.rs', 'w', encoding='utf-8') as f:
    f.write(gui)
print('Done patching gui.rs')
