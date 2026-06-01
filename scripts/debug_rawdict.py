"""Inspect rawdict structure."""
import sys, json
sys.path.insert(0, 'python')
import pymupdf
import pymupdf.pro
import pymupdf_pro_integration as m

pymupdf.pro.unlock(m.PYMUPDF_PRO_KEY)
doc = pymupdf.open('AU Bank Statements/IA_Bank_Statement_202602.pdf')
page = doc[0]

raw = page.get_text('rawdict')
# Find first non-empty block.
for block in raw.get('blocks', []):
    for line in block.get('lines', []):
        for span in line.get('spans', []):
            text = span.get('text', '')
            chars = span.get('chars', [])
            if len(chars) > 3:
                print(f'span text: {text!r}')
                print(f'span bbox: {span.get("bbox")}')
                print(f'#chars: {len(chars)}')
                print(f'first 3 chars: {chars[:3]}')
                doc.close()
                sys.exit(0)

print('no qualifying span found')
doc.close()
