// Pointer-based reordering remains available when native file drops are enabled.
export function reorderable(element, index, handle, isBusy, move, { onStart } = {}) {
  element.dataset.reorderIndex = String(index);
  handle.classList.add('reorder-handle');
  let suppressClick = false;
  element.addEventListener('click', (event) => {
    if (suppressClick) { event.preventDefault(); event.stopImmediatePropagation(); suppressClick = false; }
  }, true);
  handle.addEventListener('pointerdown', (start) => {
    if (start.button !== 0 || isBusy() || start.target.closest('button,input') || document.body.classList.contains('reordering')) return;
    let moving = false; let target = null; let after = false; let ghost = null; let frame = null;
    let x = start.clientX; let y = start.clientY; let lastTime = performance.now();
    const group = element.parentElement; const scroll = element.closest('.content-scroll');
    const markTarget = () => {
      target?.classList.remove('drag-target', 'drag-after');
      const candidate = document.elementFromPoint(x, y)?.closest('[data-reorder-index]');
      target = candidate?.parentElement === group ? candidate : null;
      const rect = target?.getBoundingClientRect();
      after = rect ? (element.classList.contains('page-card') ? x > rect.left + rect.width / 2 : y > rect.top + rect.height / 2) : false;
      if (target && !target.classList.contains('drag-source')) { target.classList.add('drag-target'); target.classList.toggle('drag-after', after); }
    };
    const tick = (time) => {
      if (!moving) return;
      if (scroll) {
        const rect = scroll.getBoundingClientRect(); const edge = 64; const elapsed = Math.min(time - lastTime, 40);
        const speed = y < rect.top + edge ? -Math.min(1, (rect.top + edge - y) / edge) : y > rect.bottom - edge ? Math.min(1, (y - rect.bottom + edge) / edge) : 0;
        if (speed) { scroll.scrollTop += speed * elapsed * .65; markTarget(); }
      }
      lastTime = time; frame = requestAnimationFrame(tick);
    };
    const clean = () => {
      target?.classList.remove('drag-target', 'drag-after'); ghost?.remove();
      for (const item of group.querySelectorAll('.drag-source')) item.classList.remove('drag-source');
      document.body.classList.remove('reordering'); if (frame !== null) cancelAnimationFrame(frame);
      if (handle.hasPointerCapture(start.pointerId)) handle.releasePointerCapture(start.pointerId);
      document.removeEventListener('pointermove', pointerMove, true);
      document.removeEventListener('pointerup', pointerUp, true);
      document.removeEventListener('pointercancel', pointerCancel, true);
      document.removeEventListener('keydown', keyDown, true); window.removeEventListener('blur', pointerCancel);
    };
    const pointerMove = (event) => {
      if (event.pointerId !== start.pointerId) return;
      x = event.clientX; y = event.clientY;
      if (!moving && Math.hypot(x - start.clientX, y - start.clientY) < 6) return;
      event.preventDefault();
      if (!moving) {
        moving = true; const count = onStart?.() || 1; document.body.classList.add('reordering'); handle.setPointerCapture(start.pointerId);
        if (element.classList.contains('page-card')) {
          ghost = document.createElement('div'); ghost.className = 'drag-ghost'; ghost.setAttribute('aria-hidden', 'true');
          const paper = element.querySelector('.page-paper').cloneNode(true);
          const sourceCanvas = element.querySelector('canvas'); const canvas = paper.querySelector('canvas');
          if (canvas && sourceCanvas) canvas.getContext('2d').drawImage(sourceCanvas, 0, 0);
          const label = document.createElement('div'); label.className = 'drag-count'; label.textContent = `${count} ${count === 1 ? 'page' : 'pages'}`; ghost.append(paper, label); document.body.append(ghost);
          for (const item of group.querySelectorAll('.selected')) item.classList.add('drag-source');
        } else element.classList.add('drag-source');
        lastTime = performance.now(); frame = requestAnimationFrame(tick);
      }
      if (ghost) { ghost.style.left = `${x + 14}px`; ghost.style.top = `${y + 14}px`; }
      markTarget();
    };
    const pointerUp = (event) => {
      if (event.pointerId !== start.pointerId) return;
      const destination = target ? Number(target.dataset.reorderIndex) : index; const placement = after;
      clean();
      if (moving) {
        suppressClick = true; setTimeout(() => { suppressClick = false; }, 0);
        if (!isBusy() && destination !== index) move(index, destination, placement);
      }
    };
    const pointerCancel = () => clean();
    const keyDown = (event) => { if (event.key === 'Escape') { event.preventDefault(); pointerCancel(); } };
    document.addEventListener('pointermove', pointerMove, { capture: true, passive: false });
    document.addEventListener('pointerup', pointerUp, true); document.addEventListener('pointercancel', pointerCancel, true);
    document.addEventListener('keydown', keyDown, true); window.addEventListener('blur', pointerCancel);
  });
}
