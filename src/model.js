export const MAX_OUTPUTS = 500;
export function parseRange(value, total) {
  if (!value.trim()) throw new Error('Enter a page range.');
  const pages = [];
  for (const segment of value.split(',')) {
    const match = segment.trim().match(/^(\d+)(?:\s*-\s*(\d+))?$/);
    if (!match) throw new Error('Use page numbers or ranges such as 1-3,5.');
    const first = Number(match[1]); const last = Number(match[2] || match[1]);
    if (!Number.isSafeInteger(first) || !Number.isSafeInteger(last) || first < 1 || last < first || last > total) throw new Error(`Use page numbers between 1 and ${total}, with ranges in ascending order.`);
    if (pages.length + last - first + 1 > 10000) throw new Error('Choose at most 10,000 pages per output.');
    for (let page = first; page <= last; page += 1) pages.push(page);
  }
  return pages;
}
export function pageList(pages) {
  const chunks = [];
  for (let index = 0; index < pages.length; index += 1) {
    const first = pages[index]; let last = first;
    while (index + 1 < pages.length && pages[index + 1] === last + 1) last = pages[++index];
    chunks.push(first === last ? String(first) : `${first}-${last}`);
  }
  return chunks.join(', ');
}
export function splitPlan(mode, size, ranges, total, { separate = false, combine = false } = {}) {
  let plan;
  if (ranges.length > 20000) throw new Error('The page range list is too long.');
  if (mode === 'selected') {
    const pages = parseRange(ranges, total);
    plan = separate ? pages.map((page) => [page]) : [pages];
  } else if (mode === 'after') {
    const cuts = [...new Set(parseRange(ranges, total))].sort((a, b) => a - b);
    let start = 1; plan = [];
    for (const end of cuts) {
      plan.push(Array.from({ length: end - start + 1 }, (_, index) => start + index)); start = end + 1;
    }
    if (start <= total) plan.push(Array.from({ length: total - start + 1 }, (_, index) => start + index));
  } else if (mode === 'ranges') {
    const chunks = ranges.split(';');
    if (chunks.length > MAX_OUTPUTS) throw new Error(`Create at most ${MAX_OUTPUTS} groups at once.`);
    plan = chunks.map((range) => parseRange(range, total));
    if (combine) plan = [plan.flat()];
  } else if (mode === 'every_page' || mode === 'groups') {
    const count = mode === 'every_page' ? 1 : Number(size);
    if (!Number.isSafeInteger(count) || count < 1 || count > total) throw new Error(`Use a group size between 1 and ${total}.`);
    if (Math.ceil(total / count) > MAX_OUTPUTS) throw new Error(`Create at most ${MAX_OUTPUTS} PDFs at once. Choose a larger group size.`);
    plan = [];
    for (let start = 1; start <= total; start += count) plan.push(Array.from({ length: Math.min(count, total - start + 1) }, (_, index) => start + index));
  } else throw new Error('Choose a split method.');
  if (plan.length > MAX_OUTPUTS) throw new Error(`Create at most ${MAX_OUTPUTS} PDFs at once.`);
  if (plan.some((pages) => pages.length > 10000)) throw new Error('Choose at most 10,000 pages per output.');
  if (plan.reduce((count, pages) => count + pages.length, 0) > 20000) throw new Error('Choose at most 20,000 output pages in total.');
  return plan;
}

export function movePagesTo(pages, selected, position) {
  const moving = pages.filter((page) => selected.has(page.page));
  if (!moving.length) return pages;
  const remaining = pages.filter((page) => !selected.has(page.page));
  if (!Number.isInteger(position) || position < 1 || position > remaining.length + 1) throw new Error(`Choose a position between 1 and ${remaining.length + 1}.`);
  return [...remaining.slice(0, position - 1), ...moving, ...remaining.slice(position - 1)];
}
export function movePagesBeside(pages, selected, targetPage, after = false) {
  if (selected.has(targetPage)) return pages;
  const remaining = pages.filter((page) => !selected.has(page.page));
  const index = remaining.findIndex((page) => page.page === targetPage);
  if (index < 0) return pages;
  return movePagesTo(pages, selected, index + 1 + Number(after));
}
