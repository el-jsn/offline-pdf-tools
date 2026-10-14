import { readFile, writeFile, mkdir, readdir, copyFile, access } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const repo = 'https://github.com/el-jsn/offline-pdf-tools';
const architectures = { arm64: 'aarch64', x64: 'x86_64', ia32: 'i686' };
export async function collect() {
  const config = JSON.parse(await readFile(resolve(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  if (!['darwin', 'win32'].includes(process.platform) || !architectures[process.arch]) throw new Error('Build releases on a supported Mac or Windows computer.');
  const os = process.platform === 'darwin' ? 'macos' : 'windows';
  const target = `${os === 'macos' ? 'darwin' : 'windows'}-${architectures[process.arch]}`;
  const output = resolve(root, 'releases', config.version);
  const bundle = resolve(root, 'src-tauri/target/release/bundle');
  const label = `PDF-Tools_${config.version}_${os}-${process.arch}`;
  const artifacts = [];
  let updateSource; let updateName;
  if (os === 'macos') {
    const arch = process.arch === 'arm64' ? 'aarch64' : 'x64';
    artifacts.push([resolve(bundle, 'dmg', `${config.productName}_${config.version}_${arch}.dmg`), `${label}.dmg`]);
    updateSource = resolve(bundle, 'macos', `${config.productName}.app.tar.gz`);
    updateName = `${label}.app.tar.gz`;
  } else {
    const arch = { x64: 'x64', arm64: 'arm64', ia32: 'x86' }[process.arch];
    updateSource = resolve(bundle, 'nsis', `${config.productName}_${config.version}_${arch}-setup.exe`);
    updateName = `${label}-setup.exe`;
  }
  artifacts.push([updateSource, updateName], [`${updateSource}.sig`, `${updateName}.sig`]);
  for (const [source] of artifacts) await access(source);
  await mkdir(output, { recursive: true });
  for (const [source, name] of artifacts) await copyFile(source, resolve(output, name));
  const signature = (await readFile(`${updateSource}.sig`, 'utf8')).trim();
  if (!signature) throw new Error('Missing update signature.');
  const platform = { url: `${repo}/releases/download/v${config.version}/${updateName}`, signature };
  await writeFile(resolve(output, `manifest-${target}.json`), JSON.stringify({ version: config.version, platforms: { [target]: platform } }, null, 2) + '\n');
  const platforms = {};
  for (const name of await readdir(output)) {
    if (!/^manifest-(darwin|windows)-(aarch64|x86_64|i686)\.json$/.test(name)) continue;
    const fragment = JSON.parse(await readFile(resolve(output, name), 'utf8'));
    if (fragment.version !== config.version) throw new Error('Do not combine manifests from different versions.');
    Object.assign(platforms, fragment.platforms);
  }
  await writeFile(resolve(output, 'latest.json'), JSON.stringify({ version: config.version, pub_date: new Date().toISOString(), platforms }, null, 2) + '\n');
  const checksums = [];
  for (const name of (await readdir(output)).sort()) {
    if (!/\.(dmg|exe|gz|sig)$/.test(name)) continue;
    const hash = createHash('sha256');
    for await (const chunk of createReadStream(resolve(output, name))) hash.update(chunk);
    checksums.push(`${hash.digest('hex')}  ${name}`);
  }
  await writeFile(resolve(output, 'SHA256SUMS.txt'), checksums.join('\n') + '\n');
  await writeFile(resolve(output, 'INSTALL.txt'), `PDF Tools ${config.version}\n\nMac: open the matching DMG, then drag PDF Tools to Applications.\nWindows: run the matching setup.exe.\nUpdates: close the app and install the newer installer, or use Check for updates inside the app once IT has published the release.\nNo GitHub account or developer tools are required for public release downloads.\n\nThis build is not Apple Developer ID signed/notarized or Windows publisher signed. Operating system security warnings can appear.\n`);
  console.log(`Release files: ${output}`);
  console.log(`Publish tag v${config.version} with installers, update archive(s), and latest.json. Do not upload manifest-*.json or signing keys.`);
}
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) await collect();
