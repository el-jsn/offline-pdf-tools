# Internal 1.0 acceptance check

Scope: company Macs from M1 through M4, using one Apple Silicon build. This host reports arm64 and macOS 27.0 (26A428). Other physical models and the company's other macOS versions were not directly tested. Existing release files and the installed production app were not replaced.

## Processing and recovery

| Check | Evidence |
| --- | --- |
| Difficult credit report | Supplied 51-page report inspected and repaired only in private copies. Merge produced 52 pages, Split 3 selected pages, Organize reversed 51 pages and rotated the first output page. All passed strict output checks. |
| Content preservation | All lossless output pages matched extracted source text and PDFium renders at 0.75 scale, including the intended reorder and rotation. Balanced output retained extracted text across all 51 pages; lossy pixels were not required to match. Source digest stayed unchanged. Private report test copies were removed afterward. |
| Large scan cancellation | Synthetic 144 MB, 20-page scan. Cancelling an active isolated image worker returned cancellation and left the source unchanged; the engine remained usable. Native UI cancellation returned an available toolbar and reported no saved output. |
| Filled form | Synthetic filled AcroForm flattened with the same engine options used by page operations. Field value remained readable and was visually reviewed; original field value remained intact. Live-field rendering and flattened rendering were not pixel-identical around the text. |
| Encryption | Focused import checks accepted a password-free encrypted PDF and kept a supplied-password PDF locked until unlocking. A complete native password-entry/save workflow remains a pilot check. |
| Finder dragging | IT reported a manual pass. The automation could not reliably complete cross-window dragging. Specific insertion, scrolling, and background-drop variants were not separately reported. |
| Original protection | Synthetic input SHA-256 baselines unchanged. Real no-clobber publication refused an existing filename and retained its original bytes. |
| Full destination | Copy to a disposable 32 MB filesystem failed with a disk-space error and removed its staging file. The test refuses to fill the host filesystem. |
| Read-only destination | A separately mounted read-only test filesystem refused staging. |
| Disconnected destination | Captured destination identity, detached only the disposable filesystem, then verified that publication rejects the changed directory. Replacement of a selected directory at the same pathname also passes a focused regression check. |
| Quit and crash recovery | Normal native quit removed its private workspace. Forced termination of only the isolated app left a workspace; relaunch reclaimed it. The other running app's workspace remained intact throughout. |
| Large merged output | A synthetic output larger than the per-input cap is copied successfully for publication while ordinary input copying still enforces its cap. This checks the copying boundary, not a complete large native merge. |

## Changes prompted by this check

- Remember the selected output directory's filesystem and directory identity. Verify it before publication and before returning saved output, so an unmounted drive's leftover mountpoint cannot silently receive the result.
- Apply the 250 MiB import limit to input copies. Allow verified merged outputs to exceed that limit during bookmark rewriting and final saving.
- Add explicit local acceptance checks under `src-tauri/src/acceptance.rs`. Environmental checks are ignored by default and individually invoked; customer documents are never committed as fixtures.

## Packaging

The production frontend and arm64 app built. The app's ad hoc signature passed `codesign --verify --deep --strict`. The existing updater public key verified the new archive and its trusted comment; modifying the archive caused verification to fail.

A separately created local disk image and the normal Tauri disk image both mounted successfully. Their app signatures passed verification. The packaged executable reports arm64; bundled qpdf starts and the resource-directory image worker loads its bundled PDFium library. This proves local disk-image contents, not installation over an agent's existing copy or the live GitHub updater flow. No paid signing or notarization was added.

The ordinary Tauri DMG packaging attempt initially failed inside the filesystem sandbox. Retrying with system disk-image access succeeded, producing the normal DMG, updater archive, and signature. The published `releases/0.1.11` directory was not changed; these are unpublished build checks using the current source version.

## Remaining pilot checks

- Install the final versioned candidate on an agent Mac and complete the supplied-password and representative filled-form workflows through the UI.
- Confirm installation and updating on the company's actual supported macOS versions. M1 through M4 architecture compatibility does not establish those runtime results.
- Have an agent review Compare on representative documents, including scanned pages and form values that its text comparison deliberately excludes.

No version bump, GitHub publication, or installed production-app replacement was performed.
