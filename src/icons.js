const shapes = {
  merge: '<rect x="3" y="3" width="12" height="16" rx="2"/><path d="M9 5h10a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H9M7 9h4M7 13h4"/>',
  compress: '<rect x="5" y="3" width="14" height="18" rx="2"/><path d="m9 8 3 3 3-3M12 5v6m-3 5 3-3 3 3M12 19v-6"/>',
  split: '<path d="M12 3v7M12 10 5 17m7-7 7 7M3 15v5h5m8 0h5v-5"/>',
  organize: '<rect x="3" y="3" width="7" height="8" rx="1.5"/><rect x="14" y="3" width="7" height="8" rx="1.5"/><rect x="3" y="15" width="7" height="6" rx="1.5"/><rect x="14" y="15" width="7" height="6" rx="1.5"/>',
  add: '<path d="M12 5v14M5 12h14"/>',
  document: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6M8 13h8M8 17h5"/>',
  lock: '<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3M12 14v3"/>',
  trash: '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/>',
  up: '<path d="m6 14 6-6 6 6"/>', down: '<path d="m6 10 6 6 6-6"/>',
  left: '<path d="m14 6-6 6 6 6"/>', right: '<path d="m10 6 6 6-6 6"/>',
  rotate: '<path d="M20 11a8 8 0 1 1-2.4-5.7M20 3v6h-6"/>', undo: '<path d="M3 9h11a6 6 0 0 1 0 12M3 9l5-5M3 9l5 5"/>',
  check: '<path d="m5 12 4 4L19 6"/>', sidebar: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>',
  grip: '<path d="M9 5h.01M15 5h.01M9 12h.01M15 12h.01M9 19h.01M15 19h.01" stroke-width="3"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
};
export function icon(name) {
  const wrapper = document.createElement('span'); wrapper.className = 'icon'; wrapper.setAttribute('aria-hidden', 'true');
  // All markup is a fixed developer-owned SVG definition; never document data.
  wrapper.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">${shapes[name] || shapes.document}</svg>`;
  return wrapper;
}
