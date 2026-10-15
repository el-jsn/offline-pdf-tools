# PDF Tools

A desktop utility for merging, compressing, splitting, and organizing PDFs locally.
Documents are never uploaded.

## Features

- Merge PDFs and JPEG, PNG, or WebP images in the order you choose. Each image becomes one page, preserving its proportions and orientation, with transparency on white. Images are limited to 32 megapixels; animated images use their first frame. HEIC is not supported.
- Compress individually or in batches: Balanced by default, with lossless and Smallest options.
- Balanced and Smallest resize and recompress embedded images, including CMYK photographs. Text and vector objects are preserved.
- Shared images use their largest displayed size when compressed. All four tools remove unused page resources and pack PDF objects without changing page content.
- See the actual compressed size before saving and compare the original and compressed pages. Set an optional limit per PDF; an unmet limit is identified. Saving uses the measured output.
- Preview source PDFs in all four tools with page navigation and zoom. Organize previews follow the current page order and rotations.
- Organize pages with group dragging, edge scrolling, keyboard selection, previews, rotation, duplication, removal, undo/redo, and moves to a specific position.
- Split with thumbnail selection, named page groups, fixed groups, file size limits, every page, bookmarks, or page split points. Extract selected pages together or separately, combine custom groups, and preview output page lists.
- Choose System, Light, or Dark appearance in the sidebar.
- Add files through drag and drop or the file picker. Invalid or unsupported files are listed individually; valid files in the same batch remain available.

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
The native runtimes are qpdf 12.1+ and PDFium. On a fresh checkout, stage their
libraries and matching license notices using `scripts/stage-engine.py` and
`scripts/stage-pdfium.py`. The release build compiles and bundles the image worker.

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
- Digital signatures cannot be preserved through processing. A changed copy requires explicit acknowledgement; originals remain untouched.
- Page bookmarks are remapped to retained pages. Bookmarks without a resolvable page destination can be omitted. Merging retains attachments with distinct prefixes. Changed page layouts can discard named destinations, XMP metadata, and accessibility tags; accessibility changes require acknowledgement.

Input PDFs with recoverable compressed-stream warnings are repaired in a temporary
working copy. The repaired copy must pass strict structure, stream, and page-count
checks before processing. Unrepairable files are rejected. Warning-only processing
results are accepted only after the output passes a separate structure, stream, and
page-count check. Recovered data is identified in the status.

Lossless compression may produce little reduction. Balanced and Smallest resize
and JPEG-compress eligible images and convert their decoded colors to RGB.
Transparency masks, one-bit images, ambiguous image dictionaries, and images above
32 million pixels are preserved. Images inside nested graphics and images in documents with nonstandard page units are also preserved. Other presets can reduce image quality. A file that cannot be made smaller is saved as an unchanged copy.

PDF processing and previews work offline. Update checks and installer downloads
contact GitHub only when requested. Private working copies are removed when cleared or the app quits. Closing the Mac window keeps the current session in memory and its private workspace available. Abandoned private workspaces are reclaimed on a later launch; active instances are protected by file locks. A forced quit during the final output publication step can leave a staging file in the selected destination.

Organize accepts PDFs and JPEG, PNG, or WebP images through Add files or file drops. Drop beside a page to insert there, or outside the grid to append. Added pages retain their source for previews and saving, and insertion can be undone.

File size splitting uses decimal MB (1 MB = 1,000,000 bytes), measures completed outputs including bookmark changes, and preserves all pages in order. It does not reduce image quality. If one page cannot fit, the operation stops without publishing a partial output folder. The split need not use the fewest possible files.
