"""Focused bundled-compressor check using generated, non-customer PDFs only.

Requires reportlab, pypdf, Pillow and pypdfium2. Use --fixture DIRECTORY to retain
only the generated input for the explicit Rust integration test; otherwise all
documents are created and removed in a temporary directory.
"""
import argparse
import hashlib
import io
import os
from pathlib import Path
import random
import subprocess
import tempfile

from PIL import Image, ImageChops, ImageStat, ImageDraw
import pypdfium2 as pdfium
from pypdf import PdfReader, PdfWriter
from pypdf.generic import DictionaryObject, NameObject, BooleanObject, ArrayObject
from reportlab import __file__ as reportlab_file
from reportlab.lib.utils import ImageReader
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen.canvas import Canvas

REPO = Path(__file__).resolve().parents[1]
ENGINE = REPO / 'src-tauri/engine'
QPDF = ENGINE / ('qpdf.exe' if os.name == 'nt' else 'qpdf')
COMPRESSOR = ENGINE / ('compress-pdf.exe' if os.name == 'nt' else 'compress-pdf')


def create_fixture(directory):
    directory.mkdir(parents=True, exist_ok=False)
    raw = directory / 'drawing.pdf'
    rng = random.Random(781)
    image = Image.frombytes('CMYK', (1800, 1200), rng.randbytes(1800 * 1200 * 4))
    drawing = ImageDraw.Draw(image)
    for index, color in enumerate([(255, 0, 0, 0), (0, 255, 0, 0), (0, 0, 255, 0)]):
        x = 120 + index * 540
        drawing.rectangle((x, 120, x + 360, 1000), fill=color)
    encoded = io.BytesIO()
    image.save(encoded, format='JPEG', quality=95)
    font = Path(reportlab_file).parent / 'fonts/Vera.ttf'
    pdfmetrics.registerFont(TTFont('SyntheticVera', str(font)))
    canvas = Canvas(str(raw), pagesize=(612, 792))
    canvas.setFont('SyntheticVera', 12)
    canvas.drawString(30, 750, 'Synthetic compression fixture - no customer information')
    canvas.beginForm('NestedPhoto')
    canvas.drawImage(ImageReader(encoded), 0, 0, width=432, height=288)
    canvas.endForm()
    canvas.saveState()
    canvas.translate(30, 340)
    canvas.doForm('NestedPhoto')
    canvas.restoreState()
    canvas.acroForm.textfield(name='synthetic-answer', value='Saved synthetic answer',
                             x=30, y=200, width=250, height=24, forceBorder=True)
    canvas.showPage()
    canvas.setFont('SyntheticVera', 12)
    canvas.drawString(30, 750, 'Synthetic second page')
    canvas.save()
    writer = PdfWriter(clone_from=raw)
    writer.add_outline_item('Synthetic bookmark', 1)
    writer.add_attachment('synthetic.txt', b'Synthetic attachment bytes')
    writer.add_metadata({'/Title': 'Synthetic compression metadata'})
    writer.root_object[NameObject('/MarkInfo')] = DictionaryObject({NameObject('/Marked'): BooleanObject(True)})
    writer.root_object[NameObject('/StructTreeRoot')] = writer._add_object(DictionaryObject({
        NameObject('/Type'): NameObject('/StructTreeRoot'), NameObject('/K'): ArrayObject(),
    }))
    writer.pages[1].rotate(90)
    output = directory / 'synthetic.pdf'
    with output.open('xb') as stream:
        writer.write(stream)
    raw.unlink()
    return output


def run(*args):
    result = subprocess.run([str(a) for a in args], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            env={**os.environ, 'RUST_LOG': 'off'}, timeout=120)
    assert result.returncode == 0, f'Engine failed with exit {result.returncode}'


def check(directory):
    source = create_fixture(directory / 'fixture')
    original_hash = hashlib.sha256(source.read_bytes()).digest()
    before = PdfReader(source)
    text = [page.extract_text() for page in before.pages]
    for preset in ['standard', 'more']:
        output = directory / f'{preset}.pdf'
        run(COMPRESSOR, source, '--output', output, f'--{preset}', '--strip', 'none',
            '--unembed-standard-fonts', 'false', '--verify', 'structural')
        run(QPDF, '--check', output)
        after = PdfReader(output)
        assert len(after.pages) == len(before.pages)
        assert [page.extract_text() for page in after.pages] == text
        assert after.get_fields()['synthetic-answer']['/V'] == 'Saved synthetic answer'
        assert after.metadata.title == before.metadata.title
        assert after.attachments['synthetic.txt'] == [b'Synthetic attachment bytes']
        assert '/StructTreeRoot' in after.root_object
        assert [item.title for item in after.outline] == [item.title for item in before.outline]
        for a, b in zip(before.pages, after.pages):
            assert tuple(a.mediabox) == tuple(b.mediabox) and a.rotation == b.rotation
        with pdfium.PdfDocument(source) as a, pdfium.PdfDocument(output) as b:
            # The image is lossy; the separate synthetic text must still render.
            original = a[0].render(scale=1).to_pil().convert('RGB')
            compressed = b[0].render(scale=1).to_pil().convert('RGB')
            assert max(ImageStat.Stat(ImageChops.difference(original.crop((0, 0, 612, 110)), compressed.crop((0, 0, 612, 110)))).mean) < 1
            field = (30, 568, 280, 592)
            assert max(ImageStat.Stat(ImageChops.difference(original.crop(field), compressed.crop(field))).mean) < 1
            # Compare coarse image colors, so missing or badly converted nested
            # CMYK images cannot pass merely because text survived.
            box = (30, 164, 462, 452)
            original_image = original.crop(box).resize((16, 16), Image.Resampling.BOX)
            compressed_image = compressed.crop(box).resize((16, 16), Image.Resampling.BOX)
            assert max(ImageStat.Stat(ImageChops.difference(original_image, compressed_image)).mean) < 20
        assert output.stat().st_size < source.stat().st_size
        assert hashlib.sha256(source.read_bytes()).digest() == original_hash
        print(f'{preset}: {source.stat().st_size} -> {output.stat().st_size} bytes; text, saved form value, metadata, tags, attachment, bookmark and page geometry retained')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', type=Path)
    options = parser.parse_args()
    if options.fixture:
        create_fixture(options.fixture)
        print('Synthetic compression fixture created.')
    else:
        with tempfile.TemporaryDirectory(prefix='pdf-compression-check-') as work:
            check(Path(work))
