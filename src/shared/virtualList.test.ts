import { describe, expect, it } from 'vitest';
import {
  averageRowHeight,
  buildOffsets,
  computeScrollTargetForIndex,
  computeVirtualWindow,
  DEFAULT_ROW_HEIGHT,
  findIndexAtOffset,
  VIRTUALIZE_THRESHOLD,
} from './virtualList';

describe('virtualList', () => {
  it('retains small lists and bounds the first slice of an unmeasured deep list', () => {
    const ids = Array.from({ length: VIRTUALIZE_THRESHOLD }, (_, i) => `row-${i}`);
    const heights = new Map<string, number>();
    expect(computeVirtualWindow({ ids, heights, scrollTop: 0, viewportHeight: 400 })).toEqual({
      virtualized: false,
      start: 0,
      end: VIRTUALIZE_THRESHOLD,
      visibleStart: 0,
      paddingTop: 0,
      paddingBottom: 0,
      totalHeight: 0,
    });

    const deepIds = Array.from({ length: VIRTUALIZE_THRESHOLD + 50 }, (_, i) => `row-${i}`);
    const first = computeVirtualWindow({ ids: deepIds, heights, scrollTop: 0, viewportHeight: 0 });
    expect(first.virtualized).toBe(true);
    expect(first.start).toBe(0);
    expect(first.end).toBeGreaterThan(0);
    expect(first.end).toBeLessThan(VIRTUALIZE_THRESHOLD);
    expect(first.paddingBottom).toBe((deepIds.length - first.end) * DEFAULT_ROW_HEIGHT);
  });

  it('windows deep variable-height lists accurately with bounded DOM slice and exact total height', () => {
    const ids = Array.from({ length: 500 }, (_, i) => `row-${i}`);
    const heights = new Map<string, number>();
    for (let i = 0; i < ids.length; i += 1) {
      heights.set(ids[i], i % 5 === 0 ? 110 : 60);
    }
    const offsets = buildOffsets(ids, heights, DEFAULT_ROW_HEIGHT);
    const totalHeight = offsets[ids.length];

    const topWindow = computeVirtualWindow({ ids, heights, scrollTop: 0, viewportHeight: 400 });
    expect(topWindow.virtualized).toBe(true);
    expect(topWindow.start).toBe(0);
    expect(topWindow.end).toBeLessThan(40);
    expect(topWindow.paddingTop).toBe(0);
    expect(topWindow.paddingTop + (offsets[topWindow.end] - offsets[topWindow.start]) + topWindow.paddingBottom).toBe(totalHeight);

    const midScroll = offsets[250];
    const midWindow = computeVirtualWindow({ ids, heights, scrollTop: midScroll, viewportHeight: 400 });
    expect(midWindow.virtualized).toBe(true);
    expect(midWindow.start).toBeLessThan(250);
    expect(midWindow.visibleStart).toBe(250);
    expect(midWindow.end).toBeGreaterThan(255);
    expect(midWindow.end - midWindow.start).toBeLessThan(40);
    expect(midWindow.paddingTop).toBe(offsets[midWindow.start]);
    expect(midWindow.paddingBottom).toBe(totalHeight - offsets[midWindow.end]);

    const bottomWindow = computeVirtualWindow({ ids, heights, scrollTop: totalHeight - 400, viewportHeight: 400 });
    expect(bottomWindow.end).toBe(500);
    expect(bottomWindow.paddingBottom).toBe(0);
    expect(bottomWindow.start).toBeGreaterThan(450);
  });

  it('finds offsets and computes scroll targets for off-screen rows including pinned offset', () => {
    const ids = Array.from({ length: 200 }, (_, i) => `row-${i}`);
    const heights = new Map<string, number>([['row-0', 100], ['row-1', 50]]);
    const offsets = buildOffsets(ids, heights, 70);
    expect(findIndexAtOffset(offsets, 0)).toBe(0);
    expect(findIndexAtOffset(offsets, 99)).toBe(0);
    expect(findIndexAtOffset(offsets, 100)).toBe(1);
    expect(findIndexAtOffset(offsets, 149)).toBe(1);
    expect(findIndexAtOffset(offsets, 150)).toBe(2);
    expect(findIndexAtOffset(offsets, offsets[200] + 500)).toBe(199);

    // Row 10 is below viewport [0..400]
    const targetDown = computeScrollTargetForIndex(offsets, 10, 0, 400, 0);
    expect(targetDown).toBe(offsets[11] - 400 + 4);
    // Row 10 is already visible when scrolled to targetDown
    expect(computeScrollTargetForIndex(offsets, 10, targetDown!, 400, 0)).toBeNull();
    // Row 0 is above viewport when scrolled to 500
    expect(computeScrollTargetForIndex(offsets, 0, 500, 400, 0)).toBe(0);
    // Pinned offset shifts item position down by 120px
    expect(computeScrollTargetForIndex(offsets, 0, 500, 400, 120)).toBe(116);
  });

  it('computes bounded average row height safely', () => {
    expect(averageRowHeight(new Map())).toBe(DEFAULT_ROW_HEIGHT);
    expect(averageRowHeight(new Map([['a', 0], ['b', -10], ['c', Number.NaN]]))).toBe(DEFAULT_ROW_HEIGHT);
    expect(averageRowHeight(new Map([['a', 10]]))).toBe(36);
    expect(averageRowHeight(new Map([['a', 500]]))).toBe(240);
    expect(averageRowHeight(new Map([['a', 60], ['b', 100]]))).toBe(80);
  });
});
