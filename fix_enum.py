import sys

def main():
    with open("src/app/runtime.rs", "r") as f:
        code = f.read()

    code = code.replace("DocumentParserMode::Mindee", "DocumentParserMode::MindeeFinDoc")
    code = code.replace("DocumentParserMode::OfflineParser", "DocumentParserMode::PyMuPdfBuiltin")
    
    with open("src/app/runtime.rs", "w") as f:
        f.write(code)

if __name__ == "__main__":
    main()
