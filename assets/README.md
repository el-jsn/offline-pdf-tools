# App icon

Generated with the built-in image-generation tool. Both sources retain alpha transparency:

- `app-icon-light.png`: blue document mark on a light tile; the bundled app icon.
- `app-icon-dark.png`: the same mark in pale blue on a charcoal tile.

The running Mac app selects its Dock icon from the existing Appearance setting. System follows the resolved system appearance. Finder and the app before launch use the bundled light icon. AppKit supports changing the running app's Dock icon through [applicationIconImage](https://developer.apple.com/documentation/appkit/nsapplication/applicationiconimage).

Native sizes and ICNS/ICO files are generated with the installed Tauri CLI. Generate into a temporary directory, then copy only the desktop outputs into `src-tauri/icons`; mobile/Appx outputs are not used. `icon.png` and `icon-dark.png` are the embedded runtime images. The other desktop formats use the light design.

## Light prompt

Use case: logo-brand. Create ONE finished macOS app icon for PDF Tools, a precise professional document utility. This must be a genuinely different concept from the commonplace blue square with stacked white sheets: DO NOT draw stacked pages, DO NOT draw horizontal text strokes. Design direction: a soft white / very pale cool-gray rounded-square macOS tile, centered on an actually transparent square canvas with the standard roughly 10 percent exterior icon margin. On the tile, one striking cobalt-blue geometric document symbol composed of THREE solid interlocking pieces. Together they form one upright document with a folded upper-right corner; two crisp narrow negative-space seams distinguish the parts and suggest splitting, combining and arranging. Abstract graphic mark, not an illustration of writing. Strong large simple shapes, deliberate balanced geometry, confident solid blue with one restrained slightly lighter blue fold, excellent legibility at 32 pixels. Flat near-vector visual style, just a tiny natural tile edge shadow for macOS polish, no shiny effects. Centered front-on, no perspective. Very clean, minimal, distinctive, grown-up productivity app. No letters, no PDF label, no typography, no words, no numbers, no watermark, no badges, no scissors, no arrows, no tools, no cloud, no lock, no multi-icon concept sheet, no presentation layout, no gradient-filled blue background. Only the final single usable app-icon asset.

## Dark edit prompt

Reference: the approved light icon above. Use case: identity-preserve. The attached image is the APPROVED LIGHT VERSION of the PDF Tools macOS app icon. Create its DARK MODE counterpart. Preserve EXACTLY the centered interlocking document mark, all internal seams, the folded corner, shape proportions, layout, tile geometry and transparent exterior margins. Change only the color treatment: rounded-square tile becomes a very dark charcoal / cool graphite (#20242b), the main document mark becomes vivid light azure blue (#63aaff), and the folded-corner piece becomes a slightly paler blue (#a0ceff). Keep strong contrast and a restrained professional macOS utility look. Very subtle edge depth only; no glow, no grain, no texture, no speckles, no rough border. Keep actual alpha transparency outside the tile. No additional objects, no new shapes, no text, no labels, no scene, no mockup. Deliver exactly one finished dark-version icon with the same square canvas and same alignment as the approved light icon.
