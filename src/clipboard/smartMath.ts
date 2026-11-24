export interface SmartMath {
  kind: 'math';
  result: string;
  formatted: string | null;
}

export interface SmartNumber {
  kind: 'number';
  dec: string;
  hex: string;
  bin: string;
  oct: string;
  permissions: string | null;
  preferPermissionsFirst: boolean;
}

type MathToken =
  | { type: 'num'; value: number }
  | { type: 'op'; value: '+' | '-' | '*' | '/' | '%' | '**' }
  | { type: 'lparen' }
  | { type: 'rparen' };

const MAX_MATH_LEN = 256;
const MAX_MATH_DEPTH = 32;
const MAX_SAFE_MAGNITUDE = 1e15;

// Patterns that look like dates, phone numbers, or serial IDs rather than arithmetic.
const NON_MATH_PATTERNS = [
  /^\d{4}-\d{1,2}$/, // YYYY-MM
  /^\d{1,2}-\d{1,2}$/, // MM-DD or small range like 10-20 without spaces
  /^\d{1,2}[-/]\d{1,2}[-/]\d{2,4}$/, // DD/MM/YYYY, MM/DD/YYYY, DD-MM-YYYY
  /^\d{4}[-/]\d{1,2}[-/]\d{1,2}$/, // YYYY/MM/DD
  /^\d+(?:-\d+){2,}$/, // 400-800-1234, 010-1234-5678
];

function tokenizeMath(input: string): { tokens: MathToken[]; hasBinaryOp: boolean } | null {
  const tokens: MathToken[] = [];
  let i = 0;

  while (i < input.length) {
    const ch = input[i];
    if (ch === ' ' || ch === '\t') {
      i += 1;
      continue;
    }
    if (ch === '(') {
      tokens.push({ type: 'lparen' });
      i += 1;
      continue;
    }
    if (ch === ')') {
      tokens.push({ type: 'rparen' });
      i += 1;
      continue;
    }
    if (ch === '*' && input[i + 1] === '*') {
      tokens.push({ type: 'op', value: '**' });
      i += 2;
      continue;
    }
    if (ch === '+' || ch === '-' || ch === '*' || ch === '/' || ch === '%' || ch === '×' || ch === '÷') {
      const normalized = ch === '×' ? '*' : ch === '÷' ? '/' : ch;
      tokens.push({ type: 'op', value: normalized });
      i += 1;
      continue;
    }
    if ((ch >= '0' && ch <= '9') || ch === '.') {
      const start = i;
      let dots = 0;
      while (i < input.length && ((input[i] >= '0' && input[i] <= '9') || input[i] === '.')) {
        if (input[i] === '.') dots += 1;
        i += 1;
      }
      const rawNum = input.slice(start, i);
      if (dots > 1 || rawNum === '.' || rawNum.endsWith('.')) return null;
      // Reject leading-zero integers (e.g. 01, 0755) while allowing 0 and 0.xxx.
      if (rawNum.length > 1 && rawNum[0] === '0' && rawNum[1] !== '.') return null;
      const value = Number(rawNum);
      if (!Number.isFinite(value)) return null;
      tokens.push({ type: 'num', value });
      continue;
    }
    return null;
  }

  if (tokens.length < 3) return null;
  return { tokens, hasBinaryOp: false };
}

function formatMathNumber(value: number): { result: string; formatted: string | null } | null {
  if (!Number.isFinite(value) || Math.abs(value) > MAX_SAFE_MAGNITUDE) return null;
  const cleaned = Number(value.toPrecision(12));
  const normalized = Object.is(cleaned, -0) ? 0 : cleaned;
  if (!Number.isFinite(normalized)) return null;
  const result = String(normalized);
  if (result.includes('e') || result.includes('E')) {
    return { result, formatted: null };
  }
  const negative = result.startsWith('-');
  const unsigned = negative ? result.slice(1) : result;
  const [intPart, fracPart] = unsigned.split('.');
  if (intPart.length < 4) return { result, formatted: null };
  const grouped = intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const formatted = `${negative ? '-' : ''}${grouped}${fracPart !== undefined ? `.${fracPart}` : ''}`;
  return { result, formatted: formatted !== result ? formatted : null };
}

export function parseSmartMath(trimmed: string): SmartMath | null {
  if (trimmed.length < 3 || trimmed.length > MAX_MATH_LEN || /[\r\n]/.test(trimmed)) return null;
  if (NON_MATH_PATTERNS.some((pattern) => pattern.test(trimmed))) return null;

  const lexed = tokenizeMath(trimmed);
  if (!lexed) return null;
  const { tokens } = lexed;

  let pos = 0;
  let binaryOps = 0;

  function peek(): MathToken | undefined {
    return tokens[pos];
  }

  function parseExpression(depth: number): number {
    if (depth > MAX_MATH_DEPTH) return Number.NaN;
    let left = parseTerm(depth + 1);
    while (true) {
      const tok = peek();
      if (tok?.type === 'op' && (tok.value === '+' || tok.value === '-')) {
        pos += 1;
        binaryOps += 1;
        const right = parseTerm(depth + 1);
        left = tok.value === '+' ? left + right : left - right;
      } else {
        break;
      }
    }
    return left;
  }

  function parseTerm(depth: number): number {
    if (depth > MAX_MATH_DEPTH) return Number.NaN;
    let left = parsePower(depth + 1);
    while (true) {
      const tok = peek();
      if (tok?.type === 'op' && (tok.value === '*' || tok.value === '/' || tok.value === '%')) {
        pos += 1;
        binaryOps += 1;
        const right = parsePower(depth + 1);
        if (tok.value === '*') left *= right;
        else if (tok.value === '/') {
          if (right === 0) return Number.NaN;
          left /= right;
        } else {
          if (right === 0) return Number.NaN;
          left %= right;
        }
      } else {
        break;
      }
    }
    return left;
  }

  function parsePower(depth: number): number {
    if (depth > MAX_MATH_DEPTH) return Number.NaN;
    const base = parseUnary(depth + 1);
    const tok = peek();
    if (tok?.type === 'op' && tok.value === '**') {
      pos += 1;
      binaryOps += 1;
      const exp = parsePower(depth + 1);
      if (!Number.isFinite(exp) || Math.abs(exp) > 1000) return Number.NaN;
      return Math.pow(base, exp);
    }
    return base;
  }

  function parseUnary(depth: number): number {
    if (depth > MAX_MATH_DEPTH) return Number.NaN;
    const tok = peek();
    if (tok?.type === 'op' && (tok.value === '+' || tok.value === '-')) {
      pos += 1;
      const operand = parseUnary(depth + 1);
      return tok.value === '-' ? -operand : operand;
    }
    return parsePrimary(depth + 1);
  }

  function parsePrimary(depth: number): number {
    if (depth > MAX_MATH_DEPTH) return Number.NaN;
    const tok = peek();
    if (!tok) return Number.NaN;
    if (tok.type === 'num') {
      pos += 1;
      return tok.value;
    }
    if (tok.type === 'lparen') {
      pos += 1;
      const value = parseExpression(depth + 1);
      if (peek()?.type !== 'rparen') return Number.NaN;
      pos += 1;
      return value;
    }
    return Number.NaN;
  }

  const value = parseExpression(0);
  if (pos !== tokens.length || binaryOps < 1) return null;
  const formatted = formatMathNumber(value);
  if (!formatted) return null;
  return { kind: 'math', result: formatted.result, formatted: formatted.formatted };
}

function formatUnixPermissions(mode: number): string {
  const special = (mode >> 9) & 0o7;
  const owner = (mode >> 6) & 0o7;
  const group = (mode >> 3) & 0o7;
  const other = mode & 0o7;

  const triplet = (bits: number, specialBit: boolean, specialLower: string, specialUpper: string) => {
    const r = (bits & 4) ? 'r' : '-';
    const w = (bits & 2) ? 'w' : '-';
    const exec = Boolean(bits & 1);
    const x = specialBit ? (exec ? specialLower : specialUpper) : (exec ? 'x' : '-');
    return `${r}${w}${x}`;
  };

  return (
    triplet(owner, Boolean(special & 4), 's', 'S')
    + triplet(group, Boolean(special & 2), 's', 'S')
    + triplet(other, Boolean(special & 1), 't', 'T')
  );
}

function buildSmartNumber(value: bigint, permissions: string | null, preferPermissionsFirst: boolean): SmartNumber {
  return {
    kind: 'number',
    dec: value.toString(10),
    hex: `0x${value.toString(16).toUpperCase()}`,
    bin: `0b${value.toString(2)}`,
    oct: `0o${value.toString(8)}`,
    permissions,
    preferPermissionsFirst,
  };
}

export function parseSmartNumber(trimmed: string): SmartNumber | null {
  if (trimmed.length < 3 || trimmed.length > 96 || /[\r\n]/.test(trimmed)) return null;

  // chmod command with 3-4 digit octal mode, e.g. "chmod 755", "chmod -R 0644 file.txt"
  const chmodMatch = /^(?:sudo\s+)?chmod(?:\s+-[A-Za-z]+)*\s+([0-7]{3,4})(?:\s+\S.*)?$/.exec(trimmed);
  if (chmodMatch) {
    const octalDigits = chmodMatch[1];
    const value = BigInt(`0o${octalDigits}`);
    return buildSmartNumber(value, formatUnixPermissions(Number(value)), true);
  }

  // Hex literal: 0xFF, 0x1A_2B (1..16 hex digits)
  const hexMatch = /^0[xX]([0-9a-fA-F](?:_?[0-9a-fA-F]){0,15})$/.exec(trimmed);
  if (hexMatch) {
    const clean = hexMatch[1].replace(/_/g, '');
    return buildSmartNumber(BigInt(`0x${clean}`), null, false);
  }

  // Binary literal: 0b1010, 0b1111_0000 (1..64 binary digits)
  const binMatch = /^0[bB]([01](?:_?[01]){0,63})$/.exec(trimmed);
  if (binMatch) {
    const clean = binMatch[1].replace(/_/g, '');
    return buildSmartNumber(BigInt(`0b${clean}`), null, false);
  }

  // Octal literal: 0o755, 0o4755 (1..22 octal digits)
  const octMatch = /^0[oO]([0-7](?:_?[0-7]){0,21})$/.exec(trimmed);
  if (octMatch) {
    const clean = octMatch[1].replace(/_/g, '');
    const value = BigInt(`0o${clean}`);
    const permissions = clean.length >= 3 && clean.length <= 4 && value <= 0o7777n
      ? formatUnixPermissions(Number(value))
      : null;
    return buildSmartNumber(value, permissions, false);
  }

  // Classic 4-digit leading-zero octal permission: 0755, 0644, 0600
  if (/^0[0-7]{3}$/.test(trimmed)) {
    const value = BigInt(`0o${trimmed}`);
    return buildSmartNumber(value, formatUnixPermissions(Number(value)), true);
  }

  return null;
}
