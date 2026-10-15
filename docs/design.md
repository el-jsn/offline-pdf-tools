# Desktop design and implementation

## Design direction

A document utility inspired by Preview and Finder. The content is the document;
there is no marketing page or company branding. The working label is PDF Tools.

- White content surface #FFFFFF; sidebar #F7F7F8; borders #E5E5E9; text #1D1D1F;
  secondary text #66666D; action accent #0066CC.
- Use installed system fonts: San Francisco on macOS, Segoe UI on Windows.
  Body text 13px, supporting text 11px, toolbar titles 15px.
  Use normal and semibold weights, no decorative typefaces or font downloads.
- Narrow sidebar selects the four tools. A compact toolbar has a contextual title,
  file selection, and one primary action. A persistent bottom status area contains
  progress, cancellation, and the saved-file action. The document area scrolls.
- System, Light, and Dark appearances with matching native window themes. PDF
  paper retains its original colors. Native OS window controls,
  open/save dialogs, and folder selection; no simulated Mac traffic-light buttons.
- Static SVG icons, visible keyboard focus, reduced motion, descriptive labels,
  keyboard shortcuts, inline errors, and no routine confirmation modals.

UI copy uses direct labels, compact feedback, and collapsed limitations. There are
no promotional headings or repeated reassurance. The design avoids a website hero,
dashboard metrics, promotional
cards, glass effects, and excessive color. Compression choices are grouped options
because they represent a meaningful tradeoff. Page thumbnails are actual local
PDF.js canvas renders, not placeholders pretending to be document contents.

## Interaction decisions

Each tool preserves its unfinished selection and settings when switching views.
Native drops import files without webview navigation. Rust accepts dropped paths
only after receiving the native drop event. Merge and Compress both support
multi-file selection. Internal reordering uses pointer events so it remains
available when native file drops are enabled on Windows. Merge order can also
be changed with accessible move buttons.
Organizing supports selection, range selection with Shift, rotation in 90-degree
increments, removing pages, dragging, move buttons, and 50 steps of undo.
Large organizers show 48 pages at a time; the output includes all retained pages.
A page can be previewed with Enter or double-click. Ctrl/Cmd+O selects files;
Ctrl/Cmd+S starts saving; Ctrl/Cmd+Z undoes an organizer edit.

Splitting validates ranges on both sides of IPC. Semicolons separate output files;
commas combine pages within one output. The default produces one file per page;
groups and custom ranges are alternatives. A new output folder prevents collisions.

Lossless compression is the default. The other presets explicitly select JPEG
quality 80 or 55, which can alter image detail. No preset guarantees a target size.
A size target first runs the selected preset, then tries additional settings only
if needed, retaining the smallest valid candidate. Targets are best effort.
If optimization increases the size, an unchanged source copy is saved instead.

Progress describes actual stages. It uses an activity indicator during engine work
rather than an invented percentage. Cancellation terminates qpdf and discards
working output before publication. The short final save step is not interruptible.
Native file dialogs have their own cancellation control.

## PDF implementation

qpdf is used for PDF structure, page copying, rotation, and compression. Text is
not rasterized for merging, splitting, or organizing. Organized rotations are
relative to the original page orientation and apply to positions in the output.
qpdf's documented JSON job format avoids Windows command-length limits for large
page plans. PDF.js renders thumbnails using a bundled worker, CMaps, ICC profiles, fonts, and WASM assets.
PDF embedded scripts and actions are not executed. No CDN resources are used.

Selected documents are kept behind opaque IDs. Rust verifies SHA-256 fingerprints
against private working snapshots before processing. Output is checked by qpdf and
its page count is compared with the plan before publication. Existing files are
never overwritten. Split output is validated before files are placed in a new
folder; an ordinary publication error removes that folder through RAII cleanup.

## Sources consulted

- https://developer.apple.com/design/human-interface-guidelines/sidebars
- https://developer.apple.com/design/human-interface-guidelines/toolbars
- https://developer.apple.com/design/human-interface-guidelines/typography
- https://developer.apple.com/design/human-interface-guidelines/progress-indicators
- https://developer.apple.com/design/human-interface-guidelines/alerts
- https://qpdf.readthedocs.io/en/stable/cli.html
- https://mozilla.github.io/pdf.js/examples/
- https://v2.tauri.app/develop/calling-rust/

## Verification status

The standard Tauri Apple Silicon macOS release build and DMG packaging completed
after the user authorized builds. The packaged engine runs without Homebrew on
PATH, and the packaged app reports Ready. A pre-build check prevents publishing a missing or damaged engine bundle.
Windows builds and clean-machine PDF workflow validation are still pending.

The user-reported Whats_Home_Worth_Section.pdf contains an empty AcroForm container
and no fields. It was accepted by the rebuilt Mac app as a three-page PDF after
correcting the overly broad form check. Native file-drop handling and batch
compression are implemented; physical drop gestures and Windows still need validation.
