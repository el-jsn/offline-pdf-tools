"""Offline, deliberately narrow AcroForm repair worker. Never emit PDF data."""
import hashlib
import logging
from pathlib import Path
import shutil
import sys
import tempfile
import warnings

import pypdf
from pypdf import PdfReader, PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, StreamObject

VERSION = '6.20.1'
logging.disable(logging.CRITICAL)
warnings.filterwarnings('ignore')


def fingerprint(value, active=None, budget=None):
    """Compare decoded objects across rewrites, independently of object numbers."""
    active = set() if active is None else active
    budget = [250000] if budget is None else budget
    budget[0] -= 1
    if budget[0] < 0:
        raise ValueError('complexity')
    if isinstance(value, IndirectObject):
        value = value.get_object()
    if isinstance(value, (DictionaryObject, ArrayObject)):
        identity = id(value)
        if identity in active:
            raise ValueError('unsupported cycle')
        active.add(identity)
        if isinstance(value, DictionaryObject):
            # These stream keys describe encoding, not decoded content.
            ignored = {'/Length', '/Filter', '/DecodeParms'} if isinstance(value, StreamObject) else set()
            result = tuple((str(k), fingerprint(v, active, budget)) for k, v in sorted(value.items()) if k not in ignored)
            if isinstance(value, StreamObject):
                result = (result, hashlib.sha256(value.get_data()).digest())
        else:
            result = tuple(fingerprint(v, active, budget) for v in value)
        active.remove(identity)
        return result
    # Serialize primitives with PDF types intact (names, strings, booleans, null).
    from io import BytesIO
    result = BytesIO()
    value.write_to_stream(result)
    return result.getvalue()


def widget_fingerprint(widget):
    return fingerprint(DictionaryObject({k: v for k, v in widget.items() if k != '/P'}))


def inspect(reader):
    root = reader.root_object
    form = root.get('/AcroForm')
    if form is None or '/Perms' in root:
        raise ValueError('unsupported form')
    form = form.get_object()
    if '/XFA' in form:
        raise ValueError('unsupported form')
    canonical = {}
    for reference in form.get('/Fields', []):
        field = reference.get_object()
        name = field.get('/T')
        if (field.get('/Subtype') != '/Widget' or '/Parent' in field or '/Kids' in field
                or not name or name in canonical):
            raise ValueError('unsupported or ambiguous field')
        if field.get('/FT') == '/Sig' and field.get('/V') is not None:
            raise ValueError('signed')
        canonical[name] = widget_fingerprint(field)
    pages = []
    placed = {}
    for page in reader.pages:
        widgets = []
        annotations = []
        for reference in page.get('/Annots', []):
            annotation = reference.get_object()
            if annotation.get('/Subtype') != '/Widget':
                annotations.append(widget_fingerprint(annotation))
                continue
            name = annotation.get('/T')
            if (not name or '/Parent' in annotation or '/Kids' in annotation
                    or annotation.get('/FT') not in ('/Tx', '/Btn', '/Ch', '/Sig') or name in placed):
                raise ValueError('unsupported or ambiguous widget')
            if annotation.get('/FT') == '/Sig' and annotation.get('/V') is not None:
                raise ValueError('signed')
            data = widget_fingerprint(annotation)
            if name in canonical and canonical[name] != data:
                raise ValueError('conflicting widget')
            placed[name] = data
            widgets.append((name, data))
        contents = page.get_contents()
        pages.append((fingerprint(page.mediabox), fingerprint(page.cropbox), page.rotation,
                      hashlib.sha256(contents.get_data() if contents else b'').digest(),
                      fingerprint(page.get('/Resources', DictionaryObject())), widgets, annotations))
    if not placed or any(name not in placed for name in canonical):
        raise ValueError('unplaced field')
    return pages, placed, canonical


def repair(source, output):
    reader = PdfReader(source)
    if reader.is_encrypted:
        raise ValueError('encrypted')
    before, placed, _ = inspect(reader)
    writer = PdfWriter(clone_from=reader)
    writer.compress_identical_objects(remove_duplicates=True, remove_unreferenced=True)
    writer.reattach_fields()
    writer.compress_identical_objects(remove_duplicates=True, remove_unreferenced=True)
    with tempfile.TemporaryDirectory(prefix='pypdf-', dir=Path(output).parent) as work:
        candidate = Path(work) / 'candidate.pdf'
        writer.write(candidate)
        after, after_placed, canonical = inspect(PdfReader(candidate))
        if before != after or placed != after_placed or canonical != placed:
            raise ValueError('preservation')
        with candidate.open('rb') as source_handle, Path(output).open('xb') as destination:
            shutil.copyfileobj(source_handle, destination)


def main():
    if pypdf.__version__ != VERSION:
        return 2
    if sys.argv[1:] == ['--version']:
        print('pypdf-repair ' + VERSION)
        return 0
    if len(sys.argv) != 3:
        return 2
    try:
        repair(sys.argv[1], sys.argv[2])
        return 0
    except Exception:
        # No traceback, exception message, filenames, field names or values.
        return 2


if __name__ == '__main__':
    sys.exit(main())
