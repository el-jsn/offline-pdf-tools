"""Stage compress-pdf 0.1.0 and its complete third-party notices (no downloads)."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('executable', type=Path)
parser.add_argument('--notices', type=Path, required=True)
args = parser.parse_args()
engine = Path(__file__).resolve().parent.parent / 'src-tauri' / 'engine'
manifest_path = engine / 'manifest.json'
manifest = json.loads(manifest_path.read_text())
if sys.platform not in ('darwin', 'win32') or manifest['platform'] != sys.platform:
    raise RuntimeError('Stage a matching Mac or Windows qpdf runtime first.')
if not args.executable.is_file() or not args.notices.is_dir():
    raise RuntimeError('Supply the native executable and its complete license notices.')
notices = [p for p in args.notices.rglob('*') if p.is_file()]
if not notices or not any('license' in p.name.lower() for p in notices):
    raise RuntimeError('Include the compress-pdf license and dependency notices.')
version = subprocess.check_output([str(args.executable.resolve()), '--version'], timeout=10, text=True).strip()
if version != 'compress-pdf 0.1.0':
    raise RuntimeError('Use the pinned compress-pdf 0.1.0 executable.')
if sys.platform == 'darwin':
    dependencies = subprocess.check_output(['/usr/bin/otool', '-L', str(args.executable)], text=True).splitlines()[1:]
    if any(not line.strip().startswith(('/usr/lib/', '/System/Library/')) for line in dependencies):
        raise RuntimeError('The Mac compressor must depend only on system libraries.')
name = 'compress-pdf.exe' if sys.platform == 'win32' else 'compress-pdf'
with tempfile.TemporaryDirectory(prefix='compressor-staging-', dir=engine) as work:
    staged = Path(work) / name
    shutil.copy2(args.executable, staged)
    if sys.platform == 'darwin':
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(staged)], check=True)
    shutil.copy2(staged, engine / name)
notice_root = engine / 'licenses' / 'compress-pdf'
if notice_root.exists():
    shutil.rmtree(notice_root)
shutil.copytree(args.notices, notice_root)
for key in list(manifest['files']):
    if key.startswith('licenses/compress-pdf/'):
        del manifest['files'][key]
for path in [engine / name, *notice_root.rglob('*')]:
    if path.is_file():
        manifest['files'][path.relative_to(engine).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
manifest['compressor'] = {'version': '0.1.0', 'source': 'https://crates.io/crates/compress-pdf/0.1.0'}
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
print('Bundled compress-pdf 0.1.0 and license notices staged.')
