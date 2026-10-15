"""Focused repair/compression regression using generated form data only."""
import os
from pathlib import Path
import subprocess
import tempfile

from PIL import ImageChops, ImageStat
from pypdf import PdfReader, PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, NameObject, NumberObject, TextStringObject
import pypdfium2 as pdfium
from reportlab.pdfgen import canvas

ROOT = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='pdf-synthetic-duplicate-widgets-', dir='/private/tmp') as work:
    work = Path(work)
    c = canvas.Canvas(str(work / 'base.pdf'))
    c.drawString(40, 800, 'Synthetic duplicate form widget regression')
    c.acroForm.textfield(name='answer', x=40, y=700, width=300, height=35, value='Saved synthetic answer')
    c.acroForm.checkbox(name='consent', x=40, y=640, checked=True)
    c.showPage()
    c.save()
    writer = PdfWriter(clone_from=work / 'base.pdf')
    page = writer.pages[0]
    empty_signature = DictionaryObject({
        NameObject('/Type'): NameObject('/Annot'), NameObject('/Subtype'): NameObject('/Widget'),
        NameObject('/FT'): NameObject('/Sig'), NameObject('/T'): TextStringObject('EmptySyntheticSignature'),
        NameObject('/Rect'): ArrayObject([NumberObject(n) for n in [40, 550, 300, 600]]),
    })
    signature_ref = writer._add_object(empty_signature)
    writer.root_object['/AcroForm']['/Fields'].append(signature_ref)
    page['/Annots'].append(signature_ref)
    writer.write(work / 'expected.pdf')
    # The widget dictionaries and appearance references remain identical. Only
    # the object references in /Annots differ from the canonical /Fields tree.
    duplicates = ArrayObject()
    for ref in page['/Annots']:
        duplicates.append(writer._add_object(DictionaryObject(dict(ref.get_object()))))
    page[NameObject('/Annots')] = duplicates
    writer.write(work / 'duplicate.pdf')
    result = subprocess.run([str(ROOT / 'src-tauri/engine/qpdf'), '--check', str(work / 'duplicate.pdf')], capture_output=True)
    assert result.returncode == 3
    env = dict(os.environ, PDF_TOOLS_FORM_REPAIR_INPUT=str(work / 'duplicate.pdf'), PDF_TOOLS_SYNTHETIC_REPAIR_OUTPUT=str(work))
    subprocess.run(['cargo', 'test', '--manifest-path', str(ROOT / 'src-tauri/Cargo.toml'), '--bin', 'offline-pdf-tools', 'forms::tests::supplied_duplicate_widget_repair_and_compression', '--', '--exact', '--ignored'], env=env, check=True)
    expected = pdfium.PdfDocument(str(work / 'expected.pdf'))
    expected.init_forms()
    expected_image = expected[0].render(scale=1.5, may_draw_forms=True).to_pil().convert('RGB')
    expected_values = {key: field.get('/V') for key, field in PdfReader(work / 'expected.pdf').get_fields().items()}
    for name in ['repaired.pdf', 'balanced.pdf', 'smaller.pdf']:
        fields = PdfReader(work / name).get_fields()
        assert {key: field.get('/V') for key, field in fields.items()} == expected_values
        actual = pdfium.PdfDocument(str(work / name))
        actual.init_forms()
        actual_image = actual[0].render(scale=1.5, may_draw_forms=True).to_pil().convert('RGB')
        delta = max(ImageStat.Stat(ImageChops.difference(expected_image, actual_image)).mean)
        assert delta < 0.1, f'{name}: form appearance changed'
        print(f'{name}: saved values preserved; mean channel difference {delta:.6f}')
        actual.close()
    expected.close()
print('Synthetic form repair and compression checks passed; temporary files removed.')
