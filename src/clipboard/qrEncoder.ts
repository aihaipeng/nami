export interface QrMatrix {
  version: number;
  size: number;
  modules: boolean[];
}

interface VersionSpec {
  version: number;
  dataCodewords: number;
  ecPerBlock: number;
  g1Blocks: number;
  g1Data: number;
  g2Blocks: number;
  g2Data: number;
  alignment: readonly number[];
}

// ISO/IEC 18004 ECC Level M parameters for Versions 1..12.
const VERSION_SPECS: readonly VersionSpec[] = [
  { version: 1, dataCodewords: 16, ecPerBlock: 10, g1Blocks: 1, g1Data: 16, g2Blocks: 0, g2Data: 0, alignment: [] },
  { version: 2, dataCodewords: 28, ecPerBlock: 16, g1Blocks: 1, g1Data: 28, g2Blocks: 0, g2Data: 0, alignment: [6, 18] },
  { version: 3, dataCodewords: 44, ecPerBlock: 26, g1Blocks: 1, g1Data: 44, g2Blocks: 0, g2Data: 0, alignment: [6, 22] },
  { version: 4, dataCodewords: 64, ecPerBlock: 18, g1Blocks: 2, g1Data: 32, g2Blocks: 0, g2Data: 0, alignment: [6, 26] },
  { version: 5, dataCodewords: 86, ecPerBlock: 24, g1Blocks: 2, g1Data: 43, g2Blocks: 0, g2Data: 0, alignment: [6, 30] },
  { version: 6, dataCodewords: 108, ecPerBlock: 16, g1Blocks: 4, g1Data: 27, g2Blocks: 0, g2Data: 0, alignment: [6, 34] },
  { version: 7, dataCodewords: 124, ecPerBlock: 18, g1Blocks: 4, g1Data: 31, g2Blocks: 0, g2Data: 0, alignment: [6, 22, 38] },
  { version: 8, dataCodewords: 154, ecPerBlock: 22, g1Blocks: 2, g1Data: 38, g2Blocks: 2, g2Data: 39, alignment: [6, 24, 42] },
  { version: 9, dataCodewords: 182, ecPerBlock: 22, g1Blocks: 3, g1Data: 36, g2Blocks: 2, g2Data: 37, alignment: [6, 26, 46] },
  { version: 10, dataCodewords: 216, ecPerBlock: 26, g1Blocks: 4, g1Data: 43, g2Blocks: 1, g2Data: 44, alignment: [6, 28, 50] },
  { version: 11, dataCodewords: 254, ecPerBlock: 30, g1Blocks: 1, g1Data: 50, g2Blocks: 4, g2Data: 51, alignment: [6, 30, 54] },
  { version: 12, dataCodewords: 290, ecPerBlock: 22, g1Blocks: 6, g1Data: 36, g2Blocks: 2, g2Data: 37, alignment: [6, 32, 58] },
];

export const MAX_QR_UTF8_BYTES = 280;

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

function rsGenerator(degree: number): Uint8Array {
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
  for (const spec of VERSION_SPECS) {
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

function formatBits(mask: number): number {
  // ECC Level M = 0b00
  const data = (0b00 << 3) | (mask & 0b111);
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

function placeFormat(modules: Uint8Array, size: number, mask: number) {
  const bits = formatBits(mask);
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
  let bestMask = 0;
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
    placeFormat(candidate, size, mask);
    const score = penaltyScore(candidate, size);
    if (score < bestScore) {
      bestScore = score;
      bestMask = mask;
      bestModules = candidate;
    }
  }
  void bestMask;
  return {
    version: spec.version,
    size,
    modules: Array.from(bestModules, (value) => value === 1),
  };
}
