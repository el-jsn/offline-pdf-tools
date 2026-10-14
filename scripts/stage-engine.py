"""Stage a trusted native qpdf runtime; never downloads or compiles anything.

macOS: python3 scripts/stage-engine.py /absolute/path/to/qpdf --notices /path/to/notices
Windows: py scripts/stage-engine.py C:\qpdf\bin\qpdf.exe --notices C:\qpdf\notices
Run on the same OS and architecture as the installer being prepared.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
DESTINATION = ROOT / 'src-tauri' / 'engine'


def run(*arguments):
    return subprocess.check_output(arguments, text=True, stderr=subprocess.STDOUT).strip()


def rpaths(binary):
    paths = []
    lines = run('/usr/bin/otool', '-l', str(binary)).splitlines()
    for index, line in enumerate(lines):
        if line.strip() == 'cmd LC_RPATH':
            for candidate in lines[index + 1:index + 4]:
                if candidate.strip().startswith('path '):
                    paths.append(candidate.strip()[5:].rsplit(' (offset ', 1)[0])
    return paths


def dependencies(binary):
    lines = run('/usr/bin/otool', '-L', str(binary)).splitlines()[1:]
    # otool lists a dylib's own install name before its dependencies.
    if binary.suffix == '.dylib':
        lines = lines[1:]
    return [line.strip().split(' (compatibility version', 1)[0] for line in lines]


def expand(path, binary, executable):
    return path.replace('@loader_path', str(binary.parent)).replace('@executable_path', str(executable.parent))


def resolve_dependency(value, binary, executable):
    if value.startswith(('/usr/lib/', '/System/Library/')):
        return None
    if value.startswith('@rpath/'):
        suffix = value[len('@rpath/'):]
        candidates = [Path(expand(path, binary, executable)) / suffix for path in rpaths(binary)]
        candidates += [Path(expand(path, executable, executable)) / suffix for path in rpaths(executable)]
    else:
        candidates = [Path(expand(value, binary, executable))]
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    raise RuntimeError(f'Cannot resolve required library: {value}')


def stage_mac(executable, staging):
    files = {executable: 'qpdf'}
    links = {}
    queue = [executable]
    while queue:
        binary = queue.pop()
        links[binary] = []
        for value in dependencies(binary):
            dependency = resolve_dependency(value, binary, executable)
            if dependency is None:
                continue
            if dependency not in files:
                if dependency.name in files.values():
                    raise RuntimeError('Two different libraries have the same name; resolve the runtime manually.')
                files[dependency] = dependency.name
                queue.append(dependency)
            links[binary].append((value, dependency))
    for source, filename in files.items():
        target = staging / filename
        shutil.copy2(source, target)
        target.chmod(target.stat().st_mode | 0o200)
        changes = []
        if target.suffix == '.dylib':
            changes.extend(['-id', f'@loader_path/{filename}'])
        for old, dependency in links[source]:
            changes.extend(['-change', old, f'@loader_path/{files[dependency]}'])
        if changes:
            run('/usr/bin/install_name_tool', *changes, str(target))
        # Modifying Mach-O load commands invalidates existing signatures.
        run('/usr/bin/codesign', '--force', '--sign', '-', str(target))
    run(str(staging / 'qpdf'), '--version')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('executable', type=Path)
    parser.add_argument('--notices', required=True, type=Path, help='Directory containing the exact runtime licenses and third-party notices')
    args = parser.parse_args()
    executable = args.executable.expanduser().resolve(strict=True)
    if not executable.is_file() or not args.notices.is_dir():
        raise RuntimeError('Provide a qpdf executable and a directory of matching license notices.')
    version = run(str(executable), '--version')
    match = re.search(r'qpdf version (\d+)\.(\d+)', version)
    if not match or tuple(map(int, match.groups())) < (12, 1):
        raise RuntimeError('qpdf 12.1 or newer is required.')
    DESTINATION.mkdir(parents=True, exist_ok=True)
    existing = [path for path in DESTINATION.iterdir() if path.name != 'README.md']
    if existing:
        raise RuntimeError('Engine folder already contains a runtime. Review and remove it yourself before staging a replacement.')
    with tempfile.TemporaryDirectory(prefix='pdf-tools-engine-', dir=ROOT / 'src-tauri') as directory:
        staging = Path(directory)
        if sys.platform == 'darwin':
            stage_mac(executable, staging)
        elif sys.platform == 'win32':
            shutil.copy2(executable, staging / 'qpdf.exe')
            for library in executable.parent.glob('*.dll'):
                shutil.copy2(library, staging / library.name)
            for library in executable.parent.glob('*.DLL'):
                if not (staging / library.name).exists():
                    shutil.copy2(library, staging / library.name)
            run(str(staging / 'qpdf.exe'), '--version')
        else:
            raise RuntimeError('This staging script currently supports macOS and Windows only.')
        shutil.copytree(args.notices, staging / 'licenses')
        manifest = {'platform': sys.platform, 'version': version.splitlines()[0], 'files': {}}
        for path in sorted(staging.rglob('*')):
            if path.is_file():
                manifest['files'][path.relative_to(staging).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
        (staging / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        for path in staging.iterdir():
            shutil.move(str(path), str(DESTINATION / path.name))
    print(f'Runtime staged in {DESTINATION}. No installer was built.')


if __name__ == '__main__':
    main()
