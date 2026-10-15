# Document workbench

Audience: company mortgage agents assembling and reviewing local document packages.

Design direction: keep the current system typography and appearance choices. White `#ffffff` surfaces, near-black `#1d1d1f` text, quiet `#f7f7f8` navigation, `#e5e5e9` divisions, and `#0066cc` actions carry the light layout. Existing dark appearance tokens remain supported. Use the system font at 13 px for controls, 11–12 px for source metadata, and 15 px for the active tool title.

Layout: navigation on the left, working documents in the center, optional reading pane on the right. Text and controls align to the working document edges. At narrow widths the reading pane overlays the workspace with a visible close action. There are no decorative panels, banners, or new instructional sections. Selection actions appear when pages are selected; undo and page checking remain available independently.

The distinctive element is the document itself: split covers and page ranges for output review, source labels for mixed packages, and two columns showing removed and added wording. Controls use the existing SVG assets. The comparison limitation is functional information beside its results.

Implementation:

- Optional inspector follows clicked pages, preserving Organize order and rotations. Full preview and synchronized compression comparison remain available.
- Split previews show actual numbered output filenames after the same character filtering used by saving. Explicit output-name edits are reset when selecting a new source document.
- Merge optionally nests existing bookmarks beneath filename bookmarks. Existing signature/form acknowledgements still apply.
- Saved-output continuation imports the exact recorded output paths, excluding unrelated files later added to the output directory. Existing Merge, Compress, and Organize selections receive additions; Split replaces its source like the ordinary file picker.
- Upload-limit presets, quality, target, and thumbnail size are stored locally. Files, text, output names, and comparison results are not persisted as preferences. Upload limits and sizes use decimal MB.
- Comparison reads PDF.js page text with geometric spacing between font runs. Word comparison runs in a bundled worker with bounded edit-distance memory and a grouped fallback. Pages without text and grouped fallback results are identified. Images, form values, formatting, annotations, and signatures are outside comparison scope.
- Page checking compares bounded rendered previews and text, identifies possible blanks and duplicates, and offers source-page review. It never deletes or selects findings automatically. Edits invalidate findings.

Focused validation uses generated synthetic documents only. Native Mac checks verified a changed amount, choosing comparison slots in either order, a docked source-page preview, intended blank/duplicate findings, Organize saving, continuation into Split, edited output naming, identical saved split text/pixels, and saving/deleting a named upload limit. Unit checks cover changed amounts, page locations, repeated words, minimum edit counts against a small independent oracle, font-run spacing, large-change fallback, and settings validation. Native Rust checks verify filename bookmark nesting/remapping and output-path registration. No broad test suite or release publication is part of this change.
