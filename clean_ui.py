import os
import re

def clean_file(path):
    with open(path, 'rb') as f:
        text = f.read().decode('utf-8', errors='replace')
    
    original = text
    
    replacements = [
        (r'A\ufffd,\ufffd\?\?', '-'),
        (r'\ufffd\?"', '-'),
        (r'A\ufffd,\ufffdA\ufffd', '...'),
        (r'A\ufffd\?\ufffd\?T', '->'),
        (r'A\ufffd\?\?\ufffd,\ufffd', '--'),
        (r'â€”', '-'),
        (r'@@@', '-'),
        (r'A\ufffd,\ufffd', '-'),
        (r'A\ufffd\?\?', '-'),
        (r'\ufffd', ''),
        (r'——', '--'),
        (r'“', '"'),
        (r'”', '"'),
        (r'‘', "'"),
        (r'’', "'"),
        (r'…', '...'),
        (r'—', '-'),
        (r'–', '-'),
        (r'═══════════════════════════════════════════════', '='*47),
        (r'╔════════════════════════════════════════════════════════════╗', '/' + '='*60 + '\\\\'),
        (r'║', '|'),
        (r'╚════════════════════════════════════════════════════════════╝', '\\\\' + '='*60 + '/'),
        (r'✅', '[OK]'),
        (r'⚠️', '[!]'),
        (r'⚙️', '[*]'),
    ]
    
    for pat, rep in replacements:
        text = re.sub(pat, rep, text)
        
    text = re.sub(r'[^\x00-\x7F]', '', text)
    
    if text != original:
        with open(path, 'w', encoding='utf-8') as f:
            f.write(text)
        print(f"Cleaned {path}")

for root, _, files in os.walk('src'):
    for file in files:
        if file.endswith('.rs'):
            clean_file(os.path.join(root, file))
