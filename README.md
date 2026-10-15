# PDF Tools

A desktop utility for merging, compressing, splitting, and organizing PDFs locally.
Documents are never uploaded.

## Features

- Merge PDFs and JPEG, PNG, or WebP images in the order you choose. Each image becomes one page, preserving its proportions and orientation, with transparency on white. Images are limited to 32 megapixels; animated images use their first frame. HEIC is not supported.
- Compress individually or in batches: Balanced by default, with lossless and Smallest options.
- Balanced and Smallest resize and recompress embedded images, including CMYK photographs. Text and vector objects are preserved.
- See the actual compressed size before saving. Saving uses the measured output.
- Preview source PDFs in all four tools with page navigation and zoom. Organize previews follow the current page order and rotations.
- Organize pages with group dragging, edge scrolling, keyboard selection, quick preview, rotation and removal, undo, and moves to a specific position.
- Split with thumbnail selection, custom page groups, fixed groups, every page, or page split points. Extract selected pages together or separately, combine custom groups, and preview output page lists.
- Choose System, Light, or Dark appearance in the sidebar.
- Add files through drag and drop or the file picker.

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
- Thumbnail previews support files up to 64 MB; larger files can still be processed.
- Encrypted PDFs and actual fillable form fields are unsupported. Empty form metadata is accepted.
- Signed originals should not be processed: signature preservation is not supported.
- Page operations may discard bookmarks, attachments, metadata, and accessibility tags.

Warning-only processing results are accepted only after the output passes a separate
structure, stream, and page-count check. Recovered data is identified in the status.

Lossless compression may produce little reduction. Balanced and Smallest resize
and JPEG-compress eligible images and convert their decoded colors to RGB.
Transparency masks, one-bit images, ambiguous image dictionaries, and images above
32 million pixels are preserved. Other presets can reduce image quality. A file that cannot be made smaller is saved as an unchanged copy.

PDF processing and previews work offline. Update checks and installer downloads
contact GitHub only when requested. Temporary working files are normally removed
when finished or cleared; an interrupted app or forced quit can leave them behind.

Organize accepts PDFs and JPEG, PNG, or WebP images through Add files or file drops. Drop beside a page to insert there, or outside the grid to append. Added pages retain their source for previews and saving, and insertion can be undone.
