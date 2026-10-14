export const UPDATE_CHECK_INTERVAL = 24 * 60 * 60 * 1000;
const KEY = 'pdf-tools-update-status';
export function newerVersion(candidate, installed) {
  const parse = (value) => /^\d+\.\d+\.\d+$/.test(value || '') ? value.split('.').map(Number) : null;
  const next = parse(candidate); const current = parse(installed);
  if (!next || !current) return false;
  for (let index = 0; index < 3; index++) if (next[index] !== current[index]) return next[index] > current[index];
  return false;
}
export class UpdateChecks {
  constructor(storage, now = () => Date.now()) { this.storage = storage; this.now = now; this.record = {}; try { const record = JSON.parse(storage?.getItem(KEY) || '{}'); this.record = record && typeof record === 'object' && !Array.isArray(record) ? record : {}; } catch { /* Storage is optional. */ } }
  save() { try { this.storage?.setItem(KEY, JSON.stringify(this.record)); } catch { /* Keep the in-session state when storage is unavailable. */ } }
  claim() {
    const now = this.now(); const previous = this.record.checkedAt;
    if (Number.isFinite(previous) && previous > 0 && now - previous < UPDATE_CHECK_INTERVAL && now >= previous) return false;
    this.record.checkedAt = now; this.save(); return true;
  }
  remember(version) { this.record.checkedAt = this.now(); this.record.availableVersion = version || null; this.save(); }
  available(installed) { return newerVersion(this.record.availableVersion, installed) ? this.record.availableVersion : null; }
}
