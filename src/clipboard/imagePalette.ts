const SAMPLE_SIZE = 48;
const DEFAULT_MAX_COLORS = 5;
const MIN_ALPHA = 128;
const CANDIDATE_BUCKETS = 12;
const DISTINCT_DISTANCE = 68;
const RELAXED_DISTANCE = 34;

interface ColorBin {
  r: number;
  g: number;
  b: number;
  sumR: number;
  sumG: number;
  sumB: number;
  count: number;
}

interface ColorCluster {
  r: number;
  g: number;
  b: number;
  count: number;
  saturation: number;
  hex: string;
}

function toHexByte(value: number): string {
  const clamped = Math.max(0, Math.min(255, Math.round(value)));
  return clamped.toString(16).padStart(2, '0').toUpperCase();
}

function rgbToHex(r: number, g: number, b: number): string {
  return `#${toHexByte(r)}${toHexByte(g)}${toHexByte(b)}`;
}

function rgbSaturation(r: number, g: number, b: number): number {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  return max === 0 ? 0 : (max - min) / max;
}

/** Fast perceptual RGB distance (redmean weighted Euclidean, range ~0..765). */
function perceptualDistance(a: { r: number; g: number; b: number }, b: { r: number; g: number; b: number }): number {
  const rMean = (a.r + b.r) / 2;
  const dr = a.r - b.r;
  const dg = a.g - b.g;
  const db = a.b - b.b;
  return Math.sqrt((2 + rMean / 256) * dr * dr + 4 * dg * dg + (2 + (255 - rMean) / 256) * db * db);
}

interface ColorBox {
  bins: ColorBin[];
  count: number;
  minR: number;
  maxR: number;
  minG: number;
  maxG: number;
  minB: number;
  maxB: number;
}

function makeBox(bins: ColorBin[]): ColorBox {
  let count = 0;
  let minR = 255;
  let maxR = 0;
  let minG = 255;
  let maxG = 0;
  let minB = 255;
  let maxB = 0;
  for (const bin of bins) {
    count += bin.count;
    if (bin.r < minR) minR = bin.r;
    if (bin.r > maxR) maxR = bin.r;
    if (bin.g < minG) minG = bin.g;
    if (bin.g > maxG) maxG = bin.g;
    if (bin.b < minB) minB = bin.b;
    if (bin.b > maxB) maxB = bin.b;
  }
  return { bins, count, minR, maxR, minG, maxG, minB, maxB };
}

function boxSpread(box: ColorBox): number {
  return Math.max(box.maxR - box.minR, box.maxG - box.minG, box.maxB - box.minB);
}

function splitBox(box: ColorBox): [ColorBox, ColorBox] | null {
  if (box.bins.length < 2) return null;
  const rangeR = box.maxR - box.minR;
  const rangeG = box.maxG - box.minG;
  const rangeB = box.maxB - box.minB;
  const channel: 'r' | 'g' | 'b' = rangeG >= rangeR && rangeG >= rangeB ? 'g' : rangeR >= rangeB ? 'r' : 'b';
  const sorted = [...box.bins].sort((a, b) => a[channel] - b[channel]);

  const half = box.count / 2;
  let acc = 0;
  let splitIndex = 1;
  for (let i = 0; i < sorted.length - 1; i += 1) {
    acc += sorted[i].count;
    splitIndex = i + 1;
    if (acc >= half) break;
  }
  return [makeBox(sorted.slice(0, splitIndex)), makeBox(sorted.slice(splitIndex))];
}

function boxToCluster(box: ColorBox): ColorCluster {
  let sumR = 0;
  let sumG = 0;
  let sumB = 0;
  let count = 0;
  for (const bin of box.bins) {
    sumR += bin.sumR;
    sumG += bin.sumG;
    sumB += bin.sumB;
    count += bin.count;
  }
  const r = Math.round(sumR / count);
  const g = Math.round(sumG / count);
  const b = Math.round(sumB / count);
  return {
    r,
    g,
    b,
    count,
    saturation: rgbSaturation(r, g, b),
    hex: rgbToHex(r, g, b),
  };
}

/**
 * Extracts up to `maxColors` (default 5) representative uppercase `#RRGGBB` colors
 * from raw RGBA pixel data via 5-bit bucketing, median-cut quantization, and
 * perceptual deduplication.
 */
export function extractPaletteFromRgba(
  rgba: Uint8ClampedArray | Uint8Array | readonly number[],
  maxColors = DEFAULT_MAX_COLORS,
): string[] {
  if (maxColors <= 0 || rgba.length < 4) return [];

  const binMap = new Map<number, ColorBin>();
  let totalOpaque = 0;
  const pixelEnd = rgba.length - (rgba.length % 4);
  for (let i = 0; i < pixelEnd; i += 4) {
    if (rgba[i + 3] < MIN_ALPHA) continue;
    const r = rgba[i];
    const g = rgba[i + 1];
    const b = rgba[i + 2];
    const key = ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
    const existing = binMap.get(key);
    if (existing) {
      existing.sumR += r;
      existing.sumG += g;
      existing.sumB += b;
      existing.count += 1;
    } else {
      binMap.set(key, { r, g, b, sumR: r, sumG: g, sumB: b, count: 1 });
    }
    totalOpaque += 1;
  }

  if (totalOpaque === 0 || binMap.size === 0) return [];

  const bins: ColorBin[] = [];
  for (const bin of binMap.values()) {
    bin.r = bin.sumR / bin.count;
    bin.g = bin.sumG / bin.count;
    bin.b = bin.sumB / bin.count;
    bins.push(bin);
  }

  const boxes: ColorBox[] = [makeBox(bins)];
  const targetBuckets = Math.max(maxColors * 2, CANDIDATE_BUCKETS);
  while (boxes.length < targetBuckets) {
    let bestIndex = -1;
    let bestPriority = 0;
    for (let i = 0; i < boxes.length; i += 1) {
      const box = boxes[i];
      if (box.bins.length < 2) continue;
      const spread = boxSpread(box);
      if (spread <= 8) continue;
      const priority = box.count * spread;
      if (priority > bestPriority) {
        bestPriority = priority;
        bestIndex = i;
      }
    }
    if (bestIndex < 0) break;
    const split = splitBox(boxes[bestIndex]);
    if (!split) break;
    boxes.splice(bestIndex, 1, split[0], split[1]);
  }

  const clusters = boxes.map(boxToCluster);
  if (clusters.length === 0) return [];

  // Always start with the single highest-coverage color as primary tone.
  clusters.sort((a, b) => b.count - a.count);
  const selected: ColorCluster[] = [clusters[0]];

  const minNoiseCount = Math.max(2, Math.floor(totalOpaque * 0.004));
  const remaining = clusters
    .slice(1)
    .filter((cluster) => cluster.count >= minNoiseCount)
    .sort((a, b) => (b.count * (1 + 1.5 * b.saturation)) - (a.count * (1 + 1.5 * a.saturation)));

  const isFarEnough = (candidate: ColorCluster, threshold: number) =>
    selected.every((picked) => picked.hex !== candidate.hex && perceptualDistance(picked, candidate) >= threshold);

  for (const candidate of remaining) {
    if (selected.length >= maxColors) break;
    if (isFarEnough(candidate, DISTINCT_DISTANCE)) {
      selected.push(candidate);
    }
  }

  // Fallback for monochromatic/low-contrast images with rich tonal variation:
  // fill up to 4 swatches using a gentler perceptual threshold, without inventing colors on flat graphics.
  const relaxedTarget = Math.min(maxColors, 4);
  if (selected.length < relaxedTarget) {
    const minRelaxedCount = Math.max(3, Math.floor(totalOpaque * 0.015));
    for (const candidate of remaining) {
      if (selected.length >= relaxedTarget) break;
      if (candidate.count < minRelaxedCount) continue;
      if (isFarEnough(candidate, RELAXED_DISTANCE)) {
        selected.push(candidate);
      }
    }
  }

  selected.sort((a, b) => b.count - a.count);
  return selected.map((cluster) => cluster.hex);
}

/**
 * Downsamples an already-decoded HTMLImageElement onto an offscreen canvas (max 48x48)
 * and extracts its dominant color palette locally with zero external dependencies.
 */
export function extractImagePalette(img: HTMLImageElement, maxColors = DEFAULT_MAX_COLORS): string[] {
  const width = img.naturalWidth || img.width;
  const height = img.naturalHeight || img.height;
  if (width <= 0 || height <= 0 || typeof document === 'undefined') return [];

  const scale = Math.min(1, SAMPLE_SIZE / Math.max(width, height));
  const targetWidth = Math.max(1, Math.round(width * scale));
  const targetHeight = Math.max(1, Math.round(height * scale));

  try {
    const canvas = document.createElement('canvas');
    canvas.width = targetWidth;
    canvas.height = targetHeight;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) return [];
    context.drawImage(img, 0, 0, targetWidth, targetHeight);
    const { data } = context.getImageData(0, 0, targetWidth, targetHeight);
    return extractPaletteFromRgba(data, maxColors);
  } catch {
    return [];
  }
}
