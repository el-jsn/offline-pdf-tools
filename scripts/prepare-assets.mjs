import { cp, mkdir } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
const require = createRequire(import.meta.url);
const root = dirname(require.resolve('pdfjs-dist/package.json'));
const destination = resolve('public/pdfjs');
await mkdir(destination, { recursive: true });
for (const folder of ['cmaps', 'iccs', 'standard_fonts', 'wasm']) await cp(join(root, folder), join(destination, folder), { recursive: true });
await cp(join(root, 'LICENSE'), join(destination, 'LICENSE'));
