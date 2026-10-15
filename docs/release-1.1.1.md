# PDF Tools 1.1.1

- Fixed size targets producing larger PDFs than the selected compression preset. Compression now runs the preset first and only tries additional settings if needed, keeping the smallest valid result.
- Renamed Limit per PDF to Target per PDF and clarified that it is optional and best effort.
- Added percentage savings and an indication of whether compressed PDFs meet the target.
- Added a subtle activity bar during compression and a short result reveal, with Reduce Motion support.

[Download for Apple Silicon Macs](https://github.com/el-jsn/offline-pdf-tools/releases/download/v1.1.1/PDF-Tools_1.1.1_macos-arm64.dmg), or choose **Check for updates**.

This release includes the Apple Silicon Mac installer and signed in-app update. The Mac app uses the same ad-hoc signing as previous releases; it is not Apple notarized.
