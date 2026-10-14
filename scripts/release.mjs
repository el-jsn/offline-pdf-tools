import { access } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { collect } from './collect-release.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const key = process.env.TAURI_SIGNING_PRIVATE_KEY || resolve(root, '.secrets/updater.key');
if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  try { await access(key); } catch {
    throw new Error('Update signing key missing. Restore .secrets/updater.key from your secure backup or set TAURI_SIGNING_PRIVATE_KEY. Do not generate a replacement key for an already released app.');
  }
}
const child = spawn(process.execPath, [resolve(root, 'node_modules/@tauri-apps/cli/tauri.js'), 'build', ...process.argv.slice(2)], {
  cwd: root, stdio: 'inherit', env: {
    ...process.env, TAURI_SIGNING_PRIVATE_KEY: key,
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? '',
  },
});
const code = await new Promise((resolve, reject) => {
  child.once('error', reject); child.once('exit', (code, signal) => resolve(signal ? 1 : code ?? 1));
});
if (code !== 0) process.exit(code);
await collect();
