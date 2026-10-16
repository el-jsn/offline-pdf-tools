# PDF Tools

Merge, compress, split, organize, and compare PDFs on your computer. PDF processing and previews work locally; original files are preserved.

[Download for Apple Silicon Mac](https://github.com/el-jsn/offline-pdf-tools/releases/latest) | [User guide](docs/usage.md) | [Build from source](docs/building.md)

## Install

Open the DMG and drag PDF Tools to Applications. Use **Check for updates** in the app for later releases. No developer tools or GitHub account are needed.

Current installers are not Apple Developer ID signed or notarized, so macOS security warnings may appear. Windows packaging is configured, but a Windows release has not been built or validated yet.

## What it does

- **Merge:** combine PDFs and JPEG, PNG, or WebP images.
- **Compress:** choose Original quality, Balanced, or Smaller; review the output size and compare pages before saving.
- **Split:** extract selected pages or split by groups, bookmarks, page count, or file size.
- **Organize:** reorder, rotate, duplicate, or remove pages, with previews and undo/redo.
- **Compare:** review changes in extracted text and saved form values between two PDFs.

Balanced and Smaller use **compress-pdf 0.1.0**; Original quality uses **qpdf**. The lossy presets can change image and font appearance, and size targets are not guaranteed. [Compression details and processing limits](docs/usage.md#compression-engine).

## Screenshots

### Compress PDF

This brochure went from 51.09 MB to 11.85 MB with Balanced. Results depend on the document and settings.

![Compression result showing a 51.09 MB brochure reduced to 11.85 MB using Balanced](https://github.com/user-attachments/assets/621ecdf0-72d0-4342-b9f4-4ad60b673c7e)

<details>
<summary>Organize pages</summary>

Select, rotate, duplicate, and reorder pages before saving a new PDF.

![Page organizer showing four sample pages with page 2 selected](https://github.com/user-attachments/assets/0f2ea8f5-08c2-4728-b2d6-759ec2362031)

</details>

<details>
<summary>Split PDF</summary>

Choose page ranges or select thumbnails to extract the pages you need.

![Split PDF with pages 1 and 2 selected for extraction](https://github.com/user-attachments/assets/694f9e47-3a41-43ea-8e7c-4f68c94fa45c)

</details>

<details>
<summary>Compression in progress</summary>

![PDF Tools processing a brochure locally with the Balanced compression setting](https://github.com/user-attachments/assets/2096082a-2bcd-4232-8f77-bf10de931c87)

</details>

## Documentation

- [User guide, compression behavior, and processing limits](docs/usage.md)
- [Build from source](docs/building.md)
- [Packaging, engine staging, and releases](docs/releases.md)

Update checks and installer downloads contact GitHub when requested. Document processing stays local.
