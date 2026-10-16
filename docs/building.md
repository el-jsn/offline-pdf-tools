# Building PDF Tools

[Back to the README](../README.md)

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
[release instructions](releases.md) for engine staging, version changes, and publishing.
