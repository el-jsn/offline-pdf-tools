import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
const root = fileURLToPath(new URL('../', import.meta.url));
const version = process.argv[2];
if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version ?? '') || version.split('.').some((part) => Number(part) > 65535)) throw new Error('Use a version such as 0.1.2, with each number at most 65535.');
const writes = [];
for (const name of ['package.json', 'package-lock.json', 'src-tauri/tauri.conf.json']) {
  const path = resolve(root, name); const data = JSON.parse(await readFile(path, 'utf8'));
  data.version = version; if (name === 'package-lock.json') data.packages[''].version = version;
  writes.push([path, JSON.stringify(data, null, 2) + '\n']);
}
for (const name of ['src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']) {
  const path = resolve(root, name); const data = await readFile(path, 'utf8');
  const updated = name.endsWith('.toml') ? data.replace(/(\[package\][\s\S]*?\nversion = ")[^"]+("\n)/, (_, prefix, suffix) => prefix + version + suffix) : data.replace(/(name = "offline-pdf-tools"\nversion = ")[^"]+("\n)/, (_, prefix, suffix) => prefix + version + suffix);
  if (updated === data && !data.includes(`version = "${version}"`)) throw new Error(`Version entry missing in ${name}.`);
  writes.push([path, updated]);
}
for (const [path, data] of writes) await writeFile(path, data);
console.log(`Version set to ${version}.`);
