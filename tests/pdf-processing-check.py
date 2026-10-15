"""Focused synthetic PDF checks; requires Pillow, reportlab, pypdf, pypdfium2.

Build pdf-image-worker first, then run this file from any directory. No customer
documents are used. Fixtures and outputs are removed when the check completes.
"""
import hashlib
import json
import random
import subprocess
import tempfile
from pathlib import Path

import pypdfium2 as pdfium
from PIL import Image
from pypdf import PdfReader, PdfWriter
from pypdf.generic import NameObject, NumberObject
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen.canvas import Canvas

REPO = Path(__file__).resolve().parents[1]
ENGINE = REPO / "src-tauri/engine/qpdf"
WORKER = REPO / "src-tauri/target/debug/pdf-image-worker"
LIBRARY = REPO / "src-tauri/engine/libpdfium.dylib"


def run(*args):
    return subprocess.check_output([str(arg) for arg in args], stderr=subprocess.PIPE)


def assert_pages(source, output, order, identical=True):
    run(ENGINE, "--check", output)
    source_text = PdfReader(source)
    output_text = PdfReader(output)
    assert len(output_text.pages) == len(order)
    with pdfium.PdfDocument(source) as before, pdfium.PdfDocument(output) as after:
        for index, original in enumerate(order):
            assert source_text.pages[original].extract_text() == output_text.pages[index].extract_text()
            a = before[original].render(scale=1).to_pil()
            b = after[index].render(scale=1).to_pil()
            assert a.size == b.size
            if identical:
                assert a.tobytes() == b.tobytes()
            else:
                # Text and vector line above the synthetic image stay unchanged.
                assert a.crop((0, 0, a.width, 120)).tobytes() == b.crop((0, 0, b.width, 120)).tobytes()


def check_resource_cleanup(root):
    base = root / "pages.pdf"
    canvas = Canvas(str(base))
    rng = random.Random(82)
    for index in range(4):
        image = Image.frombytes("RGB", (256, 256), rng.randbytes(256 * 256 * 3))
        canvas.drawImage(ImageReader(image), 70, 220, width=256, height=256)
        canvas.drawString(70, 520, f"Synthetic processing page {index + 1}")
        canvas.showPage()
    canvas.save()
    writer = PdfWriter(clone_from=base)
    refs = [next(iter(page["/Resources"]["/XObject"].values())) for page in writer.pages]
    # Separate resource dictionaries escape qpdf's default shared-resource cleanup.
    for page in writer.pages[:3]:
        for index, ref in enumerate(refs):
            page["/Resources"]["/XObject"][NameObject(f"/Unused{index}")] = ref
    writer.remove_page(3)
    source = root / "unused-resources.pdf"
    writer.write(source)
    digest = hashlib.sha256(source.read_bytes()).digest()
    for operation, order in [("merge", [0, 1, 2, 0]), ("split", [1]), ("organize", [2, 0]), ("compress", [0, 1, 2])]:
        sizes = []
        for compact in [False, True]:
            output = root / f"{operation}-{compact}.pdf"
            job = {"inputFile": str(source), "outputFile": str(output), "pages": [{"file": str(source), "range": ",".join(str(i + 1) for i in order)}]}
            if operation == "compress":
                job.update(compressStreams="y", recompressFlate="", compressionLevel="9", objectStreams="generate")
            if compact:
                job.update(removeUnreferencedResources="yes", objectStreams="generate")
            config = root / "job.json"
            config.write_text(json.dumps(job))
            run(ENGINE, f"--job-json-file={config}")
            assert_pages(source, output, order)
            sizes.append(output.stat().st_size)
        assert sizes[1] < sizes[0], (operation, sizes)
        print(f"{operation}: {sizes[0]} -> {sizes[1]} bytes; identical pages")
    assert hashlib.sha256(source.read_bytes()).digest() == digest


def check_shared_images(root):
    rng = random.Random(113)
    image = ImageReader(Image.frombytes("RGB", (2400, 1200), rng.randbytes(2400 * 1200 * 3)))
    for kind in ["shared", "nested", "scaled"]:
        source = root / f"{kind}.pdf"
        canvas = Canvas(str(source))
        for index in range(3):
            canvas.drawString(30, 760, f"Synthetic shared image page {index + 1}")
            canvas.setStrokeColorRGB(1, 0, 0)
            canvas.line(30, 730, 400, 730)
            if index == 0:
                canvas.drawImage(image, 30, 500, width=72, height=36)
            elif index == 1:
                canvas.saveState()
                canvas.translate(300, 300)
                canvas.rotate(90)
                canvas.drawImage(image, 0, 0, width=288, height=144)
                canvas.restoreState()
            elif kind == "nested":
                canvas.beginForm("Nested")
                canvas.drawImage(image, 0, 0, width=360, height=180)
                canvas.endForm()
                canvas.saveState()
                canvas.translate(30, 400)
                canvas.doForm("Nested")
                canvas.restoreState()
            else:
                canvas.drawImage(image, 30, 400, width=144, height=72)
            canvas.showPage()
        canvas.save()
        if kind == "scaled":
            writer = PdfWriter(clone_from=source)
            writer.pages[1][NameObject("/UserUnit")] = NumberObject(2)
            writer.write(source)
        digest = hashlib.sha256(source.read_bytes()).digest()
        directory = root / kind
        directory.mkdir()
        index, patch = directory / "index.json", directory / "patch.json"
        run(ENGINE, "--json-output=2", "--json-stream-data=file", f"--json-stream-prefix={directory}/stream", source, index)
        config = directory / "config.json"
        config.write_text(json.dumps(dict(library=str(LIBRARY), document=str(source), index=str(index), patch=str(patch), directory=str(directory), preset="balanced")))
        stats = json.loads(run(WORKER, config))
        replacements = json.loads(patch.read_text())["qpdf"][1]
        assert stats["images"] == (1 if kind == "shared" else 0)
        if kind == "shared":
            assert [(v["stream"]["dict"]["/Width"], v["stream"]["dict"]["/Height"]) for v in replacements.values()] == [(960, 480)]
        output = directory / "output.pdf"
        run(ENGINE, f"--update-from-json={patch}", "--object-streams=generate", source, output)
        assert_pages(source, output, [0, 1, 2], identical=kind != "shared")
        assert hashlib.sha256(source.read_bytes()).digest() == digest
        print(f"{kind}: {stats['images']} image replacements; pages and text verified")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="pdf-processing-check-") as directory:
        root = Path(directory)
        check_resource_cleanup(root)
        check_shared_images(root)
