import { getCurrentWindow } from '@tauri-apps/api/window';

export function setupAppearance() {
  const control = document.getElementById('appearance');
  const system = window.matchMedia('(prefers-color-scheme: dark)');
  let storage;
  try { storage = window.localStorage; } catch { /* Use the system appearance without storage. */ }
  let preference;
  try { preference = storage?.getItem('pdf-tools-appearance'); } catch { /* Storage can be blocked. */ }
  if (!['light', 'dark', 'system'].includes(preference)) preference = 'system';
  const apply = () => {
    document.documentElement.dataset.appearance = preference === 'system' ? (system.matches ? 'dark' : 'light') : preference;
    control.value = preference;
    try { getCurrentWindow().setTheme(preference === 'system' ? null : preference).catch(() => {}); }
    catch { /* Browser previews do not have a native window. */ }
  };
  control.addEventListener('change', () => {
    preference = control.value;
    try { storage?.setItem('pdf-tools-appearance', preference); } catch { /* Keep the choice for this session. */ }
    apply();
  });
  system.addEventListener('change', () => { if (preference === 'system') apply(); });
  apply();
}
