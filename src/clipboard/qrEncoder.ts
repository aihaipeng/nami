export type QrEcLevel = 'L' | 'M';

export interface QrMatrix {
  version: number;
  size: number;
  ecLevel: QrEcLevel;
  modules: boolean[];
}

interface VersionSpec {
  version: number;
  ecLevel: QrEcLevel;
  dataCodewords: number;
  ecPerBlock: number;
  g1Blocks: number;
  g1Data: number;
  g2Blocks: number;
  g2Data: number;
  alignment: readonly number[];
}

// ISO/IEC 18004 Table 9 parameters for Versions 1..40 (1-indexed via offset - 1).
const ECC_CODEWORDS_PER_BLOCK: Record<QrEcLevel, readonly number[]> = {
  L: [
    7, 10, 15, 20, 26, 18, 20, 24, 30, 18,
    20, 24, 26, 30, 22, 24, 28, 30, 28, 28,
    28, 28, 30, 30, 26, 28, 30, 30, 30, 30,
    30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
  ],
  M: [
    10, 16, 26, 18, 24, 16, 18, 22, 22, 26,
    30, 22, 22, 24, 24, 28, 28, 26, 26, 26,
    26, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
  ],
};

const NUM_ERROR_CORRECTION_BLOCKS: Record<QrEcLevel, readonly number[]> = {
  L: [
    1, 1, 1, 1, 1, 2, 2, 2, 2, 4,
    4, 4, 4, 4, 6, 6, 6, 6, 7, 8,
    8, 9, 9, 10, 12, 12, 12, 13, 14, 15,
    16, 17, 18, 19, 19, 20, 21, 22, 24, 25,
  ],
  M: [
    1, 1, 1, 2, 2, 4, 4, 4, 5, 5,
    5, 8, 9, 9, 10, 10, 11, 13, 14, 16,
    17, 17, 18, 20, 21, 23, 25, 26, 28, 29,
    31, 33, 35, 37, 38, 40, 43, 45, 47, 49,
  ],
};

function alignmentCenters(version: number): number[] {
  if (version === 1) return [];
  const count = Math.floor(version / 7) + 2;
  const step = version === 32 ? 26 : Math.ceil((version * 4 + 4) / (count * 2 - 2)) * 2;
  const centers = [6];
  for (let pos = version * 4 + 10; centers.length < count; pos -= step) {
    centers.splice(1, 0, pos);
  }
  return centers;
}

function rawDataModules(version: number): number {
  let result = (16 * version + 128) * version + 64;
  if (version >= 2) {
    const numAlign = Math.floor(version / 7) + 2;
    result -= (25 * numAlign - 10) * numAlign - 55;
    if (version >= 7) result -= 36;
  }
  return result;
}

function buildVersionSpecs(ecLevel: QrEcLevel): VersionSpec[] {
  const specs: VersionSpec[] = [];
  for (let version = 1; version <= 40; version += 1) {
    const totalCodewords = Math.floor(rawDataModules(version) / 8);
    const ecPerBlock = ECC_CODEWORDS_PER_BLOCK[ecLevel][version - 1];
    const numBlocks = NUM_ERROR_CORRECTION_BLOCKS[ecLevel][version - 1];
    const dataCodewords = totalCodewords - ecPerBlock * numBlocks;
    const g2Blocks = totalCodewords % numBlocks;
    const g1Blocks = numBlocks - g2Blocks;
    const g1Data = Math.floor(dataCodewords / numBlocks);
    const g2Data = g2Blocks > 0 ? g1Data + 1 : 0;
    specs.push({
      version,
      ecLevel,
      dataCodewords,
      ecPerBlock,
      g1Blocks,
      g1Data,
      g2Blocks,
      g2Data,
      alignment: alignmentCenters(version),
    });
  }
  return specs;
}

const VERSION_SPECS: Record<QrEcLevel, readonly VersionSpec[]> = {
  L: buildVersionSpecs('L'),
  M: buildVersionSpecs('M'),
};

export const MAX_QR_ECC_M_BYTES = 280;
export const MAX_QR_UTF8_BYTES = 2953;

const GF_EXP = new Uint8Array(512);
const GF_LOG = new Uint8Array(256);
{
  let value = 1;
  for (let i = 0; i < 255; i += 1) {
    GF_EXP[i] = value;
    GF_LOG[value] = i;
    value <<= 1;
    if (value & 0x100) value ^= 0x11d;
  }
  for (let i = 255; i < 512; i += 1) GF_EXP[i] = GF_EXP[i - 255];
}

function gfMul(a: number, b: number): number {
  return a === 0 || b === 0 ? 0 : GF_EXP[GF_LOG[a] + GF_LOG[b]];
}

const RS_GENERATORS = new Map<number, Uint8Array>();

function rsGenerator(degree: number): Uint8Array {
  const cached = RS_GENERATORS.get(degree);
  if (cached) return cached;
  let poly = new Uint8Array([1]);
  for (let i = 0; i < degree; i += 1) {
    const next = new Uint8Array(poly.length + 1);
    const root = GF_EXP[i];
    for (let j = 0; j < poly.length; j += 1) {
      next[j] ^= poly[j];
      next[j + 1] ^= gfMul(poly[j], root);
    }
    poly = next;
  }
  RS_GENERATORS.set(degree, poly);
  return poly;
}

function rsRemainder(data: Uint8Array, ecCount: number): Uint8Array {
  const gen = rsGenerator(ecCount);
  const rem = new Uint8Array(ecCount);
  for (let i = 0; i < data.length; i += 1) {
    const factor = data[i] ^ rem[0];
    rem.copyWithin(0, 1);
    rem[ecCount - 1] = 0;
    for (let j = 0; j < ecCount; j += 1) {
      rem[j] ^= gfMul(gen[j + 1], factor);
    }
  }
  return rem;
}

function selectSpec(byteLength: number): VersionSpec | null {
  // Keep higher resilience (ECC M) for compact payloads; step to ECC L for longer text to stay sparser up to 2,953 bytes.
  const level: QrEcLevel = byteLength <= MAX_QR_ECC_M_BYTES ? 'M' : 'L';
  for (const spec of VERSION_SPECS[level]) {
    const headerBits = 4 + (spec.version <= 9 ? 8 : 16);
    const capacityBits = spec.dataCodewords * 8;
    if (headerBits + byteLength * 8 <= capacityBits) return spec;
  }
  return null;
}

function encodeDataCodewords(bytes: Uint8Array, spec: VersionSpec): Uint8Array {
  const bits: number[] = [];
  const pushBits = (value: number, width: number) => {
    for (let i = width - 1; i >= 0; i -= 1) bits.push((value >>> i) & 1);
  };
  // Byte mode indicator (0100) + character count.
  pushBits(0b0100, 4);
  pushBits(bytes.length, spec.version <= 9 ? 8 : 16);
  for (const byte of bytes) pushBits(byte, 8);
  const maxBits = spec.dataCodewords * 8;
  const terminator = Math.min(4, maxBits - bits.length);
  for (let i = 0; i < terminator; i += 1) bits.push(0);
  while (bits.length % 8 !== 0) bits.push(0);
  const out = new Uint8Array(spec.dataCodewords);
  const byteCount = bits.length / 8;
  for (let i = 0; i < byteCount; i += 1) {
    let value = 0;
    for (let bit = 0; bit < 8; bit += 1) value = (value << 1) | bits[i * 8 + bit];
    out[i] = value;
  }
  for (let i = byteCount, pad = 0; i < out.length; i += 1, pad ^= 1) {
    out[i] = pad === 0 ? 0xec : 0x11;
  }
  return out;
}

function interleaveCodewords(data: Uint8Array, spec: VersionSpec): Uint8Array {
  const dataBlocks: Uint8Array[] = [];
  const ecBlocks: Uint8Array[] = [];
  let offset = 0;
  for (let i = 0; i < spec.g1Blocks; i += 1) {
    const slice = data.subarray(offset, offset + spec.g1Data);
    offset += spec.g1Data;
    dataBlocks.push(slice);
    ecBlocks.push(rsRemainder(slice, spec.ecPerBlock));
  }
  for (let i = 0; i < spec.g2Blocks; i += 1) {
    const slice = data.subarray(offset, offset + spec.g2Data);
    offset += spec.g2Data;
    dataBlocks.push(slice);
    ecBlocks.push(rsRemainder(slice, spec.ecPerBlock));
  }
  const totalBlocks = dataBlocks.length;
  const maxDataLen = Math.max(spec.g1Data, spec.g2Data);
  const totalBytes = spec.dataCodewords + totalBlocks * spec.ecPerBlock;
  const interleaved = new Uint8Array(totalBytes);
  let cursor = 0;
  for (let i = 0; i < maxDataLen; i += 1) {
    for (const block of dataBlocks) {
      if (i < block.length) interleaved[cursor++] = block[i];
    }
  }
  for (let i = 0; i < spec.ecPerBlock; i += 1) {
    for (const block of ecBlocks) {
      interleaved[cursor++] = block[i];
    }
  }
  return interleaved;
}

function formatBits(ecLevel: QrEcLevel, mask: number): number {
  // ISO/IEC 18004 Table 25: ECC Level L = 0b01, M = 0b00.
  const ecIndicator = ecLevel === 'L' ? 0b01 : 0b00;
  const data = (ecIndicator << 3) | (mask & 0b111);
  let rem = data << 10;
  for (let i = 14; i >= 10; i -= 1) {
    if ((rem >>> i) & 1) rem ^= 0x537 << (i - 10);
  }
  return ((data << 10) | rem) ^ 0x5412;
}

function versionBits(version: number): number {
  let rem = version << 12;
  for (let i = 17; i >= 12; i -= 1) {
    if ((rem >>> i) & 1) rem ^= 0x1f25 << (i - 12);
  }
  return (version << 12) | rem;
}

function maskCondition(mask: number, row: number, col: number): boolean {
  switch (mask) {
    case 0: return (row + col) % 2 === 0;
    case 1: return row % 2 === 0;
    case 2: return col % 3 === 0;
    case 3: return (row + col) % 3 === 0;
    case 4: return (Math.floor(row / 2) + Math.floor(col / 3)) % 2 === 0;
    case 5: return ((row * col) % 2) + ((row * col) % 3) === 0;
    case 6: return (((row * col) % 2) + ((row * col) % 3)) % 2 === 0;
    default: return (((row + col) % 2) + ((row * col) % 3)) % 2 === 0;
  }
}

function penaltyScore(modules: Uint8Array, size: number): number {
  let score = 0;
  const at = (r: number, c: number) => modules[r * size + c];
  // Rule 1: runs of 5+ same-colored modules.
  for (let r = 0; r < size; r += 1) {
    let runColor = at(r, 0);
    let runLen = 1;
    for (let c = 1; c < size; c += 1) {
      const value = at(r, c);
      if (value === runColor) {
        runLen += 1;
      } else {
        if (runLen >= 5) score += 3 + (runLen - 5);
        runColor = value;
        runLen = 1;
      }
    }
    if (runLen >= 5) score += 3 + (runLen - 5);
  }
  for (let c = 0; c < size; c += 1) {
    let runColor = at(0, c);
    let runLen = 1;
    for (let r = 1; r < size; r += 1) {
      const value = at(r, c);
      if (value === runColor) {
        runLen += 1;
      } else {
        if (runLen >= 5) score += 3 + (runLen - 5);
        runColor = value;
        runLen = 1;
      }
    }
    if (runLen >= 5) score += 3 + (runLen - 5);
  }
  // Rule 2: 2x2 blocks of same color.
  for (let r = 0; r < size - 1; r += 1) {
    for (let c = 0; c < size - 1; c += 1) {
      const value = at(r, c);
      if (value === at(r, c + 1) && value === at(r + 1, c) && value === at(r + 1, c + 1)) score += 3;
    }
  }
  // Rule 3: finder-like 1:1:3:1:1 patterns with 4 light modules on either side.
  const checkPattern = (line: Uint8Array) => {
    for (let i = 0; i <= line.length - 11; i += 1) {
      const a = line[i] === 1 && line[i + 1] === 0 && line[i + 2] === 1 && line[i + 3] === 1 && line[i + 4] === 1
        && line[i + 5] === 0 && line[i + 6] === 1 && line[i + 7] === 0 && line[i + 8] === 0 && line[i + 9] === 0 && line[i + 10] === 0;
      const b = line[i] === 0 && line[i + 1] === 0 && line[i + 2] === 0 && line[i + 3] === 0 && line[i + 4] === 1
        && line[i + 5] === 0 && line[i + 6] === 1 && line[i + 7] === 1 && line[i + 8] === 1 && line[i + 9] === 0 && line[i + 10] === 1;
      if (a || b) score += 40;
    }
  };
  const line = new Uint8Array(size);
  for (let r = 0; r < size; r += 1) {
    for (let c = 0; c < size; c += 1) line[c] = at(r, c);
    checkPattern(line);
  }
  for (let c = 0; c < size; c += 1) {
    for (let r = 0; r < size; r += 1) line[r] = at(r, c);
    checkPattern(line);
  }
  // Rule 4: dark/light balance.
  let dark = 0;
  for (let i = 0; i < modules.length; i += 1) dark += modules[i];
  const percent = Math.abs((dark * 100) / modules.length - 50);
  score += Math.floor(percent / 5) * 10;
  return score;
}

function placeFormat(modules: Uint8Array, size: number, ecLevel: QrEcLevel, mask: number) {
  const bits = formatBits(ecLevel, mask);
  const set = (r: number, c: number, bit: number) => { modules[r * size + c] = bit; };
  // Top-left around finder (LSB = bit 0).
  for (let i = 0; i <= 5; i += 1) set(8, i, (bits >>> (14 - i)) & 1);
  set(8, 7, (bits >>> 8) & 1);
  set(8, 8, (bits >>> 7) & 1);
  set(7, 8, (bits >>> 6) & 1);
  for (let i = 9; i < 15; i += 1) set(14 - i, 8, (bits >>> (14 - i)) & 1);
  // Bottom-left and top-right copies.
  for (let i = 0; i < 7; i += 1) set(size - 1 - i, 8, (bits >>> (14 - i)) & 1);
  for (let i = 7; i < 15; i += 1) set(8, size - 15 + i, (bits >>> (14 - i)) & 1);
}

function placeVersion(modules: Uint8Array, isFunction: Uint8Array, size: number, version: number) {
  if (version < 7) return;
  const bits = versionBits(version);
  for (let i = 0; i < 18; i += 1) {
    const bit = (bits >>> i) & 1;
    const a = size - 11 + (i % 3);
    const b = Math.floor(i / 3);
    modules[b * size + a] = bit;
    isFunction[b * size + a] = 1;
    modules[a * size + b] = bit;
    isFunction[a * size + b] = 1;
  }
}

export function encodeQrCode(text: string): QrMatrix | null {
  if (!text) return null;
  const bytes = new TextEncoder().encode(text);
  if (bytes.length === 0 || bytes.length > MAX_QR_UTF8_BYTES) return null;
  const spec = selectSpec(bytes.length);
  if (!spec) return null;

  const size = spec.version * 4 + 17;
  const base = new Uint8Array(size * size);
  const isFunction = new Uint8Array(size * size);
  const setFn = (r: number, c: number, dark: boolean) => {
    if (r < 0 || r >= size || c < 0 || c >= size) return;
    base[r * size + c] = dark ? 1 : 0;
    isFunction[r * size + c] = 1;
  };

  // Finder patterns + separators.
  const drawFinder = (top: number, left: number) => {
    for (let dy = -1; dy <= 7; dy += 1) {
      for (let dx = -1; dx <= 7; dx += 1) {
        const dark = dx >= 0 && dx <= 6 && dy >= 0 && dy <= 6
          && (dx === 0 || dx === 6 || dy === 0 || dy === 6 || (dx >= 2 && dx <= 4 && dy >= 2 && dy <= 4));
        setFn(top + dy, left + dx, dark);
      }
    }
  };
  drawFinder(0, 0);
  drawFinder(0, size - 7);
  drawFinder(size - 7, 0);

  // Timing patterns.
  for (let i = 8; i < size - 8; i += 1) {
    const dark = i % 2 === 0;
    setFn(6, i, dark);
    setFn(i, 6, dark);
  }

  // Alignment patterns (skip only centers overlapping the three finder areas).
  for (const r of spec.alignment) {
    for (const c of spec.alignment) {
      if ((r <= 8 && c <= 8) || (r <= 8 && c >= size - 8) || (r >= size - 8 && c <= 8)) continue;
      for (let dy = -2; dy <= 2; dy += 1) {
        for (let dx = -2; dx <= 2; dx += 1) {
          const dist = Math.max(Math.abs(dx), Math.abs(dy));
          setFn(r + dy, c + dx, dist === 0 || dist === 2);
        }
      }
    }
  }

  // Reserve format info areas + dark module.
  for (let i = 0; i <= 8; i += 1) {
    if (i !== 6) {
      setFn(8, i, false);
      setFn(i, 8, false);
    }
  }
  for (let i = size - 8; i < size; i += 1) {
    setFn(8, i, false);
    setFn(i, 8, false);
  }
  setFn(size - 8, 8, true);
  placeVersion(base, isFunction, size, spec.version);

  // Interleave codewords and place data bits in zigzag columns.
  const codewords = interleaveCodewords(encodeDataCodewords(bytes, spec), spec);
  let bitIndex = 0;
  const totalBits = codewords.length * 8;
  let upward = true;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let step = 0; step < size; step += 1) {
      const r = upward ? size - 1 - step : step;
      for (let dx = 0; dx < 2; dx += 1) {
        const c = right - dx;
        if (isFunction[r * size + c]) continue;
        let dark = 0;
        if (bitIndex < totalBits) {
          dark = (codewords[bitIndex >>> 3] >>> (7 - (bitIndex & 7))) & 1;
          bitIndex += 1;
        }
        base[r * size + c] = dark;
      }
    }
    upward = !upward;
  }

  // Pick the lowest-penalty mask.
  let bestScore = Number.POSITIVE_INFINITY;
  let bestModules = base;
  for (let mask = 0; mask < 8; mask += 1) {
    const candidate = base.slice();
    for (let r = 0; r < size; r += 1) {
      for (let c = 0; c < size; c += 1) {
        if (!isFunction[r * size + c] && maskCondition(mask, r, c)) {
          candidate[r * size + c] ^= 1;
        }
      }
    }
    placeFormat(candidate, size, spec.ecLevel, mask);
    const score = penaltyScore(candidate, size);
    if (score < bestScore) {
      bestScore = score;
      bestModules = candidate;
    }
  }
  return {
    version: spec.version,
    size,
    ecLevel: spec.ecLevel,
    modules: Array.from(bestModules, (value) => value === 1),
  };
}

export function packQrModules(matrix: QrMatrix): number[] {
  const packed = new Array<number>(Math.ceil(matrix.modules.length / 8)).fill(0);
  for (let i = 0; i < matrix.modules.length; i += 1) {
    if (matrix.modules[i]) packed[i >>> 3] |= 1 << (7 - (i & 7));
  }
  return packed;
}
