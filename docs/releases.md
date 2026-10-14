# Releases

## Build an installer

On the target operating system, install Node 22.16+, npm, Rust, and Tauri's platform
prerequisites. Stage qpdf 12.1+ with its matching native libraries and license notices:

```sh
python3 scripts/stage-engine.py /absolute/path/to/qpdf --notices /path/to/notices
```

On Windows, use `py` and the path to `qpdf.exe`. The runtime must match the target
architecture. The staging script copies native dependencies; the release build
checks hashes, version, and runtime loading. Users do not need qpdf or Homebrew.

Install dependencies with `npm ci`. Run `npm run release` to build the installer,
signed update artifact, and `latest.json`. Output is in `releases/<version>/`.
Build on the native Mac or Windows architecture; the collection script does not
support cross-target build arguments. The Windows configuration uses a per-user
NSIS installer and includes the offline WebView2 installer. Windows builds and
clean-machine installer validation are still pending.

The private update key lives at `.secrets/updater.key` in this checkout. It is
ignored by Git. Back it up securely and use the same key on every release computer.
Alternatively, set `TAURI_SIGNING_PRIVATE_KEY` to the key file path or its content;
set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if needed. Never commit or upload the key.
Losing or replacing it breaks updates for existing installations.

Update signatures authenticate app updates. They do not replace Apple's Developer
ID signing/notarization or Windows publisher signing. This checkout has no valid
Apple signing identity configured, so its DMG can trigger a first-launch block.

## Publish an update

1. Run `npm run version:set -- 0.1.2`, replacing the version with the next release.
2. Build with `npm run release` on each supported operating system/architecture.
3. Combine their release files in the same version directory. Copy each
   `manifest-<platform>.json` fragment too, then run `node scripts/collect-release.mjs`
   on a computer with that version's local build to regenerate the combined
   `latest.json` and checksums. Never mix fragments from different versions.
4. Create a GitHub release with the exact tag `v<version>`, such as `v0.1.2`.
5. Upload the DMG/Windows installer, Mac `.app.tar.gz` update archive, their `.sig`
   files, and the combined `latest.json`. Optionally include `INSTALL.txt` and
   `SHA256SUMS.txt`. Do not upload `manifest-*.json` or anything in `.secrets`.
6. Publish it as the latest non-prerelease release. Leave previous release assets
   available. Do not replace an already published version's files; bump the version.

Public release assets can be downloaded without employee GitHub accounts. The
code does not publish releases or push commits automatically.

## Update contract

The app checks this URL only when the user clicks Check for updates:

`https://github.com/el-jsn/offline-pdf-tools/releases/latest/download/latest.json`

The JSON follows [Tauri's static updater contract](https://v2.tauri.app/plugin/updater/#static-json-file):
`version` is the released application version. Each `platforms` entry uses a key
such as `darwin-aarch64` or `windows-x86_64`, and contains the HTTPS artifact `url`
and the complete `signature` string from the generated `.sig` file. A platform is
included only when its artifact has been built. Artifact URLs use the immutable
`/releases/download/v<version>/` path.

When a newer version is available, the app offers Install. A native dialog confirms
the restart and loss of unsaved selections. Installation is blocked during PDF
operations. The updater verifies the signed download before installing and then
restarts the app. There are no background checks or PDF uploads.

The first published release is necessary before the live update endpoint works.
The updater has been compiled into the Mac app, but a published version-to-version
update and a Windows update have not yet been verified.
