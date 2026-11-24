import { MAX_QR_UTF8_BYTES } from './qrEncoder';

export interface SmartColor {
  kind: 'color';
  cssColor: string;
  hasAlpha: boolean;
  hex: string;
  rgb: string;
  hsl: string;
}

export interface SmartJson {
  kind: 'json';
  pretty: string;
  minified: string;
  alreadyPretty: boolean;
}

export interface SmartTimestamp {
  kind: 'timestamp';
  unit: 'seconds' | 'milliseconds';
  epochMs: number;
  local: string;
  iso: string;
}

export type SmartDetailMatch = SmartColor | SmartJson | SmartTimestamp;

const MAX_SMART_TEXT_CHARS = 200_000;
const NUMBER_RE = /^[+-]?(?:\d+\.?\d*|\.\d+)$/;
const PERCENT_RE = /^[+-]?(?:\d+\.?\d*|\.\d+)%$/;
const ANGLE_RE = /^([+-]?(?:\d+\.?\d*|\.\d+))(deg|rad|grad|turn)?$/i;
const HEX_RE = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
const CONTROL_RE = /[\x00-\x1f\x7f\u2028\u2029]/;

const MIN_UNIX_SECONDS = 1_000_000_000; // 2001-09-09T01:46:40Z
const MAX_UNIX_SECONDS = 4_102_444_800; // 2100-01-01T00:00:00Z

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function parseBoundedNumber(token: string, min: number, max: number): number | null {
  if (!NUMBER_RE.test(token)) return null;
  const value = Number(token);
  return Number.isFinite(value) && value >= min && value <= max ? value : null;
}

function parseBoundedPercent(token: string, min = 0, max = 100): number | null {
  if (!PERCENT_RE.test(token)) return null;
  const value = Number(token.slice(0, -1));
  return Number.isFinite(value) && value >= min && value <= max ? value : null;
}

function parseAlpha(token: string): number | null {
  const percent = parseBoundedPercent(token);
  if (percent !== null) return percent / 100;
  return parseBoundedNumber(token, 0, 1);
}

function parseAngleDegrees(token: string): number | null {
  const match = ANGLE_RE.exec(token);
  if (!match) return null;
  const value = Number(match[1]);
  if (!Number.isFinite(value)) return null;
  const unit = (match[2] ?? 'deg').toLowerCase();
  if (unit === 'rad') return (value * 180) / Math.PI;
  if (unit === 'grad') return value * 0.9;
  if (unit === 'turn') return value * 360;
  return value;
}

function splitColorArgs(inner: string): { channels: string[]; alpha: string | null } | null {
  if (inner.includes(',')) {
    if (inner.includes('/')) return null;
    const parts = inner.split(',').map((part) => part.trim());
    if ((parts.length !== 3 && parts.length !== 4) || parts.some((part) => !part)) return null;
    return { channels: parts.slice(0, 3), alpha: parts[3] ?? null };
  }
  const slashParts = inner.split('/');
  if (slashParts.length > 2) return null;
  const channels = slashParts[0].trim().split(/\s+/).filter(Boolean);
  if (channels.length !== 3) return null;
  const alpha = slashParts.length === 2 ? slashParts[1].trim() : null;
  if (slashParts.length === 2 && (!alpha || /\s/.test(alpha))) return null;
  return { channels, alpha };
}

function hslToRgb(hDeg: number, sPct: number, lPct: number): [number, number, number] {
  const h = (((hDeg % 360) + 360) % 360) / 360;
  const s = clamp(sPct / 100, 0, 1);
  const l = clamp(lPct / 100, 0, 1);
  if (s === 0) {
    const gray = Math.round(l * 255);
    return [gray, gray, gray];
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const hue2rgb = (t: number) => {
    let wrapped = t;
    if (wrapped < 0) wrapped += 1;
    if (wrapped > 1) wrapped -= 1;
    if (wrapped < 1 / 6) return p + (q - p) * 6 * wrapped;
    if (wrapped < 1 / 2) return q;
    if (wrapped < 2 / 3) return p + (q - p) * (2 / 3 - wrapped) * 6;
    return p;
  };
  return [
    Math.round(hue2rgb(h + 1 / 3) * 255),
    Math.round(hue2rgb(h) * 255),
    Math.round(hue2rgb(h - 1 / 3) * 255),
  ];
}

function oklchToRgb(l: number, c: number, hDeg: number): [number, number, number] {
  const rad = (hDeg * Math.PI) / 180;
  const a = c * Math.cos(rad);
  const b = c * Math.sin(rad);
  const l_ = l + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = l - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = l - 0.0894841775 * a - 1.2914855480 * b;
  const l3 = l_ * l_ * l_;
  const m3 = m_ * m_ * m_;
  const s3 = s_ * s_ * s_;
  const linR = +4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
  const linG = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
  const linB = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3;
  const toSrgb = (v: number) => {
    const clamped = clamp(v, 0, 1);
    const gamma = clamped <= 0.0031308 ? 12.92 * clamped : 1.055 * Math.pow(clamped, 1 / 2.4) - 0.055;
    return Math.round(clamp(gamma, 0, 1) * 255);
  };
  return [toSrgb(linR), toSrgb(linG), toSrgb(linB)];
}

function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, Math.round(l * 100)];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h = 0;
  if (max === rn) h = (gn - bn) / d + (gn < bn ? 6 : 0);
  else if (max === gn) h = (bn - rn) / d + 2;
  else h = (rn - gn) / d + 4;
  return [Math.round(h * 60) % 360, Math.round(s * 100), Math.round(l * 100)];
}

function formatAlpha(alpha: number): string {
  return String(Math.round(alpha * 1000) / 1000);
}

function toHexByte(value: number): string {
  return clamp(Math.round(value), 0, 255).toString(16).padStart(2, '0').toUpperCase();
}

function buildSmartColor(r: number, g: number, b: number, alpha: number, cssColor: string): SmartColor {
  const cr = clamp(Math.round(r), 0, 255);
  const cg = clamp(Math.round(g), 0, 255);
  const cb = clamp(Math.round(b), 0, 255);
  const ca = clamp(alpha, 0, 1);
  const hasAlpha = ca < 0.9995;
  const [h, s, l] = rgbToHsl(cr, cg, cb);
  const hex = hasAlpha
    ? `#${toHexByte(cr)}${toHexByte(cg)}${toHexByte(cb)}${toHexByte(ca * 255)}`
    : `#${toHexByte(cr)}${toHexByte(cg)}${toHexByte(cb)}`;
  const alphaText = formatAlpha(ca);
  const rgb = hasAlpha ? `rgba(${cr}, ${cg}, ${cb}, ${alphaText})` : `rgb(${cr}, ${cg}, ${cb})`;
  const hsl = hasAlpha ? `hsla(${h}, ${s}%, ${l}%, ${alphaText})` : `hsl(${h}, ${s}%, ${l}%)`;
  return { kind: 'color', cssColor, hasAlpha, hex, rgb, hsl };
}

export function parseSmartColor(trimmed: string): SmartColor | null {
  if (trimmed.length < 4 || trimmed.length > 96 || /[\r\n]/.test(trimmed)) return null;
  const hexMatch = HEX_RE.exec(trimmed);
  if (hexMatch) {
    const raw = hexMatch[1];
    if (raw.length === 4 && /^[0-9]{4}$/.test(raw)) return null;
    const lower = raw.toLowerCase();
    const expanded = lower.length === 3 || lower.length === 4
      ? lower.split('').map((ch) => ch + ch).join('')
      : lower;
    const r = parseInt(expanded.slice(0, 2), 16);
    const g = parseInt(expanded.slice(2, 4), 16);
    const b = parseInt(expanded.slice(4, 6), 16);
    const a = expanded.length === 8 ? parseInt(expanded.slice(6, 8), 16) / 255 : 1;
    return buildSmartColor(r, g, b, a, `#${lower}`);
  }

  const fnMatch = /^(rgba?|hsla?|oklch)\(\s*([^()]+)\s*\)$/i.exec(trimmed);
  if (!fnMatch) return null;
  const fn = fnMatch[1].toLowerCase();
  const parsedArgs = splitColorArgs(fnMatch[2]);
  if (!parsedArgs) return null;
  const { channels, alpha: alphaToken } = parsedArgs;
  const alpha = alphaToken !== null ? parseAlpha(alphaToken) : 1;
  if (alpha === null) return null;

  if (fn === 'rgb' || fn === 'rgba') {
    const nums = channels.map((token) => parseBoundedNumber(token, 0, 255));
    if (nums.every((n) => n !== null)) {
      const [r, g, b] = nums;
      const css = alpha < 1 ? `rgb(${r} ${g} ${b} / ${formatAlpha(alpha)})` : `rgb(${r} ${g} ${b})`;
      return buildSmartColor(r, g, b, alpha, css);
    }
    const pcts = channels.map((token) => parseBoundedPercent(token));
    if (pcts.every((n) => n !== null)) {
      const [r, g, b] = pcts;
      const css = alpha < 1 ? `rgb(${r}% ${g}% ${b}% / ${formatAlpha(alpha)})` : `rgb(${r}% ${g}% ${b}%)`;
      return buildSmartColor((r / 100) * 255, (g / 100) * 255, (b / 100) * 255, alpha, css);
    }
    return null;
  }

  if (fn === 'hsl' || fn === 'hsla') {
    const hue = parseAngleDegrees(channels[0]);
    const sat = parseBoundedPercent(channels[1]);
    const light = parseBoundedPercent(channels[2]);
    if (hue === null || sat === null || light === null) return null;
    const [r, g, b] = hslToRgb(hue, sat, light);
    const css = alpha < 1 ? `hsl(${hue}deg ${sat}% ${light}% / ${formatAlpha(alpha)})` : `hsl(${hue}deg ${sat}% ${light}%)`;
    return buildSmartColor(r, g, b, alpha, css);
  }

  if (fn === 'oklch') {
    if (fnMatch[2].includes(',')) return null;
    const lPct = parseBoundedPercent(channels[0]);
    const lNum = lPct === null ? parseBoundedNumber(channels[0], 0, 1) : null;
    const lVal = lPct !== null ? lPct / 100 : lNum;
    const cPct = parseBoundedPercent(channels[1]);
    const cNum = cPct === null ? parseBoundedNumber(channels[1], 0, 1) : null;
    const cVal = cPct !== null ? (cPct / 100) * 0.4 : cNum;
    const hue = parseAngleDegrees(channels[2]);
    if (lVal === null || cVal === null || hue === null) return null;
    const [r, g, b] = oklchToRgb(lVal, cVal, hue);
    const css = alpha < 1
      ? `oklch(${channels[0]} ${channels[1]} ${hue}deg / ${formatAlpha(alpha)})`
      : `oklch(${channels[0]} ${channels[1]} ${hue}deg)`;
    return buildSmartColor(r, g, b, alpha, css);
  }

  return null;
}

export function parseSmartJson(trimmed: string): SmartJson | null {
  if (trimmed.length < 2 || trimmed.length > MAX_SMART_TEXT_CHARS) return null;
  const first = trimmed[0];
  const last = trimmed[trimmed.length - 1];
  if (!((first === '{' && last === '}') || (first === '[' && last === ']'))) return null;
  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (typeof parsed !== 'object' || parsed === null) return null;
    const pretty = JSON.stringify(parsed, null, 2);
    const minified = JSON.stringify(parsed);
    return { kind: 'json', pretty, minified, alreadyPretty: trimmed === pretty };
  } catch {
    return null;
  }
}

function formatLocalTimestamp(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function parseSmartTimestamp(trimmed: string): SmartTimestamp | null {
  if (!/^\d{10}(?:\d{3})?$/.test(trimmed)) return null;
  const unit: 'seconds' | 'milliseconds' = trimmed.length === 10 ? 'seconds' : 'milliseconds';
  const raw = Number(trimmed);
  if (!Number.isSafeInteger(raw)) return null;
  const seconds = unit === 'seconds' ? raw : raw / 1000;
  if (seconds < MIN_UNIX_SECONDS || seconds > MAX_UNIX_SECONDS) return null;
  const epochMs = unit === 'seconds' ? raw * 1000 : raw;
  const date = new Date(epochMs);
  if (Number.isNaN(date.getTime())) return null;
  return { kind: 'timestamp', unit, epochMs, local: formatLocalTimestamp(date), iso: date.toISOString() };
}

export function analyzeSmartDetail(raw: string): SmartDetailMatch | null {
  if (!raw || raw.length > MAX_SMART_TEXT_CHARS) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  return parseSmartColor(trimmed) ?? parseSmartTimestamp(trimmed) ?? parseSmartJson(trimmed);
}

export function canEncodeQr(raw: string): boolean {
  const trimmed = raw.trim();
  if (!trimmed || trimmed.length > MAX_QR_UTF8_BYTES) return false;
  return new TextEncoder().encode(trimmed).length <= MAX_QR_UTF8_BYTES;
}

export function isOpenableQrLink(raw: string): boolean {
  const value = raw.trim();
  if (!value || value.length > 4096 || CONTROL_RE.test(value) || /\s/.test(value) || value.includes('\\')) return false;
  const lower = value.toLowerCase();
  const hasScheme = lower.startsWith('http://') || lower.startsWith('https://');
  const hasWww = !hasScheme && lower.startsWith('www.');
  if (!hasScheme && !hasWww) return false;
  if (hasScheme && value[value.indexOf('://') + 3] === '/') return false;
  try {
    const url = new URL(hasWww ? `https://${value}` : value);
    if (!url.hostname || url.username || url.password) return false;
    if (hasWww && !url.hostname.slice(4).includes('.')) return false;
    return true;
  } catch {
    return false;
  }
}
