"""Stage a matching PDFium runtime and its complete build license notices."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parent.parent
engine = root / 'src-tauri' / 'engine'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('library', type=Path)
parser.add_argument('--notices', type=Path, required=True)
parser.add_argument('--version', required=True)
parser.add_argument('--source', required=True, help='Pinned upstream release URL or build provenance')
args = parser.parse_args()
manifest_path = engine / 'manifest.json'
manifest = json.loads(manifest_path.read_text())
if manifest['platform'] != sys.platform: raise RuntimeError('Stage qpdf for this target first.')
if not args.library.is_file() or not args.notices.is_dir() or not any(args.notices.rglob('*')): raise RuntimeError('Supply a PDFium library and its complete license notices.')
name = 'pdfium.dll' if sys.platform == 'win32' else 'libpdfium.dylib'
if sys.platform not in ['darwin', 'win32']: raise RuntimeError('Mac and Windows are supported.')
with tempfile.TemporaryDirectory(prefix='pdfium-staging-', dir=engine) as work:
    work = Path(work)
    shutil.copy2(args.library, work / name)
    source_hash = hashlib.sha256((work / name).read_bytes()).hexdigest()
    if sys.platform == 'darwin': subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(work / name)], check=True)
    shutil.copytree(args.notices, work / 'licenses')
    shutil.copy2(work / name, engine / name)
    shutil.copytree(work / 'licenses', engine / 'licenses' / 'pdfium', dirs_exist_ok=True)
manifest['pdfium'] = {'version': args.version, 'source': args.source, 'upstreamSha256': source_hash}
for path in [engine / name, *(engine / 'licenses' / 'pdfium').rglob('*')]:
    if path.is_file(): manifest['files'][path.relative_to(engine).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
print('PDFium runtime staged. The build verifies runtime loading in the isolated worker.')
