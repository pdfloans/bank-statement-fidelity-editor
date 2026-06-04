import pymupdf
import sys
import os

def append_audit(pdf_path, json_path):
    if not os.path.exists(pdf_path) or not os.path.exists(json_path):
        sys.exit(1)
        
    doc = pymupdf.open(pdf_path)
    page = doc.new_page(-1)
    
    with open(json_path, 'r', encoding='utf-8') as f:
        audit_text = f.read()
        
    # Split the JSON text into lines so it fits on the page (rudimentary wrapping)
    # PyMuPDF's insert_text handles newlines, but we can also use insert_textbox for auto-wrapping
    rect = pymupdf.Rect(50, 50, page.rect.width - 50, page.rect.height - 50)
    page.insert_textbox(rect, "=== AUTOMATED AUDIT REPORT ===\n\n" + audit_text, fontsize=10, fontname="helv")
    
    # Save the document in place
    doc.saveIncr()
    doc.close()

if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(1)
    append_audit(sys.argv[1], sys.argv[2])
