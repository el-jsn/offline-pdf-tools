"""Focused frozen-worker and Rust integration checks; generated PDFs only."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile

from PIL import ImageChops
from pypdf import PdfReader, PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, NameObject, TextStringObject
import pypdfium2 as pdfium
from reportlab.pdfgen import canvas

ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / 'src-tauri/engine'
WORKER = ENGINE / 'pypdf-repair' / ('pypdf-repair.exe' if os.name == 'nt' else 'pypdf-repair')
QPDF = ENGINE / ('qpdf.exe' if os.name == 'nt' else 'qpdf')


def render(path):
    document = pdfium.PdfDocument(path)
    document.init_forms()
    image = document[0].render(scale=1.5, may_draw_forms=True).to_pil().convert('RGB')
    document.close()
    return image


with tempfile.TemporaryDirectory(prefix='pdf-synthetic-pypdf-') as work:
    work = Path(work)
    clean = work / 'clean.pdf'
    c = canvas.Canvas(str(clean))
    c.drawString(40, 800, 'Generated AcroForm regression')
    c.acroForm.textfield(name='answer', x=40, y=700, width=300, height=35, value='Synthetic answer')
    c.acroForm.checkbox(name='consent', x=40, y=640, checked=True)
    c.showPage()
    c.save()
    expected_image = render(clean)
    expected_values = {key: field.get('/V') for key, field in PdfReader(clean).get_fields().items()}
    cases = ['missing_fields', 'duplicate', 'conflicting_value', 'conflicting_appearance',
             'same_name', 'nested', 'signed', 'xfa']
    for case in cases:
        writer = PdfWriter(clone_from=clean)
        form = writer.root_object['/AcroForm']
        page = writer.pages[0]
        if case == 'missing_fields':
            form[NameObject('/Fields')] = ArrayObject()
        elif case in ('duplicate', 'conflicting_value', 'conflicting_appearance'):
            copies = ArrayObject(writer._add_object(DictionaryObject(dict(ref.get_object()))) for ref in page['/Annots'])
            page[NameObject('/Annots')] = copies
            if case == 'conflicting_value':
                copies[0].get_object()[NameObject('/V')] = TextStringObject('Conflicting synthetic value')
            if case == 'conflicting_appearance':
                copies[0].get_object()[NameObject('/AP')] = DictionaryObject()
        elif case == 'same_name':
            page['/Annots'][1].get_object()[NameObject('/T')] = TextStringObject('answer')
        elif case == 'nested':
            page['/Annots'][0].get_object()[NameObject('/Kids')] = ArrayObject()
        elif case == 'signed':
            widget = page['/Annots'][0].get_object()
            widget[NameObject('/FT')] = NameObject('/Sig')
            widget[NameObject('/V')] = DictionaryObject()
        elif case == 'xfa':
            form[NameObject('/XFA')] = TextStringObject('Generated unsupported XFA')
        source = work / (case + '.pdf')
        writer.write(source)
        before = hashlib.sha256(source.read_bytes()).digest()
        output = work / (case + '-repaired.pdf')
        result = subprocess.run([str(WORKER), str(source), str(output)], capture_output=True, timeout=30,
                                env={**os.environ, 'PATH': '', 'PYTHONPATH': '', 'PYTHONHOME': ''})
        assert result.stdout == b'' and result.stderr == b'', 'Worker emitted document diagnostics'
        if case in ('missing_fields', 'duplicate'):
            assert result.returncode == 0, case
            assert subprocess.run([str(QPDF), '--check', str(output)], capture_output=True).returncode == 0
            assert {key: field.get('/V') for key, field in PdfReader(output).get_fields().items()} == expected_values
            assert ImageChops.difference(expected_image, render(output)).getbbox() is None, 'Appearance changed'
            output_hash = hashlib.sha256(output.read_bytes()).digest()
            assert subprocess.run([str(WORKER), str(source), str(output)], capture_output=True).returncode != 0
            assert hashlib.sha256(output.read_bytes()).digest() == output_hash, 'Existing output overwritten'
        else:
            assert result.returncode != 0 and not output.exists(), case
        assert hashlib.sha256(source.read_bytes()).digest() == before, 'Original changed'
        print(case + ': passed')
    env = {**os.environ, 'PDF_TOOLS_PYPDF_INPUT': str(work / 'missing_fields.pdf'), 'PDF_TOOLS_PYPDF_EXPECTED': str(clean)}
    subprocess.run(['cargo', 'test', '--manifest-path', str(ROOT / 'src-tauri/Cargo.toml'), '--bin',
                    'offline-pdf-tools', 'forms::tests::supplied_pypdf_missing_fields', '--', '--exact', '--ignored'],
                   env=env, check=True)
print('pypdf repair checks passed; generated PDFs removed.')
