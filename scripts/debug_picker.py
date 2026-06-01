import os, sys, json, shutil, tempfile
sys.path.insert(0, 'python')

cache_dir = tempfile.mkdtemp(prefix='picker_')
os.environ['FONT_CACHE_DIR'] = cache_dir
shutil.copy(r'C:\Windows\Fonts\arial.ttf', os.path.join(cache_dir, 'arial.ttf'))
manifest = {'arial': 'arial.ttf'}
with open(os.path.join(cache_dir, 'manifest.json'), 'w') as f:
    json.dump(manifest, f)

import font_replicator
print('cache dir:', font_replicator._cache_dir())
print('donor for arial:', font_replicator._pick_local_donor('arial'))
print('donor for ARIAL:', font_replicator._pick_local_donor('ARIAL'))
