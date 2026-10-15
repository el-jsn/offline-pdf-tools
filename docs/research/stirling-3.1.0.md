# Stirling PDF 3.1.0: focused product comparison

Scope: improve the existing Merge, Compress, Split, and Organize workflows for mortgage agents. Keep processing local and preserve the current appearance choices. Ordinary viewing, annotation, signing, form filling, rotation, cropping, and image export remain with Apple Preview.

## Evidence

REA 6.5.0 CLI inspected and extracted the installed `/Applications/Stirling PDF.app/Contents/Resources/libs/stirling-pdf-3.1.0.jar` without executing Stirling or loading customer files.

- JAR SHA-256: `3afd14c6c134ff19a8f56f5fdae83b27909df13bebeac2e1c23dcb6f504cd515`
- Inspection: `ev_2fcdb7d3a523ce2df457cac1f5ceadbb9937301b239e393bc67cf62791052a15`
- Inventory: `ev_0d798aa4dc67b153bb25e157427d6d0fdae62451c91de509388ea2664d1c7b72`
- Manifest: `agm_02642e4453a55c923355fe5f09050f64f4c736efff4a152761b23e182b49a651`
- Extraction: `ev_9b3cf490e0669fc93694bf567b6549eeaad8b3c66340b84166fa03687d4ad701`

Full local REA records: `/private/tmp/stirling-jar-rea-evidence.json` and `/private/tmp/stirling-extract-rea-evidence.json`. The extracted JSON configurations below were read with ordinary file tools and their digests checked against REA's extraction record.

## Findings and decisions

| Observed packaged definition | Product implication | Decision for this app |
| --- | --- | --- |
| `Prepare-pdfs-for-email.json` chains repair, sanitization, compression, then `/api/v1/general/split-by-size-or-count` with `splitType: 0`, `splitValue: "15MB"`. | Size-based splitting complements compression when upload limits cannot be met in one file. | Implement File size inside Split. Measure finished PDFs after bookmark handling, retain all pages in order, and reject a single page that cannot fit. No additional sidebar tool. |
| `Pre-publish-sanitization.json` independently controls JavaScript, embedded files, links, metadata, and fonts; it also contains flattening and annotation removal steps. | A universal cleanup preset can remove useful document content. | Keep existing contextual document-change handling. Any future cleanup needs explicit options tied to the document. |
| `OCR images.json` chains image conversion, merging, and English OCR with `skip-text`; cleanup and deskew are disabled in this packaged preset. | Combining existing tools into a workflow is useful; OCR adds dependencies and recognition errors. | Record as a future option if agent demand establishes the need. Do not add an OCR runtime in this change. |
| `split-rotate-auto-rename.json` chains section splitting, rotation, and renaming. | Reusable settings could reduce repetitive work. | Consider settings-only presets after the four core workflows are stable. Do not retain client document content as a preset. |

These are observations of shipped configuration and implementation choices for our app. They do not prove that every declared Stirling feature is enabled or working in this desktop build. No runtime performance, output quality, cloud/offline behavior, or Java method semantics were established by this inspection.

## Existing processing improvements

The follow-up scope is improvements to the four existing tools, without additional features or UI. A local Java class-file parser read fields and constant-pool references from REA's extracted classes. Each class digest was checked against the extraction manifest. This establishes declared identities and references, not executed paths, algorithms, or runtime quality.

| Extracted class | Static observation | Decision for our implementation |
| --- | --- | --- |
| `misc/CompressController` and its `ImageIdentity` / `ImageReference` classes | Separate identity and placement records; identity fields include color space, masks, dimensions, filter, and decode parameters. Controller references image discovery, replacement, masks, and quality settings. | Improve our existing worker's shared-image handling conservatively. Size a shared stream for its largest known direct placement, using transformed axis lengths for rotation. Preserve ambiguous stream identities, masks, nested graphics, and documents with nonstandard page units. |
| `MergeController` | References combined bookmark trees, form flattening, image conversion, and PDFium merging. | Retain our existing qpdf page copying, explicit document-change handling, and bookmark remapping. No engine replacement justified by static references. |
| `SplitPDFController` | References page-range extraction, PDFBox, PDFium, and orphaned form-field pruning. | Retain strict output validation and existing form handling. Improve resource packing around page extraction instead of adding another engine. |

Resource cleanup uses qpdf's `removeUnreferencedResources: "yes"` for initial output jobs, including measured size-split candidates, and `objectStreams: "generate"` for structural packing. The navigation rewrite preserves packing without repeating forced resource analysis. Qpdf documents that forced cleanup can be slower; its default `auto` targets shared resource dictionaries. Our fixture demonstrates unused resources in separate dictionaries that `auto` leaves behind. Object streams require PDF 1.5 or later. See [qpdf's resource and output options](https://qpdf.readthedocs.io/en/stable/cli.html#option-remove-unreferenced-resources).

Focused verification on synthetic documents:

- Four worker unit checks cover largest placement, no upscaling, rotated axes, unknown nested placement, and unusual page units.
- `tests/pdf-processing-check.py` checks strict qpdf validation, page order, extracted text, unchanged originals, and rendered pixels. It requires Pillow, reportlab, pypdf, and pypdfium2 plus the built debug worker and staged Mac runtimes.
- On its crafted unused-resource fixture: Merge 790,406 to 592,166 bytes; Split 788,857 to 197,930; Organize 789,452 to 395,043; lossless Compress 789,030 to approximately 592,152. Every rendered page is identical. These savings demonstrate this fixture, not expected savings on arbitrary PDFs.
- A shared 2400 × 1200 image used at several sizes, with the largest placement rotated 90 degrees, becomes 960 × 480 at Balanced's 240 DPI. The pre-change source worker skips it; the updated worker replaces one shared stream. Output is approximately 356 KB instead of 8.64 MB after structural packing. This is a lossy synthetic noise image; the result is not a general compression or quality benchmark. Text and the separate vector line are unchanged.
- Shared images also used inside nested graphics remain unchanged. A document using `/UserUnit: 2` remains unchanged by the image worker. Their rendered pages are identical.
- Existing bookmark-remapping and finished-file-size split checks pass with the new packing options, including cleanup after a page that cannot fit. No broad test suite was run.

Verified class SHA-256 identities (under `BOOT-INF/classes/stirling/software/SPDF/controller/api/`):

- `misc/CompressController.class`: `cda7be8cc0a00e1c100ff523da9a44851562780372c439fff16734b50de07231`
- `misc/CompressController$ImageReference.class`: `7772cbed7bffc884e28925307ba22c73219f0fb57d8fd390f2921f59ab879a00`
- `misc/CompressController$ImageIdentity.class`: `96edd1ff980da9198133b6216baf1a7404a613066e854dbf792e84975da4b4c1`
- `MergeController.class`: `2447d9a4d1239bb74e10e25b2fc6af86dc70403a05de4037df8cbc3e7133c63f`
- `SplitPDFController.class`: `03799bf5ff1f97e6fb222093cfddf97891264b15a495162f9710624d3f936d6c`

## Verified configuration identities

- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/OCR images.json` — SHA-256 `04e001e65a96f98121d7a97094e4d81e307500c833c746c8ca42ff7e688f40e1`
- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/Pre-publish-sanitization.json` — SHA-256 `e62b49d9bc082294b7b9836e8f5f8f7b1fdf309029a23b32e22c4a7b31cff66d`
- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/Prepare-pdfs-for-email.json` — SHA-256 `a2fd9e5c745229f93e4cdd67c9eb1cb7cba029b08c2cb68095a8924396c5d831`
- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/split-rotate-auto-rename.json` — SHA-256 `e2743fd653b2e1803f41eaf4f9580883f340a410e7d3ab7b964369fd23f4e5f7`
