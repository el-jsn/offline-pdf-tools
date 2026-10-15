import { createHash } from 'node:crypto';
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const directory = join(root, 'src-tauri', 'engine');
const executable = join(directory, process.platform === 'win32' ? 'qpdf.exe' : 'qpdf');
try {
  if (!existsSync(executable)) throw new Error('Missing bundled qpdf. Stage the native runtime with scripts/stage-engine.py before building.');
  const manifest = JSON.parse(readFileSync(join(directory, 'manifest.json'), 'utf8'));
  if (manifest.platform !== process.platform) throw new Error('The bundled engine belongs to another platform. Stage a runtime for this machine.');
  if (!manifest.files || !Object.keys(manifest.files).some((name) => name.startsWith('licenses/'))) throw new Error('The engine bundle is missing its license notices.');
  for (const [name, expected] of Object.entries(manifest.files)) {
    const path = resolve(directory, name);
    if (!path.startsWith(directory + sep)) throw new Error('Invalid file path in engine manifest.');
    const actual = createHash('sha256').update(readFileSync(path)).digest('hex');
    if (actual !== expected) throw new Error(`Bundled engine file changed: ${name}. Restage the runtime.`);
  }
  const version = execFileSync(executable, ['--version'], { encoding: 'utf8', timeout: 10000, windowsHide: true });
  const match = version.match(/qpdf version (\d+)\.(\d+)/);
  if (!match || Number(match[1]) < 12 || (Number(match[1]) === 12 && Number(match[2]) < 1)) throw new Error('Bundled qpdf must be version 12.1 or newer.');
  const compressorName = process.platform === 'win32' ? 'compress-pdf.exe' : 'compress-pdf';
  const compressor = join(directory, compressorName);
  if (manifest.compressor?.version !== '0.1.0' || !manifest.files[compressorName] || !existsSync(compressor)
      || !Object.keys(manifest.files).some((name) => name.startsWith('licenses/compress-pdf/'))) {
    throw new Error('Missing bundled compress-pdf 0.1.0 or its notices. Run scripts/stage-compressor.py before building.');
  }
  const compressorVersion = execFileSync(compressor, ['--version'], { encoding: 'utf8', timeout: 10000, windowsHide: true }).trim();
  if (compressorVersion !== 'compress-pdf 0.1.0') throw new Error('Unexpected bundled compressor version. Restage compress-pdf 0.1.0.');
  const worker = join(directory, process.platform === 'win32' ? 'pdf-image-worker.exe' : 'pdf-image-worker');
  const pdfium = join(directory, process.platform === 'win32' ? 'pdfium.dll' : 'libpdfium.dylib');
  if (!manifest.pdfium || !existsSync(worker) || !existsSync(pdfium)) throw new Error('Missing PDFium image runtime or worker.');
  execFileSync(worker, ['--check-runtime', pdfium], { encoding: 'utf8', timeout: 10000, windowsHide: true });
  const repairName = process.platform === 'win32' ? 'pypdf-repair.exe' : 'pypdf-repair';
  const repair = join(directory, 'pypdf-repair', repairName);
  const repairSource = createHash('sha256').update(readFileSync(join(root, 'scripts/pypdf-repair.py'))).digest('hex');
  if (manifest.pypdf?.version !== '6.20.1' || manifest.pypdf?.worker_sha256 !== repairSource
      || !manifest.files[`pypdf-repair/${repairName}`]
      || !Object.keys(manifest.files).some((name) => name.startsWith('pypdf-repair/licenses/pypdf/'))
      || !manifest.files['pypdf-repair/licenses/Python-LICENSE.txt']
      || !Object.keys(manifest.files).some((name) => name.startsWith('pypdf-repair/licenses/python-native/'))) {
    throw new Error('Missing or stale pypdf repair runtime/notices. Run scripts/build-pypdf-worker.py with the pinned build environment.');
  }
  const repairVersion = execFileSync(repair, ['--version'], { encoding: 'utf8', timeout: 30000, windowsHide: true }).trim();
  if (repairVersion !== 'pypdf-repair 6.20.1') throw new Error('Unexpected bundled repair worker version.');
  console.log('Bundled PDF, compression, image and repair engines verified.');
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
