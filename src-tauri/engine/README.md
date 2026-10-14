# Bundled PDF engine

Agents receive qpdf with the app; they do not install anything separately.
Use a trusted qpdf 12.1+ native runtime from https://github.com/qpdf/qpdf.

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
