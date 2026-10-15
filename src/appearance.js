import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';

export function setupAppearance() {
  const control = document.getElementById('appearance');
  const choices = [...control.querySelectorAll('[data-appearance-choice]')];
  const system = window.matchMedia('(prefers-color-scheme: dark)');
  let storage;
  try { storage = window.localStorage; } catch { /* Use the system appearance without storage. */ }
  let preference;
  try { preference = storage?.getItem('pdf-tools-appearance'); } catch { /* Storage can be blocked. */ }
  if (!['light', 'dark', 'system'].includes(preference)) preference = 'system';
  const apply = () => {
    const appearance = preference === 'system' ? (system.matches ? 'dark' : 'light') : preference;
    document.documentElement.dataset.appearance = appearance;
    for (const button of choices) button.setAttribute('aria-pressed', String(button.dataset.appearanceChoice === preference));
    try { getCurrentWindow().setTheme(preference === 'system' ? null : preference).catch(() => {}); }
    catch { /* Browser previews do not have a native window. */ }
    invoke('set_app_icon_appearance', { dark: appearance === 'dark' }).catch(() => {});
  };
  for (const button of choices) button.addEventListener('click', () => {
    preference = button.dataset.appearanceChoice;
    try { storage?.setItem('pdf-tools-appearance', preference); } catch { /* Keep the choice for this session. */ }
    apply();
  });
  system.addEventListener('change', () => { if (preference === 'system') apply(); });
  apply();
}
