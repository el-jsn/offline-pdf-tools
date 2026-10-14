# PDF Tools

A personal, offline desktop application for internal use on macOS and Windows.
The name is a working label. Separate from the Mortio web application.

## Source implementation

- Single light theme inspired by Apple's Human Interface Guidelines.
- Add PDFs by native file drop or multi-file selection. Drops never navigate the webview.
- Merge PDFs with pointer reordering, keyboard shortcuts, and move buttons.
- Compress one or multiple PDFs with lossless, JPEG quality 80, or JPEG quality 55 presets. Batch outputs are saved separately in a new folder.
- Split into single pages, fixed groups, or custom page ranges.
- Organize using real page thumbnails: reorder, rotate, remove, and undo.
- Local previews through Mozilla PDF.js with a bundled worker and support assets.
- Native open/save dialogs and folder selection. Reveal saved outputs in Finder
  or Windows Explorer.
- Unfinished work is preserved separately for each tool during the app session.
- Cancellable processing, fingerprint checks, structural PDF checks, page-count
  verification, and new output files only. Originals are never overwritten.
- No accounts, uploads, analytics, runtime downloads, or automatic updater.

The Apple Silicon macOS app and DMG have now been built through the standard
Tauri packager with the native qpdf runtime included. The packaged engine runs
without Homebrew on PATH. Windows builds and clean-machine PDF validation remain
pending. Releases are not yet Developer ID signed or notarized.

## Development prerequisites

Node 22.16+, pnpm, a Rust toolchain, Tauri 2 platform prerequisites, and qpdf 12.1+
from https://github.com/qpdf/qpdf. The current PDF.js dependency is pinned to 6.4.299.

Install JavaScript dependencies with `npm install`. Start the desktop app with
`npm run desktop`. Build the platform installer with `npm run package`. The hooks
use npm directly; no JSON configuration overrides are needed.

The engine is included in the local Mac source checkout. For a fresh source clone,
stage a matching native engine before building. PDF_TOOLS_QPDF is an optional debug
override, accepted only in debug builds and ignored in releases.
PDF.js support assets are copied locally by `scripts/prepare-assets.mjs` before
Vite starts; no CDN is used. Desktop commands compile Rust. The macOS release
build was authorized and completed, and the packaged app reports Ready.

Individual JavaScript files can be checked with `node --check path/to/file.js`.

## Packaging

The app must ship its own qpdf runtime. Agents must not install PDF processing
libraries or run terminal commands. The staging script does not download anything:

macOS: `python3 scripts/stage-engine.py /absolute/path/to/qpdf --notices /path/to/notices`

Windows: `py scripts/stage-engine.py C:\qpdf\bin\qpdf.exe --notices C:\qpdf\notices`

Supply notices matching qpdf and all native runtime dependencies. On macOS the
script copies dependent dylibs, adjusts their loader paths, and applies local ad hoc
signatures. On Windows it copies neighboring DLLs. It records file hashes in a
manifest. Run on the target OS/architecture. The staging script has been executed on Apple Silicon macOS. Windows staging is
not yet validated. Release builds check engine presence, runtime loading, version,
platform, notices, and file hashes before compiling.

Before distribution:

1. Install and lock Node and Rust dependencies; check toolchain compatibility.
2. Stage the native engine with its exact license notices and support libraries.
3. Confirm icon assets, the final product name, identifier, and supported OS versions.
4. Build and validate on clean Mac and Windows machines when builds are authorized.
5. Decide whether to use developer signing/notarization. The app has no subscription,
   but signing services may have costs. Ad hoc signing is not notarization.

See https://v2.tauri.app/start/prerequisites/ and the engine folder README.

## Practical limits

Inputs are limited to 100 selected documents across all tools, 250 MB per document,
10,000 pages per document, and 20,000 total input/output pages per job. Splitting is
limited to 500 output PDFs per job. Thumbnails render at most two at once and the
organizer shows 48 pages per view. Previews support documents up to 64 MB; larger
PDFs still support processing and show an unavailable-preview label.

The qpdf engine preserves vector text during page operations. Lossless compression
may produce little reduction. Image presets can affect readability and do not
resample images or promise a target file size. If compression increases file size,
a byte-for-byte copy of the unchanged source snapshot is saved instead.

PDFs containing actual interactive fields or encryption are rejected. Empty
AcroForm containers are accepted; they are not treated as fillable forms. Form detection is not a
complete digital-signature audit: do not process signed originals. Merging,
splitting, and organizing copy pages into new PDFs and do not preserve every
outline, attachment, metadata entry, or accessibility tag. There is no OCR,
redaction, text editing, password entry, or digital signing in this version.
All qpdf warnings currently fail the job instead of silently repairing files.

Working files are placed in a private temporary directory within the selected
output folder and removed on ordinary completion or error. A crash, forced quit,
or power loss can leave `.pdf-tools-job-*` directories or a partial split-output
folder. A short final save phase completes without cancellation. Saved copies and
working directories can sync externally if the chosen folder is cloud-synced or
on network storage; choose local, unsynced storage when files must stay on-device.
There is no secure-erasure claim or protection from other software on the computer.
qpdf runs under the user's OS account rather than an OS sandbox.

Before agent use, validate sample PDFs, page order/rotation/count, fine-text
readability, forms, signed documents, encrypted and damaged inputs, offline use,
cancellation, long/Unicode names, file permissions, disk space failures, and output
collisions on both platforms. Use synthetic documents; never commit client PDFs.

## Implementation references

Design decisions and official sources are recorded in [docs/design.md](docs/design.md).
PDF structure and operation schema: https://qpdf.readthedocs.io/en/stable/qpdf-job.html
Local page rendering: https://mozilla.github.io/pdf.js/examples/

## Maintenance

Update Tauri, Rust crates, PDF.js, and the bundled native engine when fixes are
released. Revalidate PDF behavior and target-platform installers after relevant
changes. No automatic update service is included.

## Automatic compression size

Adding PDFs in Compress or changing the quality setting automatically runs the
actual local compressor. The UI shows the verified output size, with per-file
sizes for batches. Saving copies those exact cached bytes after rechecking the
input files and cached output hashes. Sizes are not inferred from a percentage.
When compression does not make a file smaller, its unchanged copy is measured
and saved instead. Measurements can be cancelled; no size is shown on failure.

Measured outputs stay in a private OS temporary directory during the session,
and are removed when replaced, cleared, or on normal app exit. A crash or forced
quit may leave temporary files. This adds a compression pass when settings change,
but saving a successfully measured selection does not recompress it.
