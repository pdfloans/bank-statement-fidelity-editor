import sys

def main():
    with open("src/app/runtime.rs", "r") as f:
        code = f.read()
    
    # Change macro to return the next parser (which will be an Option)
    code = code.replace("Some($next_parser)", "$next_parser")
    
    # Update the usage
    code = code.replace("DocumentParserMode::MindeeFinDoc", "Some(DocumentParserMode::MindeeFinDoc)")
    code = code.replace("DocumentParserMode::DocumentAi", "Some(DocumentParserMode::DocumentAi)")
    
    # For PyMuPdfBuiltin, we have it in a few places
    # "Mindee parse failed" -> Some(PyMuPdfBuiltin)
    code = code.replace("format!(\"Mindee parse failed: {e}\"), DocumentParserMode::PyMuPdfBuiltin", "format!(\"Mindee parse failed: {e}\"), Some(DocumentParserMode::PyMuPdfBuiltin)")
    code = code.replace("format!(\"Mindee not configured: {e}\"), DocumentParserMode::PyMuPdfBuiltin", "format!(\"Mindee not configured: {e}\"), Some(DocumentParserMode::PyMuPdfBuiltin)")
    
    # "Offline parser failed" -> None
    code = code.replace("format!(\"Offline parser failed: {e}\"), DocumentParserMode::PyMuPdfBuiltin", "format!(\"Offline parser failed: {e}\"), None")
    code = code.replace("format!(\"Offline parser panicked: {e}\"), DocumentParserMode::PyMuPdfBuiltin", "format!(\"Offline parser panicked: {e}\"), None")
    
    with open("src/app/runtime.rs", "w") as f:
        f.write(code)

if __name__ == "__main__":
    main()
