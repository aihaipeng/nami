export const VIRTUALIZE_THRESHOLD = 100;
export const DEFAULT_ROW_HEIGHT = 72;
export const VIRTUAL_OVERSCAN_PX = 480;
export const VIRTUAL_OVERSCAN_ROWS = 6;
export const ROW_GAP_PX = 4;

export interface VirtualWindowParams {
  ids: readonly string[];
  heights: ReadonlyMap<string, number>;
  fallbackHeight?: number;
  scrollTop: number;
  viewportHeight: number;
  threshold?: number;
  overscanPx?: number;
  overscanRows?: number;
}

export interface VirtualWindow {
  virtualized: boolean;
  start: number;
  end: number;
  paddingTop: number;
  paddingBottom: number;
  totalHeight: number;
}

export function averageRowHeight(heights: ReadonlyMap<string, number>, fallback = DEFAULT_ROW_HEIGHT): number {
  if (heights.size === 0) return fallback;
  let sum = 0;
  let count = 0;
  for (const value of heights.values()) {
    if (value > 0 && Number.isFinite(value)) {
      sum += value;
      count += 1;
    }
  }
  if (count === 0) return fallback;
  return Math.min(240, Math.max(36, Math.round(sum / count)));
}

export function buildOffsets(
  ids: readonly string[],
  heights: ReadonlyMap<string, number>,
  fallbackHeight = DEFAULT_ROW_HEIGHT,
): Float64Array {
  const offsets = new Float64Array(ids.length + 1);
  let cursor = 0;
  for (let i = 0; i < ids.length; i += 1) {
    offsets[i] = cursor;
    const height = heights.get(ids[i]);
    cursor += height && height > 0 ? height : fallbackHeight;
  }
  offsets[ids.length] = cursor;
  return offsets;
}

export function findIndexAtOffset(offsets: Float64Array, target: number): number {
  const count = offsets.length - 1;
  if (count <= 0) return 0;
  if (target <= 0) return 0;
  if (target >= offsets[count]) return count - 1;
  let low = 0;
  let high = count - 1;
  while (low < high) {
    const mid = (low + high) >>> 1;
    if (offsets[mid + 1] <= target) low = mid + 1;
    else high = mid;
  }
  return low;
}

export function computeVirtualWindow({
  ids,
  heights,
  fallbackHeight = DEFAULT_ROW_HEIGHT,
  scrollTop,
  viewportHeight,
  threshold = VIRTUALIZE_THRESHOLD,
  overscanPx = VIRTUAL_OVERSCAN_PX,
  overscanRows = VIRTUAL_OVERSCAN_ROWS,
}: VirtualWindowParams): VirtualWindow {
  if (ids.length <= threshold || viewportHeight <= 0) {
    return { virtualized: false, start: 0, end: ids.length, paddingTop: 0, paddingBottom: 0, totalHeight: 0 };
  }
  const offsets = buildOffsets(ids, heights, fallbackHeight);
  const clampedTop = Math.max(0, scrollTop);
  const topEdge = Math.max(0, clampedTop - overscanPx);
  const bottomEdge = clampedTop + viewportHeight + overscanPx;
  const visibleStart = findIndexAtOffset(offsets, clampedTop);
  const visibleEnd = findIndexAtOffset(offsets, clampedTop + viewportHeight);
  const start = Math.max(0, Math.min(findIndexAtOffset(offsets, topEdge), visibleStart - overscanRows));
  const end = Math.min(ids.length, Math.max(findIndexAtOffset(offsets, bottomEdge) + 1, visibleEnd + 1 + overscanRows));
  const totalHeight = offsets[ids.length];
  return {
    virtualized: true,
    start,
    end,
    paddingTop: offsets[start],
    paddingBottom: Math.max(0, totalHeight - offsets[end]),
    totalHeight,
  };
}

export function computeScrollTargetForIndex(
  offsets: Float64Array,
  index: number,
  scrollTop: number,
  viewportHeight: number,
  listOffsetTop = 0,
  margin = 4,
): number | null {
  const count = offsets.length - 1;
  if (index < 0 || index >= count || viewportHeight <= 0) return null;
  const itemTop = listOffsetTop + offsets[index];
  const itemBottom = listOffsetTop + offsets[index + 1];
  if (itemTop < scrollTop + margin) return Math.max(0, itemTop - margin);
  if (itemBottom > scrollTop + viewportHeight - margin) return Math.max(0, itemBottom - viewportHeight + margin);
  return null;
}
