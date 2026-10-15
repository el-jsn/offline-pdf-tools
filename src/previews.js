import { invoke } from '@tauri-apps/api/core';
import { getDocument, GlobalWorkerOptions } from 'pdfjs-dist/legacy/build/pdf.mjs';
import workerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url';
GlobalWorkerOptions.workerSrc = workerUrl;

const CHUNK = 1024 * 1024;

export class Previews {
  constructor() {
    this.cache = new Map(); this.queue = []; this.active = 0; this.generation = 0; this.renders = new Set(); this.activeIds = new Map();
    this.observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        this.observer.unobserve(entry.target);
        this.queue.push({ ...entry.target.previewSpec, host: entry.target, generation: this.generation });
      }
      this.pump();
    }, { root: document.getElementById('content-scroll'), rootMargin: '100px' });
  }
  resetView() {
    this.generation += 1; this.observer.disconnect(); this.queue = [];
    for (const task of this.renders) task.cancel();
    this.renders.clear();
  }
  async forget(key) {
    const entry = this.cache.get(key); this.cache.delete(key);
    if (entry) {
      try { await entry.promise; } catch { /* Failed loading tasks can still be destroyed. */ }
      await entry.task?.destroy();
    }
  }
  watch(host, id, page = 1, rotation = 0, bytes = 0) {
    host.previewSpec = { id, page, rotation, width: 150, bytes }; this.observer.observe(host);
  }
  key(spec) { return spec.previewId ? `preview:${spec.previewId}:${spec.outputIndex ?? 0}` : spec.id; }
  async readSource(spec) {
    const total = Math.max(0, Number(spec.bytes) || 0);
    const parts = [];
    for (let begin = 0; begin < total; begin += CHUNK) {
      const end = Math.min(begin + CHUNK, total);
      const chunk = await invoke('read_document_range', { id: spec.id || '', begin, end, previewId: spec.previewId || null, outputIndex: spec.outputIndex ?? null });
      parts.push(new Uint8Array(chunk));
    }
    const data = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) { data.set(part, offset); offset += part.length; }
    return data;
  }
  async pdf(spec) {
    const key = this.key(spec);
    if (this.cache.has(key)) return this.cache.get(key).promise;
    // Bound decoded-document retention; canvases already drawn remain usable.
    if (this.cache.size >= 3) {
      const oldest = [...this.cache.keys()].find((candidate) => !this.activeIds.has(candidate));
      if (oldest !== undefined) await this.forget(oldest);
    }
    if (this.cache.has(key)) return this.cache.get(key).promise;
    const entry = {}; this.cache.set(key, entry);
    entry.promise = (async () => {
      const data = await this.readSource(spec);
      const assets = new URL('/pdfjs/', window.location.href).href;
      entry.task = getDocument({
        data, isEvalSupported: false, verbosity: 0,
        cMapUrl: `${assets}cmaps/`, cMapPacked: true, iccUrl: `${assets}iccs/`,
        standardFontDataUrl: `${assets}standard_fonts/`, wasmUrl: `${assets}wasm/`,
        useSystemFonts: true, enableXfa: false,
      });
      return entry.task.promise;
    })();
    try { return await entry.promise; } catch (error) { this.cache.delete(key); throw error; }
  }
  async draw({ host, generation = this.generation, ...spec }) {
    const ticket = Symbol(); host.previewTicket = ticket;
    const key = this.key(spec);
    this.activeIds.set(key, (this.activeIds.get(key) || 0) + 1);
    try {
      const pdf = await this.pdf(spec);
      if (!host.isConnected || generation !== this.generation || host.previewTicket !== ticket) return;
      const pdfPage = await pdf.getPage(spec.page);
      if (!host.isConnected || generation !== this.generation || host.previewTicket !== ticket) return;
      const angle = (pdfPage.rotate + (spec.rotation || 0)) % 360;
      const width = spec.width || 150;
      const initial = pdfPage.getViewport({ scale: 1, rotation: angle });
      const scale = Math.min(width / initial.width, spec.height > 0 ? spec.height / initial.height : Infinity);
      const viewport = pdfPage.getViewport({ scale, rotation: angle });
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const canvas = document.createElement('canvas');
      canvas.width = Math.ceil(viewport.width * ratio); canvas.height = Math.ceil(viewport.height * ratio);
      canvas.style.width = `${viewport.width}px`; canvas.style.height = `${viewport.height}px`;
      const task = pdfPage.render({ canvas, canvasContext: canvas.getContext('2d'), viewport,
        transform: ratio === 1 ? undefined : [ratio, 0, 0, ratio, 0, 0], annotationMode: 0 });
      this.renders.add(task);
      try { await task.promise; } finally { this.renders.delete(task); }
      if (host.isConnected && generation === this.generation && host.previewTicket === ticket) host.replaceChildren(canvas);
    } catch (error) {
      if (error?.name === 'RenderingCancelledException' || !host.isConnected || generation !== this.generation || host.previewTicket !== ticket) return;
      const note = document.createElement('span'); note.className = 'preview-unavailable'; note.textContent = 'Preview unavailable';
      host.replaceChildren(note); host.title = 'The preview could not be rendered. PDF processing is still available.';
    } finally {
      const remaining = this.activeIds.get(key) - 1;
      if (remaining) this.activeIds.set(key, remaining); else this.activeIds.delete(key);
    }
  }
  pump() {
    while (this.active < 2 && this.queue.length) {
      const item = this.queue.shift(); this.active += 1;
      this.draw(item).finally(() => { this.active -= 1; this.pump(); });
    }
  }
  async releaseUnused(keys) {
    const keep = new Set(keys);
    for (const key of [...this.cache.keys()]) if (!keep.has(key)) await this.forget(key);
  }
}
