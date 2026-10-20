import type { Line } from '../../web/protocol.js';
export type Point = [number, number];
export function pagePoint(x: number, y: number, rect: { left: number; top: number; width: number; height: number }, width: number, height: number): Point {
  return [Math.max(0, Math.min(width, (x - rect.left) / rect.width * width)), Math.max(0, Math.min(height, (y - rect.top) / rect.height * height))];
}
export function highlightQuads(lines: Line[], start: Point, end: Point): number[][] {
  const left = Math.min(start[0], end[0]), right = Math.max(start[0], end[0]);
  const top = Math.min(start[1], end[1]), bottom = Math.max(start[1], end[1]);
  return lines.filter(line => line.x <= right && line.x + line.w >= left && line.y <= bottom && line.y + line.h >= top)
    .slice(0, 64).map(line => [line.x, line.y, line.x + line.w, line.y, line.x, line.y + line.h, line.x + line.w, line.y + line.h]);
}
export function placeSignature(strokes: Point[][], at: Point, pageWidth: number, pageHeight: number): Point[][] {
  const points = strokes.flat();
  if (!points.length) return [];
  const xs = points.map(p => p[0]), ys = points.map(p => p[1]);
  const x0 = Math.min(...xs), y0 = Math.min(...ys);
  const w = Math.max(1, Math.max(...xs) - x0), h = Math.max(1, Math.max(...ys) - y0);
  const scale = Math.min(180 / w, 70 / h, (pageWidth - 2) / w, (pageHeight - 2) / h);
  const x = Math.max(1, Math.min(pageWidth - w * scale - 1, at[0] - w * scale / 2));
  const y = Math.max(1, Math.min(pageHeight - h * scale - 1, at[1] - h * scale / 2));
  return strokes.map(stroke => stroke.map(p => [x + (p[0] - x0) * scale, y + (p[1] - y0) * scale]));
}
