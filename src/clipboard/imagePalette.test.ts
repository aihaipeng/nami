import { describe, expect, it } from 'vitest';
import { extractPaletteFromRgba } from './imagePalette';

function makeRgba(segments: Array<{ rgb: [number, number, number]; alpha?: number; count: number }>): Uint8ClampedArray {
  const total = segments.reduce((sum, item) => sum + item.count, 0);
  const data = new Uint8ClampedArray(total * 4);
  let offset = 0;
  for (const { rgb: [r, g, b], alpha = 255, count } of segments) {
    for (let i = 0; i < count; i += 1) {
      data[offset] = r;
      data[offset + 1] = g;
      data[offset + 2] = b;
      data[offset + 3] = alpha;
      offset += 4;
    }
  }
  return data;
}

describe('extractPaletteFromRgba', () => {
  it('returns empty array for empty or fully transparent images', () => {
    expect(extractPaletteFromRgba(new Uint8ClampedArray(0))).toEqual([]);
    expect(extractPaletteFromRgba(makeRgba([{ rgb: [255, 0, 0], alpha: 0, count: 100 }]))).toEqual([]);
    expect(extractPaletteFromRgba(makeRgba([{ rgb: [0, 255, 0], alpha: 100, count: 100 }]))).toEqual([]);
  });

  it('returns a single exact hex color for a solid color image', () => {
    const data = makeRgba([{ rgb: [14, 165, 233], count: 48 * 48 }]);
    expect(extractPaletteFromRgba(data)).toEqual(['#0EA5E9']);
  });

  it('ignores transparent background pixels and avoids fake antialias noise on flat graphics', () => {
    const data = makeRgba([
      { rgb: [0, 0, 0], alpha: 0, count: 1500 },
      { rgb: [239, 68, 68], count: 500 },
      { rgb: [255, 255, 255], count: 300 },
      { rgb: [247, 160, 160], count: 1 }, // stray single-pixel antialias noise
    ]);
    expect(extractPaletteFromRgba(data)).toEqual(['#EF4444', '#FFFFFF']);
  });

  it('extracts up to 5 distinct dominant and accent colors ordered by dominance', () => {
    const data = makeRgba([
      { rgb: [30, 41, 59], count: 900 },   // #1E293B dark slate bg
      { rgb: [33, 44, 62], count: 180 },   // near-duplicate dark slate (should merge/dedupe)
      { rgb: [56, 189, 248], count: 360 }, // #38BDF8 sky
      { rgb: [129, 140, 248], count: 280 }, // #818CF8 indigo
      { rgb: [244, 114, 182], count: 200 }, // #F472B6 pink
      { rgb: [250, 204, 21], count: 140 }, // #FACC15 amber
      { rgb: [34, 197, 94], count: 80 },   // #22C55E green (6th color, below top 5)
    ]);
    const palette = extractPaletteFromRgba(data, 5);
    expect(palette).toHaveLength(5);
    expect(palette[0]).toBe('#1E293B'); // keeps dominant slate and deduplicates near-duplicate #212C3E
    expect(palette).toContain('#38BDF8');
    expect(palette).toContain('#818CF8');
    expect(palette).toContain('#F472B6');
    expect(palette).toContain('#FACC15');
    expect(palette).not.toContain('#212C3E');
    expect(new Set(palette).size).toBe(5);
  });

  it('extracts tonal steps for monochromatic gradients via relaxed fallback', () => {
    const data = makeRgba([
      { rgb: [15, 35, 80], count: 500 },
      { rgb: [30, 65, 130], count: 500 },
      { rgb: [55, 105, 185], count: 500 },
      { rgb: [95, 155, 230], count: 500 },
    ]);
    const palette = extractPaletteFromRgba(data, 5);
    expect(palette.length).toBeGreaterThanOrEqual(4);
    expect(new Set(palette).size).toBe(palette.length);
  });
});
