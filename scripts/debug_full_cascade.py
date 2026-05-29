import os, sys, tempfile, shutil, json
sys.path.insert(0, 'python')

import pymupdf
import pymupdf.pro
from fontTools.ttLib import TTFont
from fontTools.subset import Subsetter
import pymupdf_pro_integration as m
import font_replicator
pymupdf.pro.unlock(m.PYMUPDF_PRO_KEY)

tmp = tempfile.mkdtemp(prefix='full_')
donor = r'C:\Windows\Fonts\arial.ttf'

# Set up cache.
cache = os.path.join(tmp, 'cache')
os.makedirs(cache)
shutil.copy(donor, os.path.join(cache, 'arial.ttf'))
with open(os.path.join(cache, 'manifest.json'), 'w') as f:
    json.dump({'arial': 'arial.ttf'}, f)
os.environ['FONT_CACHE_DIR'] = cache

# Build subset PDF.
sub = TTFont(donor)
ss = Subsetter()
ss.populate(text='0123e')
ss.subset(sub)
subset_path = os.path.join(tmp, 'subset.ttf')
sub.save(subset_path)

doc = pymupdf.open()
page = doc.new_page(width=400, height=100)
page.insert_font(fontname='testsubset', fontfile=subset_path)
page.insert_text(pymupdf.Point(20, 50), '0123e', fontname='testsubset', fontsize=24)
pdf_path = os.path.join(tmp, 'doc.pdf')
doc.save(pdf_path)
doc.close()

# Run cascade.
out = os.path.join(tmp, 'cascade')
os.makedirs(out, exist_ok=True)
result = font_replicator.replicate_font_for_chars(
    pdf_path=pdf_path,
    font_name='arial',
    missing_chars=['4', '5', 'A'],
    output_dir=out,
)
print(json.dumps(result, indent=2, default=str))
print()
print('artefacts:')
for f in os.listdir(out):
    full = os.path.join(out, f)
    print(f'  {f}  ({os.path.getsize(full)} bytes)')
