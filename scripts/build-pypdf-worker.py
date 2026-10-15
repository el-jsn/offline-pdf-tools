"""Bundle the local repair worker using an already provisioned Python 3.12 venv.

No downloads. Run with the venv's Python after installing the pinned build
requirements. End users need neither Python nor pip. macOS and Windows only.
"""
import hashlib
import importlib.metadata as metadata
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / 'src-tauri/engine'
manifest_path = ENGINE / 'manifest.json'
manifest = json.loads(manifest_path.read_text())
if sys.platform not in ('darwin', 'win32') or manifest['platform'] != sys.platform or sys.version_info[:2] != (3, 12):
    raise RuntimeError('Use Python 3.12 and the matching native qpdf bundle.')
for name, version in [('pypdf', '6.20.1'), ('pyinstaller', '6.22.3')]:
    if metadata.version(name) != version:
        raise RuntimeError('Install scripts/pypdf-build-requirements.txt into a clean venv.')
name = 'pypdf-repair'
with tempfile.TemporaryDirectory(prefix='pypdf-build-') as work:
    work = Path(work)
    subprocess.run([sys.executable, '-m', 'PyInstaller', '--noconfirm', '--clean', '--onedir',
                    '--name', name, '--distpath', str(work / 'dist'), '--workpath', str(work / 'build'),
                    '--specpath', str(work), '--exclude-module', 'tkinter',
                    str(ROOT / 'scripts/pypdf-repair.py')], check=True,
                   env={**os.environ, 'PYINSTALLER_CONFIG_DIR': str(work / 'cache')})
    bundle = work / 'dist' / name
    notices = bundle / 'licenses'
    notices.mkdir()
    # Include the Python license and notices from every build distribution as
    # well as bundled native-library notices from the Python distribution.
    python_license = Path(sys.base_prefix) / 'lib/python3.12/LICENSE.txt'
    if not python_license.exists():
        python_license = Path(sys.base_prefix) / 'LICENSE.txt'
    shutil.copy2(python_license, notices / 'Python-LICENSE.txt')
    for distribution in metadata.distributions():
        for entry in distribution.files or []:
            if any(word in str(entry).lower() for word in ('license', 'copying', 'notice')):
                source = Path(distribution.locate_file(entry))
                if source.is_file():
                    destination = notices / distribution.metadata['Name'] / str(entry)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, destination)
    # Standalone Python runtimes carry native dependency notices in this tree.
    native_notices = Path(sys.base_prefix) / 'licenses'
    if native_notices.is_dir():
        shutil.copytree(native_notices, notices / 'python-native')
    else:
        shutil.copytree(ROOT / 'scripts/notices/python-standalone', notices / 'python-native')
    exe = bundle / ('pypdf-repair.exe' if sys.platform == 'win32' else name)
    version = subprocess.check_output([str(exe), '--version'], text=True, timeout=30).strip()
    if version != 'pypdf-repair 6.20.1':
        raise RuntimeError('Repair worker self-check failed.')
    target = ENGINE / name
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(bundle, target, symlinks=True)
for key in list(manifest['files']):
    if key.startswith(name + '/'):
        del manifest['files'][key]
for path in target.rglob('*'):
    if path.is_file():
        manifest['files'][path.relative_to(ENGINE).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
manifest['pypdf'] = {'version': '6.20.1', 'python': platform.python_version(),
                     'source': 'https://pypi.org/project/pypdf/6.20.1/',
                     'worker_sha256': hashlib.sha256((ROOT / 'scripts/pypdf-repair.py').read_bytes()).hexdigest()}
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
print('Bundled pypdf 6.20.1 repair worker and notices staged.')
