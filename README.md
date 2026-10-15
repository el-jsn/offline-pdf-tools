# PDF Tools

A desktop utility for merging, compressing, splitting, organizing, and comparing PDFs locally.
Documents are never uploaded.

## Features

- Merge PDFs and JPEG, PNG, or WebP images in the order you choose. Each image becomes one page, preserving its proportions and orientation, with transparency on white. Images are limited to 32 megapixels; animated images use their first frame. HEIC is not supported.
- Compress individually or in batches: Balanced by default, with Original quality and Smaller options.
- Balanced and Smaller use the bundled compress-pdf engine to optimize images, embedded fonts, and PDF structure.
- Compression accounts for shared-image placements. Merge, Split, and Organize remove unused page resources and pack PDF objects without changing page drawing instructions.
- Choose a preset and click Compress PDF, then review the actual output size or compare pages before clicking Save PDF. Set an optional limit per PDF; an unmet limit is identified. Saving uses the already compressed output.
- Preview source PDFs in all four tools with page navigation and zoom. Organize previews follow the current page order and rotations.
- Organize pages with group dragging, edge scrolling, keyboard selection, previews, rotation, duplication, removal, undo/redo, and moves to a specific position.
- Split with thumbnail selection, named page groups, fixed groups, file size limits, every page, bookmarks, or page split points. Extract selected pages together or separately, combine custom groups, and preview output page lists.
- Choose System, Light, or Dark with the sidebar's appearance buttons. The running Mac app's Dock icon follows the selected appearance.
- Copy the latest app download link from the sidebar's share button.
- When files are skipped or processing fails, use Copy error details to copy an in-memory support report with the failed step, available settings, file sizes/page counts, engine exit codes, processing steps, and warning categories with occurrence counts. Repeated warnings are grouped, and capture/report limits are identified. Paths, filenames, passwords, object values and document-derived strings are excluded; unrecognized raw diagnostics are counted and omitted. Import reports remain available alongside the skipped-file list, including when all files are rejected. Reports are never uploaded or written to log files.
- Add files through drag and drop or the file picker. Invalid or unsupported files are listed individually; valid files in the same batch remain available.
- Keep an optional page preview beside the working list. Small windows show it over the workspace; full preview remains resizable.
- Review split outputs by cover, page range, and editable filename. Merge can create document bookmarks from filenames, retaining existing bookmarks beneath them.
- Save named upload limits and remember compression settings and thumbnail size. These settings contain no document content or paths. Size displays and upload limits use decimal MB.
- Continue from saved outputs into another tool without choosing those files again. Only the recorded output files are imported, up to 100 at a time.
- Check Organize pages for possible blanks and duplicates. Findings are review suggestions; pages are never removed automatically.
- Compare extracted page text and saved form values in an original and revised PDF, review changed passages and fields, and open their source pages. Comparison runs locally and supports cancellation.

Original files are preserved. Outputs are saved as new files.

## Install

Download the installer for your computer from [Releases](https://github.com/el-jsn/offline-pdf-tools/releases).

- **Mac:** open the DMG and drag PDF Tools to Applications.
- **Windows:** run the setup installer.

No developer tools or GitHub account are needed to use a public release.
Use **Check for updates** in the app to install published updates, or close the
app and install a newer release over the existing copy.

The current build is for Apple Silicon Macs. Windows installer configuration is
included, but a Windows release has not been built or validated yet. Current
installers are not Apple Developer ID signed/notarized or Windows publisher signed,
so operating system security warnings may appear.

## Build

Requires Node 22.16+, npm, Rust 1.90+, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).
The native runtimes are qpdf 12.1+, PDFium, and compress-pdf 0.1.0. On a fresh checkout, stage their
libraries and matching license notices using `scripts/stage-engine.py` and
`scripts/stage-pdfium.py`, then `scripts/stage-compressor.py` as described in
the release instructions. Building compress-pdf separately needs Rust 1.92+.
The release build compiles and bundles the image worker.

```sh
npm ci
npm run desktop
```

To create installers and signed update files:

```sh
npm run release
```

Release files are collected in `releases/<version>/`. Release signing requires the
existing private update key; it is not committed to this repository. See
[release instructions](docs/releases.md) for engine staging, version changes, and publishing.

## Processing limits

- Up to 100 selected PDFs, 250 MB per file, and 10,000 pages per file.
- Up to 20,000 pages per operation and 500 split outputs.
- Previews support selected files up to 250 MB using local range reads. Complex PDFs can still exceed rendering memory limits.
- Encrypted PDFs that open without a password are accepted. Other encrypted PDFs require a supplied password; output copies are unencrypted. Ordinary form fields can be preserved during compression or explicitly flattened for page operations. XFA forms and forms requiring regenerated appearances are unsupported for page operations.
- Digitally signed PDFs cannot be compressed without invalidating their signatures, so compression requires an unsigned copy. Empty signature fields are not treated as signed. Page operations require explicit acknowledgement of signature removal; originals remain untouched.
- Compare supports up to 500 pages, 1,000,000 extracted text and field characters, and 200,000 page-text tokens per PDF. Page-text comparison ignores whitespace and page breaks. Saved text fields, choices, checkboxes and radio selections are compared by field name; repeated widgets are grouped. Changes to field names or types appear as removed and added fields. Images, annotations, formatting, signatures, field appearance streams and XFA forms are not compared. PDFs without readable text or supported form fields require OCR in another application. Pages with no extracted text are identified; their visual content is not verified. Large differences may be grouped into a single passage. PDF text extraction order can affect results.
- Organize page checks support up to 500 pages. Blank detection uses a small rendered page and extracted text. Duplicate suggestions match the rendered preview dimensions, pixels, and extracted text, not the complete PDF objects. Small marks can be missed; all findings require visual review.
- Page bookmarks are remapped to retained pages. Bookmarks without a resolvable page destination can be omitted. Merging retains attachments with distinct prefixes. Changed page layouts can discard named destinations, XMP metadata, and accessibility tags. Accessibility tag changes show a notice; digital signature removal requires confirmation.

Input PDFs with recoverable compressed-stream warnings are repaired in a temporary
working copy. Disconnected form widgets can also be reconnected when they are exact
duplicates of existing fields, including their saved values and appearance references.
Conflicting or ambiguous duplicates and nested orphan-field structures are not repaired
automatically. The repaired copy must pass strict structure, stream, and page-count
checks before processing. Unrepairable files are rejected. Warning-only processing
results are accepted only after the output passes a separate structure, stream, and
page-count check. Recovered data is identified in the status.

Read-only metadata inspection accepts QPDF's warning-only status when its JSON and
page-count results are valid. Fatal inspection errors and malformed metadata are
rejected. This does not relax validation of saved outputs.

If the existing qpdf repair still leaves warnings, a bundled pypdf 6.20.1 worker
can reconnect ordinary flat form controls missing from the field tree. It rejects
conflicting copies, repeated names, nested fields, XFA and signed documents.
Decoded page content, resources, annotations, control appearances and saved values
must survive the rewrite; the result must also pass a clean qpdf check. This
fallback runs during processing only, without adding work to normal imports.

Original quality uses qpdf's lossless structural compression and may produce little
reduction. Balanced uses compress-pdf 0.1.0's Standard preset (150 DPI target,
JPEG quality 60); Smaller uses More (72 DPI target, JPEG quality 60, RGB image
conversion). Both also optimize embedded fonts and PDF structure. Document
stripping and standard-font unembedding are disabled. Metadata, accessibility
tags and output intents are retained; bookmarks, attachments and saved form values
are checked before accepting a compressed copy. Unsupported content may be kept
unchanged or rejected by the compressor. Image quality and font appearance can
change, so review the comparison preview. These are lossy presets, not pixel-exact
preservation. A file that cannot be made smaller is saved as an unchanged copy.

An upload limit tries progressively lower image resolutions within the selected
preset's floor, always starting each attempt from the same private source. It
does not guarantee that a requested size can be reached.

PDF processing and previews work offline. Update checks and installer downloads
contact GitHub only when requested. Private working copies are removed when cleared or the app quits. Closing the Mac window keeps the current session in memory and its private workspace available. Abandoned private workspaces are reclaimed on a later launch; active instances are protected by file locks. A forced quit during the final output publication step can leave a staging file in the selected destination.

Organize accepts PDFs and JPEG, PNG, or WebP images through Add files or file drops. Drop beside a page to insert there, or outside the grid to append. Added pages retain their source for previews and saving, and insertion can be undone.

File size splitting uses decimal MB (1 MB = 1,000,000 bytes), measures completed outputs including bookmark changes, and preserves all pages in order. It does not reduce image quality. If one page cannot fit, the operation stops without publishing a partial output folder. The split need not use the fewest possible files.
