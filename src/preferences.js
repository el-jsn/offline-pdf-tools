const KEY = 'pdf-tools-preferences-v1';
export function loadPreferences(storage) {
  let value; try { value = JSON.parse(storage?.getItem(KEY) || '{}'); } catch { value = {}; }
  const presets = Array.isArray(value?.presets) ? value.presets.filter((preset) => typeof preset?.name === 'string' && preset.name.trim() && preset.name.length <= 40 && Number.isFinite(preset.mb) && preset.mb >= 0.1 && preset.mb <= 250).slice(0, 12) : [];
  return { thumbnail: [100, 115, 130, 145, 160, 175, 190].includes(value?.thumbnail) ? value.thumbnail : 130,
    compression: ['lossless', 'balanced', 'small'].includes(value?.compression) ? value.compression : 'balanced',
    target: typeof value?.target === 'string' && (value.target === '' || Number(value.target) >= 0.1 && Number(value.target) <= 250) ? value.target : '', presets };
}
export function savePreferences(storage, preferences) {
  try { storage?.setItem(KEY, JSON.stringify(preferences)); } catch { /* Settings are optional. */ }
}
