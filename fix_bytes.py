import os

replacements = [
    (b'\xc3\xa2\xe2\x82\xac\xc2\xa6', b'...'),    # ellipsis â€¦
    (b'\xc3\xa2\xe2\x82\xac\xe2\x80\x9d', b'-'),   # em dash â€” (variant)
    (b'\xc3\xa2\xe2\x80\x94', b'-'),               # another em dash variant
    (b'\xe2\x80\x94', b'-'),                       # literal em dash
    (b'\xe2\x80\xa6', b'...'),                     # literal ellipsis
    (b'\xe2\x86\x92', b'->'),                      # literal right arrow
    (b'\xc3\xa2\xe2\x80\xa0\xe2\x80\x99', b'->'),  # right arrow corrupted
]

for root, _, files in os.walk('src'):
    for file in files:
        if file.endswith('.rs'):
            path = os.path.join(root, file)
            with open(path, 'rb') as f:
                content = f.read()
                
            original = content
            for old, new in replacements:
                content = content.replace(old, new)
                
            if content != original:
                with open(path, 'wb') as f:
                    f.write(content)
                print(f"Fixed {path}")
