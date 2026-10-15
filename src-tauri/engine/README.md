# Bundled PDF engines

Agents receive qpdf with the app; they do not install anything separately.
Use a trusted qpdf 12.1+ native runtime from https://github.com/qpdf/qpdf.

Balanced and Smaller compression use the separately bundled compress-pdf 0.1.0
executable. Stage a native build with scripts/stage-compressor.py, supplying the
MIT license and all Rust/native dependency notices. The staging script signs the
Mac executable, rejects non-system Mac library dependencies, and records hashes.
scripts/check-engine.mjs requires this runtime and its notices before packaging.
The compressor is built during developer setup, never downloaded at runtime.

Use scripts/stage-engine.py on the target OS and architecture. Supply the exact
runtime licenses and third-party notices through its --notices argument. It stages:

- macOS: qpdf and its dependent dylibs, with local loader paths and ad hoc signing.
- Windows: qpdf.exe and its neighboring DLLs.
- Both: a licenses folder and a SHA-256 manifest.

Mac staging has been executed and the packaged engine runs without Homebrew on
PATH. Windows staging and clean-machine verification remain pending, including
DLL resolution on Windows and minimum OS compatibility on Mac.
Ad hoc signatures are not Developer ID signing or Apple notarization.

There is no runtime download, PATH search, or remote processing fallback.
PDF_TOOLS_QPDF provides an absolute development executable override in debug
builds only. Release builds use the bundled engine exclusively.

Do not distribute a build with only this README in the engine directory.

The guarded AcroForm fallback uses pypdf 6.20.1 (BSD-3-Clause), frozen with
Python 3.12 and PyInstaller 6.22.3 into engine/pypdf-repair/. Users do not need
Python. On each target OS/architecture, create a clean Python 3.12 venv, install
scripts/pypdf-build-requirements.txt during developer setup, then run that venv's
Python with scripts/build-pypdf-worker.py. The build script does not download.
It stages Python, package and native dependency notices and hashes every file.
The engine check also rejects a worker built from stale source.

This fallback runs only when qpdf's existing repair leaves warnings. It supports
flat, uniquely named page widgets with missing field-tree connections. Conflicting
values/appearances, repeated names, nested fields, XFA and signed documents are
rejected. It checks decoded page content, resources, annotations and form
appearances, followed by a clean qpdf check and page-count/field-tree checks.
It does not regenerate appearances or perform image compression. Windows runtime
packaging and clean-machine verification remain pending.
