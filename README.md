# PDF Tools

A desktop utility for merging, compressing, splitting, and organizing PDFs locally.
Documents are never uploaded.

## Features

- Merge multiple PDFs in the order you choose.
- Compress individually or in batches, with three quality settings.
- See the actual compressed size before saving. Saving uses the measured output.
- Split by page, fixed groups, or custom ranges.
- Preview, reorder, rotate, and remove pages.
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
The bundled PDF engine is qpdf 12.1 or newer. On a fresh checkout, stage its native
runtime and matching license notices using `scripts/stage-engine.py`.

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

Lossless compression may produce little reduction. Other presets can reduce image
quality. A file that cannot be made smaller is saved as an unchanged copy.

PDF processing and previews work offline. Update checks and installer downloads
contact GitHub only when requested. Temporary working files are normally removed
when finished or cleared; an interrupted app or forced quit can leave them behind.
