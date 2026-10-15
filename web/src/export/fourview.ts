// Four-view sheet: layout math, edge detection from normal/depth buffers, and
// Canvas 2D composition. Conventions follow final-plan.md: 2x2 Front | Right /
// Top | Iso, all orthographic, one shared px/mm scale, 4096 sheet with 64 px
// margins/gutters and 96 px label bands (scaled proportionally for 2048).

import type { Studio } from '../render/studio';
import type { LayerSettings, MaterialSettings } from '../state/settings';

export type PanelView = 'front' | 'right' | 'top' | 'iso';

export const PANEL_ORDER: PanelView[] = ['front', 'right', 'top', 'iso'];
export const PANEL_LABELS: Record<PanelView, string> = { front: 'FRONT', right: 'RIGHT', top: 'TOP', iso: 'ISOMETRIC' };

export interface SheetLayout {
  size: number;
  margin: number;
  gutter: number;
  labelBand: number;
  cellW: number;
  imageW: number;
  imageH: number;
  /** Top-left corner of each panel image, in PANEL_ORDER. */
  origins: [number, number][];
}

export function sheetLayout(size: 2048 | 4096 | number): SheetLayout {
  const k = size / 4096;
  const margin = Math.round(64 * k), gutter = Math.round(64 * k), labelBand = Math.round(96 * k);
  const cellW = Math.floor((size - 2 * margin - gutter) / 2);
  const imageW = cellW, imageH = cellW - labelBand;
  const origins: [number, number][] = [
    [margin, margin],
    [margin + cellW + gutter, margin],
    [margin, margin + cellW + gutter],
    [margin + cellW + gutter, margin + cellW + gutter],
  ];
  return { size, margin, gutter, labelBand, cellW, imageW, imageH, origins };
}

export interface EdgeOptions {
  /** 1 - dot(n1, n2) above which a crease line is drawn (0.1 ≈ 26°). */
  normalThreshold: number;
  /** Depth jump (mm) that always counts as an edge. */
  depthThresholdMm: number;
  /** Line half-width in pixels (dilation radius). */
  radius: number;
}

/**
 * Edge strength (0..1) per pixel from an RGBA float buffer holding view-space
 * normal (xyz) and depth in mm (w). Background pixels have w <= 0. Silhouettes
 * come from coverage changes, creases from normal changes, steps from depth
 * jumps that a plane cannot explain (second difference).
 */
export function detectEdges(nd: Float32Array, width: number, height: number, opts: EdgeOptions): Float32Array {
  const raw = new Uint8Array(width * height);
  const at = (x: number, y: number) => (y * width + x) * 4;
  for (let y = 1; y < height - 1; y++) {
    for (let x = 1; x < width - 1; x++) {
      const p = at(x, y);
      const bg = nd[p + 3] <= 0;
      let edge = false;
      for (const [dx, dy] of [[1, 0], [0, 1]] as const) {
        const q = at(x + dx, y + dy);
        const qbg = nd[q + 3] <= 0;
        if (bg !== qbg) { edge = true; break; }
        if (bg) continue;
        const d = 1 - (nd[p] * nd[q] + nd[p + 1] * nd[q + 1] + nd[p + 2] * nd[q + 2]);
        if (d > opts.normalThreshold) { edge = true; break; }
        const r = at(x - dx, y - dy);
        if (nd[r + 3] > 0) {
          const second = Math.abs(nd[q + 3] + nd[r + 3] - 2 * nd[p + 3]);
          if (second > opts.depthThresholdMm) { edge = true; break; }
        }
      }
      if (edge) raw[y * width + x] = 1;
    }
  }
  // Dilate with a soft round brush for even, slightly antialiased lines.
  const out = new Float32Array(width * height);
  const R = opts.radius;
  const ri = Math.ceil(R + 1);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (!raw[y * width + x]) continue;
      for (let dy = -ri; dy <= ri; dy++) {
        const yy = y + dy;
        if (yy < 0 || yy >= height) continue;
        for (let dx = -ri; dx <= ri; dx++) {
          const xx = x + dx;
          if (xx < 0 || xx >= width) continue;
          const dist = Math.hypot(dx, dy);
          const s = Math.min(1, Math.max(0, R + 1 - dist));
          const o = yy * width + xx;
          if (s > out[o]) out[o] = s;
        }
      }
    }
  }
  return out;
}

export function compositeEdges(rgba: Uint8ClampedArray, edges: Float32Array, color: [number, number, number], opacity = 1) {
  for (let i = 0; i < edges.length; i++) {
    const e = edges[i] * opacity;
    if (e <= 0) continue;
    const o = i * 4;
    rgba[o] = rgba[o] + (color[0] - rgba[o]) * e;
    rgba[o + 1] = rgba[o + 1] + (color[1] - rgba[o + 1]) * e;
    rgba[o + 2] = rgba[o + 2] + (color[2] - rgba[o + 2]) * e;
    rgba[o + 3] = Math.max(rgba[o + 3], 255 * e);
  }
}

/** A "nice" scale-bar length (1/2/5 × 10^n mm) about a fifth of the panel. */
export function scaleBarMm(mmPerPx: number, panelWidthPx: number): number {
  const target = mmPerPx * panelWidthPx * 0.2;
  const p = Math.pow(10, Math.floor(Math.log10(target)));
  for (const m of [5, 2, 1]) if (m * p <= target) return m * p;
  return p;
}

export interface SheetMeta {
  title: string;
  sizeMm: [number, number, number];
  mmPerPx: number;
  units: string;
  extra?: string;
}

export async function composeSheet(panels: Uint8ClampedArray[], layout: SheetLayout, meta: SheetMeta, background = '#ffffff'): Promise<Blob> {
  const { size, imageW, imageH, labelBand, origins } = layout;
  const canvas = new OffscreenCanvas(size, size);
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('2D canvas unavailable');
  ctx.fillStyle = background;
  ctx.fillRect(0, 0, size, size);
  const k = size / 4096;

  panels.forEach((px, i) => {
    const [x, y] = origins[i];
    const tile = new OffscreenCanvas(imageW, imageH);
    tile.getContext('2d')!.putImageData(new ImageData(px as Uint8ClampedArray<ArrayBuffer>, imageW, imageH), 0, 0);
    ctx.drawImage(tile, x, y);
    // Label band
    ctx.fillStyle = '#1d1f24';
    ctx.font = `600 ${Math.round(40 * k)}px system-ui, "Segoe UI", sans-serif`;
    ctx.textBaseline = 'middle';
    const ly = y + imageH + labelBand / 2;
    ctx.fillText(PANEL_LABELS[PANEL_ORDER[i]], x + Math.round(8 * k), ly);
    ctx.strokeStyle = '#c9c6c0';
    ctx.lineWidth = Math.max(1, 2 * k);
    ctx.beginPath();
    ctx.moveTo(x, y + imageH + 1);
    ctx.lineTo(x + imageW, y + imageH + 1);
    ctx.stroke();
  });

  // Metadata in the Isometric label band, right-aligned.
  const [ix, iy] = origins[3];
  ctx.fillStyle = '#5b5f68';
  ctx.font = `${Math.round(30 * k)}px system-ui, "Segoe UI", sans-serif`;
  ctx.textAlign = 'right';
  const dims = meta.sizeMm.map((v) => v.toFixed(1)).join(' × ');
  ctx.fillText(`${meta.title} · ${dims} ${meta.units}${meta.extra ? ` · ${meta.extra}` : ''}`, ix + imageW - Math.round(8 * k), iy + imageH + labelBand / 2);
  ctx.textAlign = 'left';

  // Shared-scale bar in the Front panel's label band, right-aligned.
  const [fx, fy] = origins[0];
  const barMm = scaleBarMm(meta.mmPerPx, imageW);
  const barPx = barMm / meta.mmPerPx;
  const by = fy + imageH + labelBand / 2;
  const bx1 = fx + imageW - Math.round(16 * k);
  const bx0 = bx1 - barPx;
  ctx.strokeStyle = '#1d1f24';
  ctx.lineWidth = Math.max(2, 4 * k);
  ctx.beginPath();
  ctx.moveTo(bx0, by);
  ctx.lineTo(bx1, by);
  ctx.moveTo(bx0, by - 12 * k);
  ctx.lineTo(bx0, by + 12 * k);
  ctx.moveTo(bx1, by - 12 * k);
  ctx.lineTo(bx1, by + 12 * k);
  ctx.stroke();
  ctx.fillStyle = '#1d1f24';
  ctx.font = `${Math.round(30 * k)}px system-ui, "Segoe UI", sans-serif`;
  ctx.textAlign = 'right';
  ctx.fillText(`${barMm} ${meta.units}`, bx0 - Math.round(14 * k), by);
  ctx.textAlign = 'left';

  return canvas.convertToBlob({ type: 'image/png' });
}

// ------------------------------------------------------------ orchestration


export interface FourViewOptions {
  size: 2048 | 4096;
  supersample: number;
  edges: boolean;
  showLayers: boolean;
  useModelMaterial: boolean;
}

export const FOURVIEW_DEFAULTS: FourViewOptions = { size: 4096, supersample: 2, edges: true, showLayers: false, useModelMaterial: false };

export const DIAGRAM_MATERIAL: MaterialSettings = {
  preset: 'diagram', color: '#c9cbcf', roughness: 0.62, metalness: 0, clearcoat: 0, clearcoatRoughness: 0.1, sheen: 0,
  transmission: 0, thickness: 1, attenuationDistance: 100, ior: 1.5,
};

export async function exportFourView(
  studio: Studio,
  o: FourViewOptions,
  current: { material: MaterialSettings; layers: LayerSettings; name: string },
  onProgress: (done: number, total: number) => void,
): Promise<{ blob: Blob; ms: number; mmPerPx: number }> {
  const t0 = performance.now();
  const layout = sheetLayout(o.size);
  onProgress(0, 6);
  await new Promise((r) => setTimeout(r, 30));
  const panels = studio.renderFourView({
    imageW: layout.imageW,
    imageH: layout.imageH,
    supersample: o.supersample,
    material: o.useModelMaterial ? current.material : DIAGRAM_MATERIAL,
    layers: { ...current.layers, enabled: o.showLayers && current.layers.enabled },
    background: '#ffffff',
    margin: 0.1,
  });
  onProgress(4, 6);
  if (o.edges) {
    for (let i = 0; i < 4; i++) {
      const edges = detectEdges(panels.normalDepth[i], layout.imageW, layout.imageH, {
        // G-code beads would outline every groove; use silhouettes and depth steps only.
        normalThreshold: studio.modelKind === 'beads' ? 2 : 0.12,
        depthThresholdMm: panels.mmPerPx * 8,
        radius: Math.max(0.6, layout.imageW / 1600),
      });
      compositeEdges(panels.shaded[i], edges, [38, 40, 46], 0.9);
      await new Promise((r) => setTimeout(r, 0));
    }
  }
  onProgress(5, 6);
  const blob = await composeSheet(panels.shaded, layout, {
    title: current.name,
    sizeMm: panels.sizeMm,
    mmPerPx: panels.mmPerPx,
    units: 'mm',
  });
  onProgress(6, 6);
  return { blob, ms: performance.now() - t0, mmPerPx: panels.mmPerPx };
}
