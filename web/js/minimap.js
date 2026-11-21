// Overview of the whole quilt with the current viewport; click or drag to move there.
import { CW, RH } from './view.js';

export class Minimap {
  constructor(canvas, view) {
    this.canvas = canvas;
    this.view = view;
    this.ctx = canvas.getContext('2d');
    this.off = document.createElement('canvas');
    this.dragging = false;
    view.addEventListener('quilt', () => this.render());
    view.addEventListener('view', () => this.draw());
    const at = (e) => {
      const r = canvas.getBoundingClientRect();
      const q = view.q;
      if (!q) return;
      const wx = ((e.clientX - r.left) / r.width) * q.m * CW;
      const wy = ((e.clientY - r.top) / r.height) * q.n * RH;
      view.centerOn(wx, wy, false);
    };
    canvas.addEventListener('pointerdown', (e) => {
      canvas.setPointerCapture(e.pointerId);
      this.dragging = true;
      at(e);
    });
    canvas.addEventListener('pointermove', (e) => this.dragging && at(e));
    canvas.addEventListener('pointerup', () => (this.dragging = false));
    canvas.addEventListener('pointercancel', () => (this.dragging = false));
    this.render();
  }

  render() {
    const q = this.view.q;
    const W = this.canvas.width;
    const H = this.canvas.height;
    this.off.width = W;
    this.off.height = H;
    const g = this.off.getContext('2d');
    const C = this.view.colors;
    g.fillStyle = C.bg;
    g.fillRect(0, 0, W, H);
    if (q && q.n) {
      const sx = W / (q.m * CW);
      const sy = H / (q.n * RH);
      g.fillStyle = C.fg;
      const w = Math.max(1, CW * sx * 0.6);
      const h = Math.max(1, RH * sy * 0.6);
      for (let j = 0; j < q.m; j++) {
        const col = q.cols[j];
        for (const r of col.p) g.fillRect(j * CW * sx, r * RH * sy, w, h);
        for (const c of col.c) g.fillRect(j * CW * sx, c.r * RH * sy, w, h);
      }
    }
    this.draw();
  }

  draw() {
    const q = this.view.q;
    const { ctx, canvas } = this;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(this.off, 0, 0);
    if (!q || !q.n) return;
    const v = this.view.visible();
    const sx = canvas.width / (q.m * CW);
    const sy = canvas.height / (q.n * RH);
    const x0 = Math.max(0, v.x0 * sx);
    const y0 = Math.max(0, v.y0 * sy);
    const x1 = Math.min(canvas.width, v.x1 * sx);
    const y1 = Math.min(canvas.height, v.y1 * sy);
    ctx.strokeStyle = this.view.colors.focus;
    ctx.lineWidth = 2;
    ctx.fillStyle = this.view.colors.focus;
    ctx.globalAlpha = 0.15;
    ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
    ctx.globalAlpha = 1;
    ctx.strokeRect(x0 + 1, y0 + 1, Math.max(2, x1 - x0 - 2), Math.max(2, y1 - y0 - 2));
  }
}
