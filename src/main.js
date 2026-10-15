import { invoke, Channel } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { reorderable } from './reorder.js';
import { pagesFromDocuments, insertPages, insertionAtPoint } from './organize-import.js';
import { icon } from './icons.js';
import { Previews } from './previews.js';
import { splitPlan, parseRange, pageList, movePagesTo, movePagesBeside, bookmarkGroups, pageKey } from './model.js';
import { UpdateChecks } from './updates.js';
import './styles.css';
import { setupAppearance } from './appearance.js';

setupAppearance();

const $ = (id) => document.getElementById(id);
const bytes = (value) => value >= 1048576 ? `${(value / 1048576).toFixed(2)} MB` : `${(value / 1024).toFixed(1)} KB`;
const tools = {
  merge: { heading: 'Merge PDFs', action: 'Merge PDFs' },
  compress: { heading: 'Compress PDF', action: 'Compress PDF' },
  split: { heading: 'Split PDF', action: 'Split PDF' },
  organize: { heading: 'Organize pages', action: 'Save PDF' },
};
const state = Object.fromEntries(Object.keys(tools).map((tool) => [tool, {
  documents: [], importErrors: [], pages: [], selected: new Set(), history: [], future: [], pageWindow: 0,
  compressionPreview: null, measurementKey: null, measurementFailed: false, compression: 'balanced', targetSize: '', splitMode: 'selected', splitSize: '10', groupSize: '5', ranges: '', splitGroups: [''], splitNames: [''], activeGroup: 0, splitSeparate: false, splitCombine: false, splitCuts: '', flattenForms: false, acknowledgeChanges: false, result: null,
}]));
let mode = 'merge'; let busy = false; let ready = false; let jobId = null; let cancelAvailable = false; let lastSelected = null;
let compressionTimer = null;
let previewSession = null;
let previewRenderSequence = 0;
let pendingUnlock = null;
let duplicateKey = 0;
let pendingUpdate = null; let updateChecking = false; let availableUpdateVersion = null;
let updateStorage; try { updateStorage = window.localStorage; } catch { /* Storage may be unavailable. */ }
const updateChecks = new UpdateChecks(updateStorage);
const PAGE_WINDOW = 48;
const current = () => state[mode];
const previews = new Previews();
for (const element of document.querySelectorAll('[data-icon]')) element.replaceWith(icon(element.dataset.icon));
function message(text, type = '') { $('status').textContent = text; $('status').className = type; }
function outputDocuments(view) {
  if (mode !== 'organize') return view.documents;
  const used = new Set(view.pages.map((page) => page.documentId || view.documents[0]?.id));
  return view.documents.filter((doc) => used.has(doc.id));
}
function changesFor(view) {
  const pageOperation = mode !== 'compress';
  const docs = outputDocuments(view).filter((doc) => !doc.locked);
  const any = (test) => docs.some((doc) => test(doc.features));
  const blocked = any((features) => features.unsupportedForms) ? 'XFA forms are unsupported. Print or export a flattened copy, then add it again.'
    : pageOperation && any((features) => features.needsAppearances) ? 'A filled form must be saved in its original application before it can be changed.' : null;
  const flatten = !blocked && pageOperation && any((features) => features.forms);
  const acknowledge = !blocked && (any((features) => features.signatures) || (pageOperation && any((features) => features.tags)));
  const notes = [];
  if (flatten) notes.push('Forms and annotations will become static.');
  if (any((features) => features.signatures)) notes.push('Signatures will be removed in the saved copy.');
  if (pageOperation && any((features) => features.tags)) notes.push('Page changes may remove accessibility tags.');
  return { blocked, flatten, acknowledge, notes };
}
function targetBytes(view) {
  const mb = Number.parseFloat(view.targetSize);
  if (!Number.isFinite(mb) || view.compression === 'lossless') return null;
  const limit = Math.round(mb * 1048576);
  return limit >= 100000 && limit <= 250 * 1048576 ? limit : null;
}
function validation() {
  const view = current();
  if (outputDocuments(view).some((doc) => doc.locked)) return false;
  const changes = changesFor(view);
  if (changes.blocked) return false;
  if (changes.flatten && !view.flattenForms) return false;
  if (changes.acknowledge && !view.acknowledgeChanges) return false;
  if (mode === 'merge') return view.documents.length >= 2 && view.documents.reduce((sum, doc) => sum + doc.pages, 0) <= 20000;
  if (mode === 'compress') return view.documents.length > 0 && !(view.targetSize.trim() && targetBytes(view) === null);
  if (mode === 'organize') return view.pages.length > 0 && view.pages.length <= 20000;
  if (view.documents.length !== 1) return false;
  if (mode === 'split') { try { splitPlanFor(view); } catch { return false; } }
  return true;
}
function controls() {
  const view = current();
  $('save').disabled = busy || !ready || !validation() || (mode === 'compress' && !view.compressionPreview && !view.measurementFailed); $('clear').disabled = busy;
  $('app-update').disabled = busy || updateChecking;
  $('app-update').classList.toggle('update-available', Boolean(availableUpdateVersion));
  $('app-update').textContent = availableUpdateVersion ? 'Update available' : 'Check for updates';
  $('app-update').title = availableUpdateVersion ? `Install version ${availableUpdateVersion}` : '';
  $('app-update').setAttribute('aria-label', availableUpdateVersion ? `Update available. Install version ${availableUpdateVersion}` : 'Check for updates');
  $('release-notes').disabled = busy;
  $('add').disabled = busy || !ready; $('choose').disabled = busy || !ready;
  $('busy-indicator').hidden = !busy; $('loaded').setAttribute('aria-busy', String(busy));
  $('cancel').hidden = !busy || !cancelAvailable; $('reveal').hidden = busy || !view.result;
  document.querySelectorAll('.tool, #compression-options input, #split-options input, #split-options select, #split-options button').forEach((control) => { control.disabled = busy; });
  $('invert-selection').disabled = busy || !view.pages.length;
  $('add-split-group').disabled = busy || view.splitGroups.length >= 500;
  $('rotate-left').disabled = busy || !view.selected.size;
  $('move-page-position').disabled = busy || !view.selected.size; $('move-pages-to').disabled = busy || !view.selected.size;
  $('move-page-position').max = String(Math.max(1, view.pages.length - view.selected.size + 1));
  $('rotate').disabled = busy || !view.selected.size;
  $('duplicate-pages').disabled = busy || !view.selected.size;
  $('delete-pages').disabled = busy || !view.selected.size || view.selected.size === view.pages.length;
  $('move-left').disabled = busy || !view.pages.some((page, index) => index > 0 && view.selected.has(pageKey(page)) && !view.selected.has(pageKey(view.pages[index - 1])));
  $('move-right').disabled = busy || !view.pages.some((page, index) => index < view.pages.length - 1 && view.selected.has(pageKey(page)) && !view.selected.has(pageKey(view.pages[index + 1])));
  $('undo').disabled = busy || !view.history.length; $('redo').disabled = busy || !view.future.length; $('select-all').disabled = busy || !view.pages.length;
  $('previous-pages').disabled = busy || view.pageWindow === 0; $('next-pages').disabled = busy || (view.pageWindow + 1) * PAGE_WINDOW >= view.pages.length;
  $('jump-page').disabled = busy || !view.pages.length; $('thumbnail-size').disabled = busy;
  $('compare-output').disabled = busy || !state.compress.compressionPreview || state.compress.documents.length !== 1;
  $('target-size').disabled = busy || view.compression === 'lossless';
  if (view.targetSize.trim() && targetBytes(view) === null) $('target-size').setAttribute('aria-invalid', 'true'); else $('target-size').removeAttribute('aria-invalid');
  $('password-unlock').disabled = busy; $('password-skip').disabled = busy; $('pdf-password').disabled = busy;
  $('flatten-forms').disabled = busy; $('acknowledge-changes').disabled = busy;
  for (const control of document.querySelectorAll('.file-actions button, .page-actions button')) control.disabled = busy || control.dataset.boundary === 'true';
  $('save-label').textContent = mode === 'split' && view.splitMode === 'selected' ? 'Extract pages' : mode === 'compress' && view.compressionPreview ? (view.documents.length > 1 ? 'Save PDFs' : 'Save PDF') : mode === 'compress' && view.documents.length > 1 ? 'Compress PDFs' : tools[mode].action;
  compressionSummary();
  $('add-label').textContent = ['merge', 'organize'].includes(mode) ? 'Add files' : mode === 'compress' ? 'Add PDFs' : view.documents.length ? 'Replace PDF' : 'Add PDF';
}
function actionButton(label, symbol, action, disabled = false) {
  const element = document.createElement('button'); element.className = 'icon-button'; element.title = label;
  element.setAttribute('aria-label', label); element.dataset.boundary = String(disabled); element.disabled = busy || disabled;
  element.append(icon(symbol)); element.addEventListener('click', action); return element;
}
function changeOrder(from, to) {
  if (busy || from === to || to < 0 || to >= current().documents.length) return;
  current().documents.splice(to, 0, current().documents.splice(from, 1)[0]); current().result = null; render(); message('Document order updated.');
}
function renderDocuments() {
  $('documents').replaceChildren();
  current().documents.forEach((doc, index) => {
    const row = document.createElement('li'); row.className = 'document';
    if (mode === 'merge') {
      const grip = icon('grip'); row.append(grip);
      reorderable(row, index, grip, () => busy, changeOrder);
    }
    const cover = document.createElement('div'); cover.className = 'cover'; cover.append(icon('document'));
    if (!doc.locked) previews.watch(cover, doc.id, 1, 0, doc.previewBytes);
    const info = document.createElement('div'); info.className = 'file-info';
    const name = document.createElement('div'); name.className = 'file-name'; name.textContent = doc.name;
    const meta = document.createElement('div'); meta.className = 'file-meta';
    meta.textContent = doc.locked ? `${bytes(doc.bytes)}, password protected` : `${doc.pages} ${doc.pages === 1 ? 'page' : 'pages'}, ${bytes(doc.bytes)}`;
    const measured = mode === 'compress' && !doc.locked ? current().compressionPreview?.fileBytes[index] : null;
    if (measured != null) meta.textContent += `, output ${bytes(measured)}`;
    info.append(name, meta);
    const actions = document.createElement('div'); actions.className = 'file-actions';
    if (mode === 'merge') {
      const position = document.createElement('span'); position.className = 'position'; position.textContent = String(index + 1); row.append(position);
      actions.append(actionButton(`Move ${doc.name} earlier`, 'up', () => changeOrder(index, index - 1), index === 0));
      actions.append(actionButton(`Move ${doc.name} later`, 'down', () => changeOrder(index, index + 1), index === current().documents.length - 1));
    }
    if (doc.locked) {
      const unlock = document.createElement('button'); unlock.className = 'secondary'; unlock.textContent = 'Unlock';
      unlock.setAttribute('aria-label', `Unlock ${doc.name}`); unlock.addEventListener('click', () => promptPassword(doc));
      actions.append(unlock);
    } else {
      const preview = document.createElement('button'); preview.className = 'secondary';
      preview.append(icon('preview'), document.createTextNode('Preview'));
      preview.setAttribute('aria-label', `Preview ${doc.name}`); preview.addEventListener('click', () => openDocumentPreview(doc));
      actions.append(preview);
    }
    actions.append(actionButton(`Remove ${doc.name}`, 'trash', () => removeDocument(doc.id))); row.append(cover, info, actions); $('documents').append(row);
  });
}
function rememberEdit() {
  const view = current(); view.history.push(view.pages.map((page) => ({ ...page }))); view.future = [];
  if (view.history.length > 50) view.history.shift(); view.result = null;
}
function updateSelection() {
  for (const card of $('page-grid').children) {
    const selected = current().selected.has(Number(card.dataset.page)); card.classList.toggle('selected', selected); card.setAttribute('aria-pressed', String(selected));
    card.querySelector('.page-check').replaceChildren(...(selected ? [icon('check')] : []));
  }
  $('organizer-selection').textContent = mode === 'organize' ? `${current().selected.size} selected` : '';
  $('page-selection').textContent = mode === 'split' && current().splitMode === 'ranges' ? `Group ${current().activeGroup + 1}: ${current().selected.size} pages selected` : mode === 'split' && current().splitMode === 'after' ? `${current().selected.size} split points` : mode === 'split' && current().splitMode === 'bookmarks' ? '' : `${current().selected.size} selected`;
  $('select-all').textContent = current().selected.size === current().pages.length ? 'Deselect all' : 'Select all'; controls();
}
function renderPages() {
  $('page-grid').replaceChildren(); const view = current();
  view.pageWindow = Math.min(view.pageWindow, Math.max(0, Math.ceil(view.pages.length / PAGE_WINDOW) - 1));
  const start = view.pageWindow * PAGE_WINDOW;
  view.pages.slice(start, start + PAGE_WINDOW).forEach((page, offset) => {
    const position = start + offset; const card = document.createElement('div'); card.className = 'page-card'; card.tabIndex = 0;
    card.setAttribute('role', 'button'); card.setAttribute('aria-label', mode === 'split' ? `Page ${page.page}. ${view.splitMode === 'after' ? 'Toggle split point' : 'Select'}. Press Enter to preview.` : `Output page ${position + 1}, original page ${page.page}. Select, or press Enter to preview.`);
    card.dataset.page = String(pageKey(page));
    if (mode === 'organize') card.title = `${view.documents.find((doc) => doc.id === page.documentId)?.name || ''}, page ${page.page}`;
    if (mode === 'organize') reorderable(card, position, card, () => busy, (from, to, after) => {
      const pages = movePagesBeside(view.pages, view.selected, pageKey(view.pages[to]), after);
      if (pages.map(pageKey).join(',') === view.pages.map(pageKey).join(',')) return;
      rememberEdit(); view.pages = pages; render(); message('Page order updated.');
    }, { onStart: () => {
      if (!view.selected.has(pageKey(page))) view.selected = new Set([pageKey(page)]);
      updateSelection(); return view.selected.size;
    } });
    const source = view.documents.find((doc) => doc.id === page.documentId) || view.documents[0];
    const paper = document.createElement('div'); paper.className = 'page-paper'; previews.watch(paper, source.id, page.page, page.rotation, source.previewBytes);
    const check = document.createElement('span'); check.className = 'page-check'; check.setAttribute('aria-hidden', 'true');
    const caption = document.createElement('span'); caption.className = 'page-caption'; caption.textContent = `Page ${mode === 'organize' ? position + 1 : page.page}${page.rotation ? `, rotated ${page.rotation} degrees` : ''}`; card.append(paper, check, caption);
    {
      const actions = document.createElement('div'); actions.className = 'page-actions';
      const preview = actionButton(`Preview page ${page.page}`, 'preview', () => openPreview(page));
      const rotate = actionButton(`Rotate page ${page.page}`, 'rotate', () => { if (busy) return; rememberEdit(); page.rotation = (page.rotation + 90) % 360; render(); message('Unsaved changes'); });
      for (const button of [preview, rotate]) button.addEventListener('click', (event) => event.stopPropagation());
      actions.addEventListener('dblclick', (event) => event.stopPropagation()); actions.append(preview); if (mode === 'organize') actions.append(rotate); card.append(actions);
    }
    const select = (event) => {
      if (busy) return;
      const key = pageKey(page);
      if (event.shiftKey && lastSelected !== null) {
        const previous = view.pages.findIndex((item) => pageKey(item) === lastSelected);
        if (previous >= 0) for (let index = Math.min(previous, position); index <= Math.max(previous, position); index += 1) view.selected.add(pageKey(view.pages[index]));
      } else if (view.selected.has(key)) view.selected.delete(key); else view.selected.add(key);
      lastSelected = key; if (mode === 'split') writeSplitSelection(); updateSelection();
    };
    card.addEventListener('click', select); card.addEventListener('dblclick', () => openPreview(page));
    card.addEventListener('keydown', (event) => {
      if (event.target !== card) return;
      const offset = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -getComputedStyle($('page-grid')).gridTemplateColumns.split(' ').length, ArrowDown: getComputedStyle($('page-grid')).gridTemplateColumns.split(' ').length }[event.key];
      if (offset && !busy) {
        const nextPosition = Math.max(0, Math.min(view.pages.length - 1, position + offset)); const next = view.pages[nextPosition]; event.preventDefault();
        if (event.shiftKey) { if (lastSelected === null) lastSelected = pageKey(page); const found = view.pages.findIndex((item) => pageKey(item) === lastSelected); const anchor = found < 0 ? position : found; for (let index = Math.min(anchor, nextPosition); index <= Math.max(anchor, nextPosition); index++) view.selected.add(pageKey(view.pages[index])); if (mode === 'split') writeSplitSelection(); updateSelection(); }
        if (Math.floor(nextPosition / PAGE_WINDOW) !== view.pageWindow) { view.pageWindow = Math.floor(nextPosition / PAGE_WINDOW); render(); }
        $('page-grid').querySelector(`[data-page="${pageKey(next)}"]`)?.focus(); return;
      }
      if (event.key === ' ') { event.preventDefault(); select(event); } if (event.key === 'Enter') { event.preventDefault(); openPreview(page); } });
    $('page-grid').append(card);
  });
  $('page-window').textContent = view.pages.length ? `${start + 1}-${Math.min(start + PAGE_WINDOW, view.pages.length)} of ${view.pages.length}` : 'No pages';
  $('jump-page').max = String(view.pages.length); updateSelection();
}
function splitSizeBytes(view) {
  const value = Number(view.splitSize), bytes = Math.floor(value * 1000000);
  if (!Number.isFinite(value) || bytes < 100000 || bytes > 250000000) throw new Error('Choose a limit between 0.1 and 250 MB.');
  return bytes;
}
function splitPlanFor(view) {
  if (view.splitMode === 'file_size') { splitSizeBytes(view); return [Array.from({ length: view.documents[0].pages }, (_, index) => index + 1)]; }
  if (view.splitMode === 'bookmarks') {
    const plan = bookmarkGroups(view.documents[0].features.bookmarks, view.documents[0].pages).map((group) => group.pages);
    if (plan.length > 500 || plan.some((group) => group.length > 10000) || plan.flat().length > 20000) throw new Error('Choose at most 20,000 output pages in total.');
    return plan;
  }
  const ranges = view.splitMode === 'ranges' ? view.splitGroups.join(';') : view.splitMode === 'after' ? view.splitCuts : view.ranges;
  const plan = splitPlan(view.splitMode, view.groupSize, ranges, view.documents[0].pages, { separate: view.splitSeparate, combine: view.splitCombine });
  if (plan.map(pageList).join(';').length > 20000) throw new Error('The page range list is too long.');
  return plan;
}
function splitOutputNames(view) {
  if (view.splitMode === 'ranges' && !view.splitCombine) return view.splitGroups.map((_, i) => view.splitNames[i]?.trim() || `Group ${i + 1}`);
  if (view.splitMode !== 'bookmarks') return [];
  try { return bookmarkGroups(view.documents[0].features.bookmarks, view.documents[0].pages).map((group) => group.name); } catch { return []; }
}
function readSplitSelection() {
  const view = current();
  if (view.splitMode === 'bookmarks') { view.selected.clear(); return; }
  const text = view.splitMode === 'ranges' ? view.splitGroups[view.activeGroup] : view.splitMode === 'after' ? view.splitCuts : view.ranges;
  try { view.selected = new Set(parseRange(text, view.documents[0].pages)); } catch { view.selected.clear(); }
}
function writeSplitSelection() {
  const view = current(); if (view.splitMode === 'bookmarks') return;
  const text = pageList([...view.selected].sort((a, b) => a - b)); view.result = null;
  if (view.splitMode === 'ranges') {
    view.splitGroups[view.activeGroup] = text;
    $(`split-group-${view.activeGroup}`).value = text;
  } else if (view.splitMode === 'after') { view.splitCuts = text; $('split-cuts').value = text; }
  else { view.ranges = text; $('split-ranges').value = text; }
  splitSummary();
}
function renderSplitGroups() {
  const view = current(); $('split-group-list').replaceChildren();
  view.splitGroups.forEach((value, index) => {
    const row = document.createElement('div'); row.className = 'split-group'; row.classList.toggle('active', index === view.activeGroup);
    const label = document.createElement('input'); label.type = 'text'; label.className = 'group-name'; label.placeholder = `Group ${index + 1}`; label.value = view.splitNames[index] || ''; label.setAttribute('aria-label', `Group ${index + 1} name`); label.maxLength = 80; label.addEventListener('input', () => { view.splitNames[index] = label.value; view.result = null; splitSummary(); });
    const input = document.createElement('input'); input.type = 'text'; input.id = `split-group-${index}`; input.setAttribute('aria-label', `Group ${index + 1} pages`); input.value = value; input.placeholder = '1-3, 5, 8'; input.autocomplete = 'off'; input.spellcheck = false;
    const activate = () => {
      if (busy) return;
      view.activeGroup = index; lastSelected = null;
      for (const [position, item] of [...$('split-group-list').children].entries()) item.classList.toggle('active', position === index);
      readSplitSelection(); updateSelection();
    };
    input.addEventListener('focus', activate);
    input.addEventListener('input', () => { view.splitGroups[index] = input.value; view.result = null; readSplitSelection(); splitSummary(); updateSelection(); });
    const select = document.createElement('button'); select.className = 'secondary'; select.textContent = 'Choose pages'; select.addEventListener('click', activate);
    const actions = document.createElement('div'); actions.className = 'file-actions';
    const move = (offset) => { if (busy) return; view.splitGroups.splice(index + offset, 0, view.splitGroups.splice(index, 1)[0]); view.splitNames.splice(index + offset, 0, view.splitNames.splice(index, 1)[0]); view.activeGroup = index + offset; view.result = null; lastSelected = null; render(); };
    actions.append(actionButton(`Move group ${index + 1} earlier`, 'up', () => move(-1), index === 0), actionButton(`Move group ${index + 1} later`, 'down', () => move(1), index === view.splitGroups.length - 1));
    actions.append(actionButton(`Remove group ${index + 1}`, 'trash', () => { if (busy) return; view.splitGroups.splice(index, 1); view.splitNames.splice(index, 1); view.activeGroup = Math.min(view.activeGroup, view.splitGroups.length - 1); view.result = null; lastSelected = null; render(); }, view.splitGroups.length === 1));
    row.append(label, input, select, actions); $('split-group-list').append(row);
  });
}
function splitSummary() {
  if (!current().documents.length) return;
  const view = current(); $('split-output-list').replaceChildren(); $('split-summary').hidden = false;
  for (const input of $('split-options').querySelectorAll('input')) input.removeAttribute('aria-invalid');
  try {
    if (view.splitMode === 'file_size') {
      splitSizeBytes(view); $('split-summary').textContent = ''; $('split-summary').hidden = true;
      $('split-summary').className = 'context-note'; $('split-output-details').hidden = true; controls(); return;
    }
    const plan = splitPlanFor(view); const used = new Set(plan.flat()).size; const names = splitOutputNames(view);
    $('split-summary').textContent = `${plan.length} ${plan.length === 1 ? 'PDF' : 'PDFs'}, ${plan.flat().length} output pages${used < view.documents[0].pages ? `, ${view.documents[0].pages - used} pages excluded` : ''}.`;
    $('split-summary').className = 'context-note'; $('split-output-details').hidden = false;
    plan.slice(0, 20).forEach((pages, index) => {
      const item = document.createElement('li'); item.textContent = `${names[index] || `PDF ${index + 1}`}: ${pages.length} ${pages.length === 1 ? 'page' : 'pages'} (${pageList(pages)})`; $('split-output-list').append(item);
    });
    if (plan.length > 20) { const item = document.createElement('li'); item.textContent = `${plan.length - 20} more PDFs`; $('split-output-list').append(item); }
  } catch (error) {
    $('split-summary').textContent = error.message; $('split-summary').className = 'context-note invalid'; $('split-output-details').hidden = true;
    if (view.splitMode === 'ranges') view.splitGroups.forEach((range, index) => { try { parseRange(range, view.documents[0].pages); } catch { $(`split-group-${index}`).setAttribute('aria-invalid', 'true'); } });
    else if (view.splitMode !== 'bookmarks') $(view.splitMode === 'file_size' ? 'split-size' : view.splitMode === 'after' ? 'split-cuts' : view.splitMode === 'groups' ? 'group-size' : 'split-ranges').setAttribute('aria-invalid', 'true');
  }
  controls();
}
function renderChanges() {
  const view = current(); const section = $('document-changes');
  const info = view.documents.length ? changesFor(view) : null;
  const active = Boolean(info) && (Boolean(info.blocked) || info.flatten || info.acknowledge || info.notes.length > 0);
  section.hidden = !active;
  if (!active) return;
  $('changes-message').textContent = info.blocked || info.notes.join(' ');
  $('changes-message').className = info.blocked ? 'invalid' : '';
  $('flatten-row').hidden = !info.flatten; $('acknowledge-row').hidden = !info.acknowledge;
  $('flatten-forms').checked = view.flattenForms; $('acknowledge-changes').checked = view.acknowledgeChanges;
}
function render() {
  document.querySelector('.drop-label').textContent = ['merge', 'organize'].includes(mode) ? 'Drop PDFs or images here' : 'Drop PDFs here';
  $('choose-label').textContent = ['merge', 'organize'].includes(mode) ? 'Choose files' : mode === 'compress' ? 'Choose PDFs' : 'Choose a PDF';
  const previousCards = mode === 'organize' ? new Map([...$('page-grid').children].map((card) => [card.dataset.page, card.getBoundingClientRect()])) : new Map();
  previews.resetView(); const view = current(); const hasDocuments = view.documents.length > 0;
  $('empty').hidden = hasDocuments; $('loaded').hidden = !hasDocuments;
  $('compression-options').hidden = mode !== 'compress'; $('split-options').hidden = mode !== 'split'; $('organizer').hidden = mode !== 'organize' && !(mode === 'split' && ['selected', 'ranges', 'after'].includes(view.splitMode));
  $('organizer-edits').hidden = mode !== 'organize'; $('move-pages-controls').hidden = mode !== 'organize'; $('organizer-selection').hidden = mode !== 'organize'; $('undo').hidden = mode !== 'organize'; $('redo').hidden = mode !== 'organize'; $('invert-selection').hidden = mode !== 'split';
  $('page-grid').classList.toggle('split-pages', mode === 'split'); $('page-grid').setAttribute('aria-label', mode === 'split' ? 'Source pages' : 'Pages in output order');
  $('organizer-tools').parentElement.setAttribute('aria-label', mode === 'split' ? 'Select pages for splitting' : 'Organize selected pages');
  $('size-row').hidden = view.splitMode !== 'file_size'; $('group-row').hidden = view.splitMode !== 'groups'; $('range-row').hidden = view.splitMode !== 'ranges'; $('selection-row').hidden = view.splitMode !== 'selected'; $('cut-row').hidden = view.splitMode !== 'after'; renderDocuments();
  if (mode === 'split' && hasDocuments) { renderSplitGroups(); readSplitSelection(); }
  if (!$('organizer').hidden && hasDocuments) renderPages(); if (mode === 'split' && hasDocuments) splitSummary();
  if (mode === 'organize' && !window.matchMedia('(prefers-reduced-motion: reduce)').matches) for (const card of $('page-grid').children) {
    const before = previousCards.get(card.dataset.page); if (!before) continue; const after = card.getBoundingClientRect();
    if (before.x !== after.x || before.y !== after.y) card.animate([{ transform: `translate(${before.x - after.x}px, ${before.y - after.y}px)` }, { transform: 'translate(0, 0)' }], { duration: 200, easing: 'cubic-bezier(.2,.8,.2,1)' });
  }
  const total = view.documents.reduce((sum, doc) => sum + doc.bytes, 0);
  const pages = mode === 'organize' ? view.pages.length : view.documents.reduce((sum, doc) => sum + doc.pages, 0);
  $('summary').textContent = hasDocuments ? `${view.documents.length} ${view.documents.length === 1 ? 'document' : 'documents'}, ${pages} pages, ${bytes(total)}` : 'No PDFs selected';
  renderChanges(); renderImportErrors();
  controls(); scheduleCompression();
}
async function cleanRegistry() {
  const ids = [...new Set(Object.values(state).flatMap((view) => view.documents.map((doc) => doc.id)))];
  await invoke('retain_documents', { ids });
  if (!state.compress.compressionPreview) await invoke('discard_compression_preview');
  await previews.releaseUnused(ids);
}
function resetEdits(view) {
  view.flattenForms = false; view.acknowledgeChanges = false; view.future = [];
}
async function removeDocument(id) {
  if (busy) return; busy = true;
  invalidateCompression(); current().documents = current().documents.filter((doc) => doc.id !== id); current().pages = mode === 'organize' ? current().pages.filter((page) => page.documentId !== id) : []; current().selected.clear(); current().history = []; current().result = null; resetEdits(current()); render();
  try { await cleanRegistry(); message('Document removed.'); }
  catch (error) { message(String(error), 'error'); }
  finally { busy = false; render(); }
}
async function acceptSelection(selected, insertionIndex) {
  invalidateCompression();
  const view = current();
  if (['merge', 'compress'].includes(mode)) {
    let position = insertionIndex ?? view.documents.length;
    for (const doc of selected) {
      const index = view.documents.findIndex((old) => old.id === doc.id);
      if (index === -1) view.documents.splice(position++, 0, doc); else view.documents[index] = doc;
    }
  } else if (mode === 'organize') {
    if (view.pages.length + selected.reduce((sum, doc) => sum + doc.pages, 0) > 20000) throw new Error('Choose at most 20,000 pages.');
    const added = pagesFromDocuments(selected, () => --duplicateKey);
    rememberEdit();
    const position = insertionIndex ?? view.pages.length;
    let pendingPosition = position;
    for (const doc of selected) {
      const existing = view.documents.findIndex((item) => item.id === doc.id);
      if (doc.locked) doc.insertionIndex = pendingPosition;
      pendingPosition += doc.pages;
      if (existing < 0) view.documents.push(doc); else view.documents[existing] = doc;
    }
    view.pages = insertPages(view.pages, added, position);
    view.selected = new Set(added.map(pageKey));
    view.pageWindow = Math.floor(position / PAGE_WINDOW); lastSelected = null;
  } else {
    view.documents = selected;
    view.pages = ['organize', 'split'].includes(mode) ? Array.from({ length: selected[0].pages }, (_, index) => ({ page: index + 1, rotation: 0 })) : [];
    view.selected.clear(); view.history = []; view.pageWindow = 0; lastSelected = null;
    if (mode === 'split') { view.ranges = ''; view.splitGroups = ['']; view.splitNames = ['']; view.activeGroup = 0; view.splitCuts = ''; $('split-ranges').value = ''; $('split-cuts').value = ''; }
  }
  resetEdits(view); view.result = null;
  for (const doc of selected) await previews.forget(doc.id);
  await cleanRegistry();
}
function promptPassword(doc) {
  if (busy || $('password-dialog').open) return;
  pendingUnlock = doc;
  $('password-name').textContent = doc.name;
  $('password-error').hidden = true; $('password-error').textContent = '';
  $('pdf-password').value = '';
  $('password-dialog').showModal();
  $('pdf-password').focus();
}
function promptNextLocked() {
  const locked = current().documents.find((doc) => doc.locked);
  if (locked) promptPassword(locked);
}
function showImportResult(result) {
  const added = result.documents.length, skipped = result.rejected.length;
  const parts = [];
  if (added) parts.push(`${added} ${added === 1 ? 'file added' : 'files added'}`);
  if (skipped) parts.push(`${skipped} ${skipped === 1 ? 'file skipped' : 'files skipped'}`);
  message(parts.join('. ') || 'Selection cancelled.', skipped && !added ? 'error' : '');
}
function renderImportErrors() {
  const report = $('import-errors'); const errors = current().importErrors;
  report.hidden = !errors.length;
  $('import-errors-summary').textContent = `${errors.length} ${errors.length === 1 ? 'file skipped' : 'files skipped'}`;
  $('import-errors-list').replaceChildren(...errors.map((error) => {
    const item = document.createElement('li');
    const name = document.createElement('strong'); name.textContent = error.name;
    item.append(name, document.createTextNode(`: ${error.reason}`)); return item;
  }));
}
async function choose() {
  if (busy || !ready) return; busy = true; controls(); message(['merge', 'organize'].includes(mode) ? 'Select files' : 'Select PDFs');
  try {
    const result = await invoke('choose_documents', { multiple: ['merge', 'compress', 'organize'].includes(mode), allowImages: ['merge', 'organize'].includes(mode) });
    if (!result.documents.length && !result.rejected.length) { message('Selection cancelled.'); return; }
    current().importErrors = result.rejected; $('import-errors').open = !result.documents.length;
    if (result.documents.length) await acceptSelection(result.documents);
    showImportResult(result);
  } catch (error) {
    try { await cleanRegistry(); } catch { /* Keep the original import error visible. */ }
    message(String(error), 'error');
  } finally { busy = false; render(); }
  promptNextLocked();
}
async function importDrop(paths, position) {
  if ($('preview-dialog').open || $('password-dialog').open) { clearDropIndicator(); return; }
  if (busy) { message('Wait until processing finishes.'); return; }
  if (!ready) { message('PDF engine unavailable.', 'error'); return; }
  if (mode === 'split' && paths.length > 1) { message('This tool uses one PDF at a time.', 'error'); return; }
  const row = position ? document.elementFromPoint(position.x / window.devicePixelRatio, position.y / window.devicePixelRatio)?.closest('.document') : null;
  const insertionIndex = mode === 'organize' ? organizeDropTarget(position).index : mode === 'merge' && row ? Number(row.dataset.reorderIndex) : undefined;
  clearDropIndicator();
  busy = true; controls(); message(['merge', 'organize'].includes(mode) ? 'Adding files...' : 'Adding PDFs...');
  try {
    const result = await invoke('import_dropped_documents', { paths, allowImages: ['merge', 'organize'].includes(mode) });
    current().importErrors = result.rejected; $('import-errors').open = !result.documents.length;
    if (result.documents.length) await acceptSelection(result.documents, insertionIndex);
    showImportResult(result);
  } catch (error) {
    try { await cleanRegistry(); } catch { /* Keep the original import error visible. */ }
    message(String(error), 'error');
  } finally { busy = false; render(); }
  promptNextLocked();
}
const dropIndicator = document.createElement('div');
dropIndicator.className = 'file-drop-indicator'; dropIndicator.hidden = true;
dropIndicator.setAttribute('role', 'status'); dropIndicator.setAttribute('aria-live', 'polite'); document.body.append(dropIndicator);
let dropCard = null;
function organizeDropTarget(position) {
  if (!position) return { index: current().pages.length, card: null, after: true };
  const x = position.x / window.devicePixelRatio, y = position.y / window.devicePixelRatio;
  const grid = $('page-grid').getBoundingClientRect();
  if ($('organizer').hidden || x < grid.left || x > grid.right || y < grid.top || y > grid.bottom) return { index: current().pages.length, card: null, after: true };
  const cards = [...$('page-grid').children].map((element) => ({ element, index: Number(element.dataset.reorderIndex), rect: element.getBoundingClientRect() }));
  return insertionAtPoint(cards, x, y, current().pages.length);
}
function clearDropIndicator() {
  dropCard?.classList.remove('drag-target', 'drag-after'); dropCard = null;
  dropIndicator.hidden = true; $('content-scroll').classList.remove('file-drop-active');
}
function showDropIndicator(position) {
  clearDropIndicator();
  if (busy || !ready || $('preview-dialog').open || $('password-dialog').open) return;
  $('content-scroll').classList.add('file-drop-active');
  if (mode !== 'organize') return;
  if (position) {
    const scroll = $('content-scroll'); const rect = scroll.getBoundingClientRect();
    const y = position.y / window.devicePixelRatio;
    if (y > rect.bottom - 48 && y < rect.bottom) scroll.scrollTop += 16;
    else if (y < rect.top + 48 && y > rect.top) scroll.scrollTop -= 16;
  }
  const target = organizeDropTarget(position);
  dropCard = target.card?.element || null;
  dropCard?.classList.add('drag-target'); dropCard?.classList.toggle('drag-after', target.after);
  dropIndicator.textContent = target.index === current().pages.length ? 'Add pages at end' : `Insert before page ${target.index + 1}`;
  dropIndicator.hidden = false;
}
// Stop the browser's default file navigation even outside the document list.
for (const type of ['dragover', 'drop']) window.addEventListener(type, (event) => event.preventDefault(), true);
try {
  const unlisten = await getCurrentWebview().onDragDropEvent(({ payload }) => {
    if (payload.type === 'enter' || payload.type === 'over') showDropIndicator(payload.position);
    if (payload.type === 'leave') clearDropIndicator();
    if (payload.type === 'drop') importDrop(payload.paths, payload.position);
  });
  window.addEventListener('beforeunload', () => { unlisten(); }, { once: true });
} catch { message('File drop could not start.', 'error'); }
function invalidateCompression() {
  if (mode !== 'compress') return;
  const view = current();
  view.compressionPreview = null; view.measurementKey = null; view.measurementFailed = false;
}
function compressionSummary() {
  const view = state.compress; const host = $('compression-size');
  host.hidden = mode !== 'compress' || !view.documents.length;
  const measured = view.compressionPreview;
  if (!measured) {
    host.removeAttribute('title');
    host.classList.remove('invalid'); host.textContent = !validation() ? '' : view.measurementFailed ? 'Size unavailable' : 'Calculating output size...';
    return;
  }
  const saved = measured.inputBytes - measured.outputBytes;
  host.textContent = saved > 0 ? `Expected output: ${bytes(measured.outputBytes)}, saves ${bytes(saved)}` : `Expected output: ${bytes(measured.outputBytes)}, already optimized`;
  const limit = targetBytes(view);
  if (limit && measured.fileBytes.some((size) => size > limit)) host.textContent += ' — Over limit';
  host.classList.toggle('invalid', Boolean(limit && measured.fileBytes.some((size) => size > limit)));
  host.title = `${measured.outputBytes.toLocaleString()} bytes${measured.files > 1 ? ' total' : ''}`;
}
function scheduleCompression() {
  clearTimeout(compressionTimer);
  if (mode !== 'compress' || busy || !ready || !current().documents.length || !validation()) return;
  const view = current();
  const key = JSON.stringify([view.compression, view.targetSize, view.acknowledgeChanges, view.documents.map((doc) => doc.id)]);
  if (view.measurementKey === key) return;
  compressionTimer = setTimeout(() => measureCompression(key), 250);
}
async function measureCompression(key) {
  if (mode !== 'compress' || busy || !ready || !current().documents.length || !validation()) return;
  const view = current();
  if (key !== JSON.stringify([view.compression, view.targetSize, view.acknowledgeChanges, view.documents.map((doc) => doc.id)])) return;
  view.measurementKey = key; view.measurementFailed = false; view.compressionPreview = null;
  busy = true; cancelAvailable = false; jobId = crypto.randomUUID(); controls();
  const onProgress = new Channel();
  onProgress.onmessage = (progress) => { message(progress.message); cancelAvailable = progress.cancellable; controls(); };
  try {
    view.compressionPreview = await invoke('process_documents', {
      request: { jobId, operation: 'compress', ids: view.documents.map((doc) => doc.id), compression: view.compression, targetBytes: targetBytes(view), acknowledgeChanges: view.acknowledgeChanges, measureOnly: true }, onProgress,
    });
    message(view.compressionPreview.recovered ? 'Ready to save. PDF data recovered; review the saved copy.' : 'Ready to save');
  } catch (error) {
    view.measurementFailed = true;
    message(String(error), String(error).startsWith('Cancelled') ? '' : 'error');
  } finally {
    busy = false; jobId = null; cancelAvailable = false; render();
  }
}
async function save() {
  if (busy || !ready || !validation() || (mode === 'compress' && !current().compressionPreview && !current().measurementFailed)) return;
  busy = true; cancelAvailable = false; const activeMode = mode; const view = current(); jobId = crypto.randomUUID();
  const onProgress = new Channel(); onProgress.onmessage = (progress) => { message(progress.message); cancelAvailable = progress.cancellable; controls(); };
  controls(); message(mode === 'split' || (mode === 'compress' && view.documents.length > 1) ? 'Select output folder' : 'Select save location');
  try {
    const splitRanges = mode === 'split' ? splitPlanFor(view).map(pageList).join(';') : view.ranges;
    const result = await invoke('process_documents', { request: { jobId, operation: mode, ids: outputDocuments(view).map((doc) => doc.id), compression: view.compression, previewId: view.compressionPreview?.id ?? null, splitMode: mode === 'split' ? (view.splitMode === 'file_size' ? 'file_size' : 'ranges') : 'every_page', pagesPerFile: Number(view.groupSize), ranges: splitRanges, pages: view.pages.map((page) => ({ documentId: page.documentId || '', page: page.page, rotation: page.rotation })), targetBytes: mode === 'compress' ? targetBytes(view) : null, maxOutputBytes: mode === 'split' && view.splitMode === 'file_size' ? splitSizeBytes(view) : null, outputNames: mode === 'split' ? splitOutputNames(view) : [], flattenForms: view.flattenForms, acknowledgeChanges: view.acknowledgeChanges }, onProgress });
    if (!result) { message('Save cancelled.'); return; } view.result = result;
    let text = activeMode !== 'split' && result.files === 1 ? `Saved ${result.name}. ${bytes(result.outputBytes)}.` : `Saved ${result.files} PDFs in ${result.name}.`;
    if (activeMode === 'compress') text += result.outputBytes < result.inputBytes ? ` ${Math.round((1 - result.outputBytes / result.inputBytes) * 100)}% smaller.` : result.recovered ? ' Saved a repaired copy.' : ' Already optimized. Saved an unchanged copy.';
    if (result.recovered) text += ' PDF data recovered; review the saved copy.';
    message(text, 'success');
  } catch (error) { message(String(error), String(error).startsWith('Cancelled') ? '' : 'error'); }
  finally { busy = false; jobId = null; cancelAvailable = false; controls(); }
}
function editPages(action) {
  if (busy || !current().selected.size || (action === 'remove' && current().selected.size === current().pages.length)) return;
  const view = current(); rememberEdit();
  if (action === 'rotate' || action === 'rotate-left') view.pages = view.pages.map((page) => view.selected.has(pageKey(page)) ? { ...page, rotation: (page.rotation + (action === 'rotate-left' ? 270 : 90)) % 360 } : page);
  if (action === 'remove') { view.pages = view.pages.filter((page) => !view.selected.has(pageKey(page))); view.selected.clear(); lastSelected = null; }
  if (action === 'earlier') for (let index = 1; index < view.pages.length; index += 1) { if (view.selected.has(pageKey(view.pages[index])) && !view.selected.has(pageKey(view.pages[index - 1]))) [view.pages[index - 1], view.pages[index]] = [view.pages[index], view.pages[index - 1]]; }
  if (action === 'later') for (let index = view.pages.length - 2; index >= 0; index -= 1) { if (view.selected.has(pageKey(view.pages[index])) && !view.selected.has(pageKey(view.pages[index + 1]))) [view.pages[index + 1], view.pages[index]] = [view.pages[index], view.pages[index + 1]]; }
  document.querySelector('.page-more')?.removeAttribute('open');
  render(); message('Unsaved changes');
}
function duplicateSelected() {
  const view = current();
  if (busy || !view.selected.size) return;
  if (view.pages.length + view.selected.size > 20000) { message('Keep at most 20,000 pages.', 'error'); return; }
  rememberEdit();
  const copies = [];
  view.pages = view.pages.flatMap((page) => {
    if (!view.selected.has(pageKey(page))) return [page];
    const copy = { ...page, key: --duplicateKey }; copies.push(copy);
    return [page, copy];
  });
  view.selected = new Set(copies.map(pageKey)); lastSelected = null;
  render(); message('Unsaved changes');
}
function openDocumentPreview(doc, pageNumber = null) {
  if (busy || doc.locked || $('preview-dialog').open) return;
  const edited = mode === 'organize' && current().pages.length > 0;
  previewSession = {
    doc, pages: edited ? current().pages.map((page) => ({ ...page })) : null,
    index: edited && pageNumber !== null ? Math.max(0, current().pages.findIndex((page) => page.page === pageNumber)) : edited ? Math.max(0, current().pages.findIndex((page) => page.documentId === doc.id)) : (pageNumber ?? 1) - 1,
    count: edited ? current().pages.length : doc.pages,
    compare: null,
  };
  $('preview-title').textContent = doc.name;
  $('preview-zoom').value = '1';
  $('preview-dialog').showModal();
  renderPreview();
}
function openCompare() {
  const view = state.compress;
  if (busy || !view.compressionPreview || view.documents.length !== 1 || $('preview-dialog').open) return;
  const doc = view.documents[0];
  previewSession = { doc, pages: null, index: 0, count: doc.pages, compare: { previewId: view.compressionPreview.id, outputIndex: 0, bytes: view.compressionPreview.fileBytes[0] } };
  $('preview-title').textContent = 'Compare';
  $('preview-zoom').value = '1';
  $('preview-dialog').showModal();
  renderPreview();
}
function openPreview(page) {
  const doc = current().documents.find((item) => item.id === page.documentId) || current().documents[0];
  openDocumentPreview(doc, page.page);
  if (mode === 'organize' && previewSession) { previewSession.index = current().pages.findIndex((item) => pageKey(item) === pageKey(page)); renderPreview(); }
}
async function renderPreview() {
  if (!previewSession || !$('preview-dialog').open) return;
  const sequence = ++previewRenderSequence;
  const { doc, pages, index, count, compare } = previewSession;
  $('compressed-pane').hidden = !compare; $('original-label').hidden = !compare; $('preview-panes').classList.toggle('comparing', Boolean(compare));
  const page = pages ? pages[index] : { page: index + 1, rotation: 0 };
  const source = pages ? state.organize.documents.find((item) => item.id === page.documentId) || doc : doc;
  if (pages) $('preview-title').textContent = source.name;
  $('preview-page').value = String(index + 1); $('preview-page').max = String(count);
  $('preview-total').textContent = `of ${count}`;
  $('preview-previous').disabled = index === 0; $('preview-next').disabled = index === count - 1;
  const zoom = Number($('preview-zoom').value);
  const host = $('large-preview'); host.textContent = 'Preparing preview...'; host.scrollTop = 0; host.scrollLeft = 0;
  const width = Math.max(1, host.clientWidth - 44) * zoom;
  const height = Math.max(1, host.clientHeight - 44) * zoom;
  await previews.draw({ host, id: source.id, page: page.page, rotation: page.rotation, width, height, bytes: source.previewBytes });
  if (sequence === previewRenderSequence && compare && previewSession && previewSession.compare === compare && $('preview-dialog').open) {
    const compressedHost = $('compressed-preview'); compressedHost.textContent = 'Preparing preview...'; compressedHost.scrollTop = 0; compressedHost.scrollLeft = 0;
    const compressedWidth = Math.max(1, compressedHost.clientWidth - 44) * zoom;
    const compressedHeight = Math.max(1, compressedHost.clientHeight - 44) * zoom;
    await previews.draw({ host: compressedHost, id: doc.id, previewId: compare.previewId, outputIndex: compare.outputIndex, page: page.page, rotation: page.rotation, width: compressedWidth, height: compressedHeight, bytes: compare.bytes });
  }
}
function resizePreview(width, height) {
  const dialog = $('preview-dialog');
  dialog.style.width = `${Math.max(Math.min(480, innerWidth - 32), Math.min(width, innerWidth - 32))}px`;
  dialog.style.height = `${Math.max(Math.min(420, innerHeight - 32), Math.min(height, innerHeight - 32))}px`;
}
$('preview-resize').addEventListener('pointerdown', (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  const grip = event.currentTarget;
  const rect = $('preview-dialog').getBoundingClientRect();
  const startX = event.clientX, startY = event.clientY;
  grip.setPointerCapture(event.pointerId);
  const move = (next) => resizePreview(rect.width + (next.clientX - startX) * 2, rect.height + (next.clientY - startY) * 2);
  const stop = () => { grip.removeEventListener('pointermove', move); grip.removeEventListener('pointerup', stop); grip.removeEventListener('pointercancel', stop); };
  grip.addEventListener('pointermove', move);
  grip.addEventListener('pointerup', stop);
  grip.addEventListener('pointercancel', stop);
});
$('preview-resize').addEventListener('keydown', (event) => {
  const directions = { ArrowLeft: [-20, 0], ArrowRight: [20, 0], ArrowUp: [0, -20], ArrowDown: [0, 20] };
  if (!directions[event.key]) return;
  event.preventDefault();
  const rect = $('preview-dialog').getBoundingClientRect();
  const [width, height] = directions[event.key];
  resizePreview(rect.width + width, rect.height + height);
});
let previewResizeTimer = null;
let previewSize = '';
new ResizeObserver(([entry]) => {
  const size = `${Math.round(entry.contentRect.width)}:${Math.round(entry.contentRect.height)}`;
  if (size === previewSize) return;
  previewSize = size;
  if (!previewSession || !$('preview-dialog').open) return;
  clearTimeout(previewResizeTimer);
  previewResizeTimer = setTimeout(() => { previewResizeTimer = null; renderPreview(); }, 120);
}).observe($('preview-dialog'));
function movePreview(offset) {
  if (!previewSession) return;
  previewSession.index = Math.max(0, Math.min(previewSession.count - 1, previewSession.index + offset));
  renderPreview();
}
async function checkEngine() {
  if (busy) return; busy = true; controls(); $('retry-engine').hidden = true;
  try { await invoke('engine_status'); ready = true; message('Ready'); }
  catch (error) { ready = false; message(String(error), 'error'); $('retry-engine').hidden = false; }
  finally { busy = false; render(); }
}
function setMode(next) {
  if (busy || next === mode) return; mode = next; lastSelected = null; const tool = tools[mode];
  document.title = tool.heading; invoke('set_view_title', { tool: mode }).catch(() => {}); $('heading').textContent = tool.heading; $('save-label').textContent = tool.action;
  $('choose-label').textContent = ['merge', 'organize'].includes(mode) ? 'Choose files' : mode === 'compress' ? 'Choose PDFs' : 'Choose a PDF';
  $('list-label').textContent = ['merge', 'compress', 'organize'].includes(mode) ? 'Documents' : 'Document';
  $('split-size').value = current().splitSize; $('split-mode').value = current().splitMode; $('group-size').value = current().groupSize; $('split-ranges').value = current().ranges; $('split-cuts').value = current().splitCuts; $('split-separate').checked = current().splitSeparate; $('split-combine').checked = current().splitCombine;
  $('target-size').value = current().targetSize;
  for (const input of document.querySelectorAll('input[name="compression"]')) input.checked = input.value === current().compression;
  for (const button of document.querySelectorAll('.tool')) { button.classList.toggle('active', button.dataset.mode === mode); button.setAttribute('aria-pressed', String(button.dataset.mode === mode)); }
  $('content-scroll').scrollTop = 0; message(ready ? 'Ready' : 'The local PDF engine is unavailable.', ready ? '' : 'error'); render();
}
for (const button of document.querySelectorAll('.tool')) button.addEventListener('click', () => setMode(button.dataset.mode));
$('add').addEventListener('click', choose); $('choose').addEventListener('click', choose); $('save').addEventListener('click', save);
$('clear').addEventListener('click', async () => {
  if (busy) return; busy = true; invalidateCompression(); current().importErrors = []; current().documents = []; current().pages = []; current().history = []; current().selected.clear(); current().result = null; resetEdits(current()); render();
  try { await cleanRegistry(); message('Documents cleared.'); }
  catch (error) { message(String(error), 'error'); }
  finally { busy = false; controls(); }
});
$('password-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (!pendingUnlock || busy) return;
  const doc = pendingUnlock;
  busy = true; controls();
  let info;
  try {
    const password = $('pdf-password').value; $('pdf-password').value = '';
    info = await invoke('unlock_document', { id: doc.id, password });
  } catch (error) {
    $('password-error').textContent = String(error); $('password-error').hidden = false;
    busy = false; controls(); return;
  }
  for (const [tool, view] of Object.entries(state)) {
    const index = view.documents.findIndex((item) => item.id === info.id);
    if (index === -1) continue;
    const previous = view.documents[index];
    view.documents[index] = info;
    if (tool === 'organize') {
      view.history.push(view.pages.map((page) => ({ ...page }))); view.future = [];
      const added = pagesFromDocuments([info], () => --duplicateKey);
      const position = Math.min(previous.insertionIndex ?? view.pages.length, view.pages.length);
      view.pages = insertPages(view.pages, added, position);
      for (const pending of view.documents) if (pending.locked && pending.insertionIndex >= position) pending.insertionIndex += added.length;
      view.selected = new Set(added.map(pageKey)); view.result = null;
    } else if (tool === 'split') {
      view.pages = Array.from({ length: info.pages }, (_, page) => ({ page: page + 1, rotation: 0 }));
      view.selected.clear(); view.history = []; view.future = []; view.pageWindow = 0; view.result = null;
    }
  }
  await previews.forget(info.id);
  $('password-dialog').close();
  message('PDF unlocked.');
  busy = false; render();
  promptNextLocked();
});
$('password-skip').addEventListener('click', () => $('password-dialog').close());
$('password-dialog').addEventListener('close', () => { pendingUnlock = null; $('pdf-password').value = ''; });
$('password-dialog').addEventListener('cancel', (event) => { if (busy) event.preventDefault(); });
async function fetchUpdate() {
  const update = await invoke('check_app_update');
  pendingUpdate = update; availableUpdateVersion = update?.version || null; updateChecks.remember(availableUpdateVersion); controls(); return update;
}
async function automaticUpdateCheck(installedVersion) {
  availableUpdateVersion = updateChecks.available(installedVersion); controls();
  if (!updateChecks.claim()) return;
  updateChecking = true; controls();
  try { await fetchUpdate(); } catch { /* Offline launches remain quiet. Try again manually or on a later launch. */ }
  finally { updateChecking = false; controls(); }
}
$('app-update').addEventListener('click', async () => {
  if (busy || updateChecking) return;
  clearTimeout(compressionTimer); busy = true; cancelAvailable = false; controls();
  try {
    if (availableUpdateVersion) {
      if (!pendingUpdate) await fetchUpdate();
      if (!pendingUpdate) { message('You have the latest version.'); return; }
      const approved = await invoke('confirm_app_update', { version: pendingUpdate.version });
      if (!approved) return;
      const onProgress = new Channel(); onProgress.onmessage = (progress) => message(progress.message);
      await invoke('install_app_update', { updateId: pendingUpdate.id, onProgress });
    } else {
      message('Checking for updates...');
      await fetchUpdate();
      message(availableUpdateVersion ? `Version ${availableUpdateVersion} is available.` : 'You have the latest version.');
    }
  } catch (error) {
    pendingUpdate = null; message(String(error), String(error).startsWith('No update') ? '' : 'error');
  } finally { busy = false; controls(); scheduleCompression(); }
});
$('release-notes').addEventListener('click', async () => {
  if (busy) return;
  try { await invoke('open_release_notes'); } catch (error) { message(String(error), 'error'); }
});
$('retry-engine').addEventListener('click', checkEngine);
$('cancel').addEventListener('click', async () => {
  if (!jobId) return; const cancelledId = jobId; cancelAvailable = false; controls();
  try { await invoke('cancel_job', { jobId: cancelledId }); if (busy && jobId === cancelledId) message('Cancelling local processing...'); }
  catch (error) { if (busy && jobId === cancelledId) message(String(error), 'error'); }
});
$('reveal').addEventListener('click', async () => { if (!current().result) return; try { await invoke('reveal_result', { resultId: current().result.id }); } catch (error) { message(String(error), 'error'); } });
$('toggle-sidebar').addEventListener('click', () => {
  const narrow = window.matchMedia('(max-width: 680px)').matches;
  const hidden = narrow ? !$('workspace').classList.toggle('sidebar-visible') : $('workspace').classList.toggle('sidebar-hidden');
  $('toggle-sidebar').setAttribute('aria-label', hidden ? 'Show sidebar' : 'Hide sidebar'); $('toggle-sidebar').title = hidden ? 'Show sidebar' : 'Hide sidebar';
});
for (const input of document.querySelectorAll('input[name="compression"]')) input.addEventListener('change', () => { invalidateCompression(); current().compression = input.value; current().result = null; render(); });
$('target-size').addEventListener('input', () => { invalidateCompression(); current().targetSize = $('target-size').value; current().result = null; render(); });
$('compare-output').addEventListener('click', openCompare);
$('flatten-forms').addEventListener('change', () => { current().flattenForms = $('flatten-forms').checked; current().result = null; controls(); });
$('acknowledge-changes').addEventListener('change', () => { invalidateCompression(); current().acknowledgeChanges = $('acknowledge-changes').checked; current().result = null; render(); });
$('split-mode').addEventListener('change', () => { current().splitMode = $('split-mode').value; current().result = null; lastSelected = null; render(); });
$('group-size').addEventListener('input', () => { current().groupSize = $('group-size').value; current().result = null; splitSummary(); });
$('split-size').addEventListener('input', () => { current().splitSize = $('split-size').value; current().result = null; splitSummary(); });
$('split-ranges').addEventListener('input', () => { current().ranges = $('split-ranges').value; current().result = null; readSplitSelection(); splitSummary(); updateSelection(); });
$('split-cuts').addEventListener('input', () => { current().splitCuts = $('split-cuts').value; current().result = null; readSplitSelection(); splitSummary(); updateSelection(); });
$('split-separate').addEventListener('change', () => { current().splitSeparate = $('split-separate').checked; current().result = null; splitSummary(); });
$('split-combine').addEventListener('change', () => { current().splitCombine = $('split-combine').checked; current().result = null; splitSummary(); });
$('add-split-group').addEventListener('click', () => { if (busy || current().splitGroups.length >= 500) return; const view = current(); view.splitGroups.push(''); view.splitNames.push(''); view.activeGroup = view.splitGroups.length - 1; view.result = null; lastSelected = null; render(); $(`split-group-${view.activeGroup}`).focus(); });
$('invert-selection').addEventListener('click', () => { if (busy) return; const view = current(); view.selected = new Set(view.pages.filter((page) => !view.selected.has(pageKey(page))).map(pageKey)); lastSelected = null; writeSplitSelection(); updateSelection(); });
$('select-all').addEventListener('click', () => { if (busy) return; const view = current(); view.selected = view.selected.size === view.pages.length ? new Set() : new Set(view.pages.map(pageKey)); lastSelected = null; if (mode === 'split') writeSplitSelection(); updateSelection(); });
$('rotate-left').addEventListener('click', () => editPages('rotate-left'));
$('duplicate-pages').addEventListener('click', duplicateSelected);
$('move-pages-to').addEventListener('click', () => {
  if (busy || !current().selected.size) return;
  try { const pages = movePagesTo(current().pages, current().selected, Number($('move-page-position').value)); rememberEdit(); current().pages = pages; current().pageWindow = Math.floor((Number($('move-page-position').value) - 1) / PAGE_WINDOW); document.querySelector('.page-more')?.removeAttribute('open'); render(); $('page-grid').querySelector('.selected')?.focus(); message('Page order updated.'); } catch (error) { message(error.message, 'error'); }
});
$('rotate').addEventListener('click', () => editPages('rotate')); $('delete-pages').addEventListener('click', () => editPages('remove'));
$('move-left').addEventListener('click', () => editPages('earlier')); $('move-right').addEventListener('click', () => editPages('later'));
$('undo').addEventListener('click', () => { const view = current(); if (busy || !view.history.length) return; view.future.push(view.pages.map((page) => ({ ...page }))); view.pages = view.history.pop(); view.selected.clear(); lastSelected = null; view.result = null; render(); message('Page edit undone.'); });
$('redo').addEventListener('click', () => { const view = current(); if (busy || !view.future.length) return; view.history.push(view.pages.map((page) => ({ ...page }))); view.pages = view.future.pop(); view.selected.clear(); lastSelected = null; view.result = null; render(); message('Page edit redone.'); });
$('previous-pages').addEventListener('click', () => { if (busy || !current().pageWindow) return; current().pageWindow -= 1; render(); });
$('next-pages').addEventListener('click', () => { if (busy || (current().pageWindow + 1) * PAGE_WINDOW >= current().pages.length) return; current().pageWindow += 1; render(); });
$('jump-page').addEventListener('change', () => {
  const view = current(); const value = Number($('jump-page').value);
  if (!busy && Number.isInteger(value) && value >= 1 && value <= view.pages.length) {
    view.pageWindow = Math.floor((value - 1) / PAGE_WINDOW); render();
    $('page-grid').children[(value - 1) % PAGE_WINDOW]?.focus();
  }
  $('jump-page').value = '';
});
$('thumbnail-size').addEventListener('input', () => { $('page-grid').style.setProperty('--thumb', `${$('thumbnail-size').value}px`); });
$('preview-previous').addEventListener('click', () => movePreview(-1));
$('preview-next').addEventListener('click', () => movePreview(1));
$('preview-page').addEventListener('change', () => {
  if (!previewSession) return;
  const value = Number($('preview-page').value);
  if (Number.isInteger(value) && value >= 1 && value <= previewSession.count) previewSession.index = value - 1;
  renderPreview();
});
$('preview-zoom').addEventListener('change', renderPreview);
$('preview-dialog').addEventListener('keydown', (event) => {
  if (event.target.closest('input, select') || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
  event.preventDefault(); movePreview(event.key === 'ArrowLeft' ? -1 : 1);
});
$('close-preview').addEventListener('click', () => $('preview-dialog').close()); $('preview-dialog').addEventListener('close', () => {
  previewSession = null;
  for (const host of [$('large-preview'), $('compressed-preview')]) { host.previewTicket = null; host.replaceChildren(); }
  $('compressed-pane').hidden = true; $('original-label').hidden = true;
});
document.addEventListener('keydown', (event) => {
  const editing = ['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName);
  if (!busy && mode === 'organize' && !$('preview-dialog').open && !$('password-dialog').open && !editing && event.target.closest('#organizer')) {
    if (['Backspace', 'Delete'].includes(event.key)) { event.preventDefault(); if (!$('delete-pages').disabled) editPages('remove'); return; }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a') { event.preventDefault(); current().selected = new Set(current().pages.map(pageKey)); updateSelection(); return; }
  }
  if (!(event.metaKey || event.ctrlKey) || busy || $('preview-dialog').open || $('password-dialog').open) return;
  if (event.key.toLowerCase() === 'o') { event.preventDefault(); choose(); }
  if (event.key.toLowerCase() === 's') { event.preventDefault(); save(); }
  if (event.key.toLowerCase() === 'z' && mode === 'organize' && !['INPUT', 'TEXTAREA'].includes(event.target.tagName)) { event.preventDefault(); (event.shiftKey ? $('redo') : $('undo')).click(); }
});
$('page-grid').style.setProperty('--thumb', `${$('thumbnail-size').value}px`);
const installedVersion = getVersion().then((version) => { $('app-version').textContent = `Version ${version}`; $('app-version').hidden = false; return version; }).catch(() => null);
render();
Promise.all([checkEngine(), installedVersion]).then(([, version]) => { if (version) automaticUpdateCheck(version); });
