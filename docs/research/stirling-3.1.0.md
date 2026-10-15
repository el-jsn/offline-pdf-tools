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

## Verified configuration identities

- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/OCR images.json` — SHA-256 `04e001e65a96f98121d7a97094e4d81e307500c833c746c8ca42ff7e688f40e1`
- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/Pre-publish-sanitization.json` — SHA-256 `e62b49d9bc082294b7b9836e8f5f8f7b1fdf309029a23b32e22c4a7b31cff66d`
- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/Prepare-pdfs-for-email.json` — SHA-256 `a2fd9e5c745229f93e4cdd67c9eb1cb7cba029b08c2cb68095a8924396c5d831`
- `BOOT-INF/classes/static/pipeline/defaultWebUIConfigs/split-rotate-auto-rename.json` — SHA-256 `e2743fd653b2e1803f41eaf4f9580883f340a410e7d3ab7b964369fd23f4e5f7`
