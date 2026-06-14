import re

content = open('Cargo.toml', 'r', encoding='utf-8').read()

content = content.replace('[dependencies]', '''[dependencies]
pyo3 = { version = "0.20.0", features = ["auto-initialize"] }
pdfium-render = "0.8.21"
mupdf = "0.7.0"
''')

open('Cargo.toml', 'w', encoding='utf-8').write(content)
print("Updated Cargo.toml")
