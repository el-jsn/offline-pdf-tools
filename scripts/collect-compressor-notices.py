"""Collect notices from the cached sources of a locked compress-pdf native build.

Requires Python 3.11+. Does not download or execute package code. The source and
registry directories must belong to the exact cargo install --locked build.
"""
import argparse
import json
from pathlib import Path
import shutil
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('source', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
source = args.source.resolve()
package = tomllib.loads((source / 'Cargo.toml').read_text())['package']
if (package['name'], package['version']) != ('compress-pdf', '0.1.0'):
    raise RuntimeError('Supply the cached compress-pdf 0.1.0 source.')
args.output.mkdir(parents=True, exist_ok=False)
shutil.copy2(source / 'LICENSE', args.output / 'LICENSE')
shutil.copy2(source / 'Cargo.lock', args.output / 'Cargo.lock')
inventory = []
# These subcrates omit the repository-wide notices from their crate archive.
shared = {'alloc-stdlib': 'alloc-no-stdlib-2.0.4', 'defmt-parser': 'defmt-1.1.1'}
for dependency in tomllib.loads((source / 'Cargo.lock').read_text())['package']:
    name, version = dependency['name'], dependency['version']
    directory = source.parent / f'{name}-{version}'
    if not directory.is_dir():
        continue  # Cargo only fetches dependencies needed for this target/build.
    metadata = tomllib.loads((directory / 'Cargo.toml').read_text())['package']
    files = [p for p in directory.rglob('*') if p.is_file() and not p.is_symlink()
             and any(word in p.name.lower() for word in ('license', 'copying', 'notice', 'copyright'))]
    notice_source = directory
    if not files and name in shared:
        notice_source = source.parent / shared[name]
        files = [p for p in notice_source.glob('LICENSE*') if p.is_file()]
    if not files:
        raise RuntimeError(f'License notices missing for {name} {version}; supply the upstream notices.')
    destination = args.output / 'dependencies' / f'{name}-{version}'
    destination.mkdir(parents=True)
    shutil.copy2(directory / 'Cargo.toml', destination / 'Cargo.toml')
    for path in files:
        target = destination / path.relative_to(notice_source)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
    inventory.append({'name': name, 'version': version, 'license': metadata.get('license'),
                      'repository': metadata.get('repository'), 'noticeFiles': len(files)})
(args.output / 'DEPENDENCIES.json').write_text(json.dumps(inventory, indent=2) + '\n')
print(f'Collected notices for {len(inventory)} cached packages, including native-library notices.')
