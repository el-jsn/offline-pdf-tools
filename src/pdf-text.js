// PDF.js may split a word across font runs. Use baseline spacing rather than
// inserting a space between every extracted item, which creates false changes.
export function pageText(items) {
  let result = '', previous = null;
  for (const item of items) {
    if (typeof item.str !== 'string' || !item.str) continue;
    if (previous && !/\s$/.test(result) && !/^\s/.test(item.str)) {
      const [a, b, , , x, y] = previous.transform;
      const length = Math.hypot(a, b) || 1;
      const dx = item.transform[4] - x, dy = item.transform[5] - y;
      const gap = (dx * a + dy * b) / length - previous.width;
      const lineOffset = Math.abs((dy * a - dx * b) / length);
      if (previous.hasEOL || item.dir !== 'ltr' || previous.dir !== 'ltr' || lineOffset > length * 0.5 || gap > length * 0.15) result += ' ';
    }
    result += item.str; previous = item;
  }
  return result;
}
