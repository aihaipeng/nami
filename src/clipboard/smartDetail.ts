import { MAX_QR_UTF8_BYTES } from './qrEncoder';
import {
  computeNextCronRuns,
  describeCron,
  formatCronRunTime,
  parseSmartCron,
  type CronField,
  type SmartCron,
} from './smartCron';
import {
  parseSmartMath,
  parseSmartNumber,
  type SmartMath,
  type SmartNumber,
} from './smartMath';

export {
  computeNextCronRuns,
  describeCron,
  formatCronRunTime,
  parseSmartCron,
  parseSmartMath,
  parseSmartNumber,
};
export type { CronField, SmartCron, SmartMath, SmartNumber };

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

export interface SmartDateTime {
  kind: 'datetime';
  epochMs: number;
  unixSeconds: string;
  unixMilliseconds: string;
  local: string;
  iso: string;
  showLocal: boolean;
  showIso: boolean;
}

export interface SmartUrlParam {
  key: string;
  value: string;
}

export interface SmartUrl {
  kind: 'url';
  cleanUrl: string | null;
  removedTrackingCount: number;
  decodedUrl: string | null;
  params: SmartUrlParam[];
  totalParams: number;
}

export interface SmartJwt {
  kind: 'jwt';
  algorithm: string | null;
  headerPretty: string;
  payloadPretty: string;
  issuedAtMs: number | null;
  notBeforeMs: number | null;
  expiresAtMs: number | null;
}

export interface SmartDecoded {
  kind: 'decoded';
  encoding: 'escape' | 'base64' | 'url';
  text: string;
  jsonPretty: string | null;
}

export interface SmartImageData {
  kind: 'imageData';
  sourceKind: 'svg' | 'data-uri';
  format: string;
  src: string;
  estimatedBytes: number;
}

export type SmartDetailMatch =
  | SmartColor
  | SmartJson
  | SmartTimestamp
  | SmartDateTime
  | SmartUrl
  | SmartJwt
  | SmartImageData
  | SmartNumber
  | SmartMath
  | SmartCron
  | SmartDecoded;

const MAX_SMART_TEXT_CHARS = 200_000;
const MAX_URL_PARAMS = 24;
const NUMBER_RE = /^[+-]?(?:\d+\.?\d*|\.\d+)$/;
const PERCENT_RE = /^[+-]?(?:\d+\.?\d*|\.\d+)%$/;
const ANGLE_RE = /^([+-]?(?:\d+\.?\d*|\.\d+))(deg|rad|grad|turn)?$/i;
const HEX_RE = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
const CONTROL_RE = /[\x00-\x1f\x7f\u2028\u2029]/;
const DECODED_CONTROL_RE = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f\ufffd]/;
const TRACKING_PARAMS = new Set([
  'fbclid', 'gclid', 'gbraid', 'wbraid', 'dclid', 'msclkid', 'twclid', 'ttclid', 'yclid', 'li_fat_id',
  'mc_cid', 'mc_eid', 'igshid', 'igsh', 'si', 'spm', 'spm_id_from', 'from_spmid', 'vd_source',
  'share_source', 'share_medium', 'share_plat', 'share_session_id', 'share_tag', 'share_times',
  'share_from', 'is_story_h5', 'buvid', 'up_id', 'ref_src', 'ref_url', '_hsenc', '_hsmi', 'mkt_tok',
  'oly_anon_id', 'oly_enc_id', 'rb_clickid', 'wickedid', 's_kwcid', 'ef_id', 'guccounter',
  'guce_referrer', 'guce_referrer_sig',
]);
const RFC2822_MONTHS: Record<string, number> = {
  jan: 1, feb: 2, mar: 3, apr: 4, may: 5, jun: 6,
  jul: 7, aug: 8, sep: 9, oct: 10, nov: 11, dec: 12,
};
const COMMON_URL_ENCODED_TOKENS = new Set([
  '%20', '%2f', '%3a', '%3d', '%3f', '%26', '%2b', '%40', '%23', '%25', '%2c',
]);

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

export function formatLocalTimestamp(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function formatRelativeTime(epochMs: number, locale: 'zh-CN' | 'en', nowMs = Date.now()): string {
  const diffSec = Math.round((epochMs - nowMs) / 1000);
  if (Math.abs(diffSec) < 5) return locale === 'zh-CN' ? '刚刚' : 'just now';
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const abs = Math.abs(diffSec);
  if (abs < 60) return rtf.format(diffSec, 'second');
  if (abs < 3600) return rtf.format(Math.round(diffSec / 60), 'minute');
  if (abs < 86_400) return rtf.format(Math.round(diffSec / 3600), 'hour');
  if (abs < 86_400 * 30) return rtf.format(Math.round(diffSec / 86_400), 'day');
  if (abs < 86_400 * 365) return rtf.format(Math.round(diffSec / (86_400 * 30)), 'month');
  return rtf.format(Math.round(diffSec / (86_400 * 365)), 'year');
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

function isValidCalendarDate(year: number, month: number, day: number): boolean {
  if (year < 1970 || year > 2100 || month < 1 || month > 12 || day < 1) return false;
  const maxDay = new Date(Date.UTC(year, month, 0)).getUTCDate();
  return day <= maxDay;
}

function isValidTime(hour: number, minute: number, second: number): boolean {
  return hour >= 0 && hour <= 23 && minute >= 0 && minute <= 59 && second >= 0 && second <= 59;
}

function parseTimezoneOffsetMs(token: string): number | null {
  const upper = token.toUpperCase();
  if (upper === 'Z' || upper === 'GMT' || upper === 'UTC') return 0;
  const match = /^([+-])(\d{2}):?(\d{2})$/.exec(token);
  if (!match) return null;
  const sign = match[1] === '+' ? 1 : -1;
  const hours = Number(match[2]);
  const minutes = Number(match[3]);
  if (hours > 14 || minutes > 59 || (hours === 14 && minutes > 0)) return null;
  return sign * (hours * 60 + minutes) * 60_000;
}

function buildSmartDateTime(epochMs: number, trimmed: string): SmartDateTime | null {
  if (!Number.isFinite(epochMs)) return null;
  const date = new Date(epochMs);
  if (Number.isNaN(date.getTime())) return null;
  const local = formatLocalTimestamp(date);
  const iso = date.toISOString();
  const isAlreadyIsoUtc = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?Z$/i.test(trimmed);
  return {
    kind: 'datetime',
    epochMs,
    unixSeconds: String(Math.floor(epochMs / 1000)),
    unixMilliseconds: String(epochMs),
    local,
    iso,
    showLocal: local !== trimmed,
    showIso: !isAlreadyIsoUtc && iso !== trimmed,
  };
}

export function parseSmartDateTime(trimmed: string): SmartDateTime | null {
  if (trimmed.length < 10 || trimmed.length > 64 || /[\r\n]/.test(trimmed)) return null;

  const isoMatch = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?(Z|[+-]\d{2}:?\d{2})?)?$/i.exec(trimmed);
  if (isoMatch) {
    const year = Number(isoMatch[1]);
    const month = Number(isoMatch[2]);
    const day = Number(isoMatch[3]);
    if (!isValidCalendarDate(year, month, day)) return null;
    const hasTime = isoMatch[4] !== undefined;
    const hour = hasTime ? Number(isoMatch[4]) : 0;
    const minute = hasTime ? Number(isoMatch[5]) : 0;
    const second = isoMatch[6] !== undefined ? Number(isoMatch[6]) : 0;
    const ms = isoMatch[7] !== undefined ? Number(isoMatch[7].slice(0, 3).padEnd(3, '0')) : 0;
    if (!isValidTime(hour, minute, second)) return null;
    if (isoMatch[8]) {
      const offsetMs = parseTimezoneOffsetMs(isoMatch[8]);
      if (offsetMs === null) return null;
      return buildSmartDateTime(Date.UTC(year, month - 1, day, hour, minute, second, ms) - offsetMs, trimmed);
    }
    return buildSmartDateTime(new Date(year, month - 1, day, hour, minute, second, ms).getTime(), trimmed);
  }

  const slashMatch = /^(\d{4})\/(\d{1,2})\/(\d{1,2})[ T](\d{1,2}):(\d{2})(?::(\d{2}))?$/.exec(trimmed);
  if (slashMatch) {
    const year = Number(slashMatch[1]);
    const month = Number(slashMatch[2]);
    const day = Number(slashMatch[3]);
    const hour = Number(slashMatch[4]);
    const minute = Number(slashMatch[5]);
    const second = slashMatch[6] !== undefined ? Number(slashMatch[6]) : 0;
    if (!isValidCalendarDate(year, month, day) || !isValidTime(hour, minute, second)) return null;
    return buildSmartDateTime(new Date(year, month - 1, day, hour, minute, second, 0).getTime(), trimmed);
  }

  const rfcMatch = /^(?:(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun),\s*)?(\d{1,2})\s+(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+(\d{4})\s+(\d{2}):(\d{2})(?::(\d{2}))?\s+(GMT|UTC|[+-]\d{4})$/i.exec(trimmed);
  if (rfcMatch) {
    const day = Number(rfcMatch[1]);
    const month = RFC2822_MONTHS[rfcMatch[2].toLowerCase()];
    const year = Number(rfcMatch[3]);
    const hour = Number(rfcMatch[4]);
    const minute = Number(rfcMatch[5]);
    const second = rfcMatch[6] !== undefined ? Number(rfcMatch[6]) : 0;
    const offsetMs = parseTimezoneOffsetMs(rfcMatch[7]);
    if (!month || offsetMs === null || !isValidCalendarDate(year, month, day) || !isValidTime(hour, minute, second)) return null;
    return buildSmartDateTime(Date.UTC(year, month - 1, day, hour, minute, second, 0) - offsetMs, trimmed);
  }

  return null;
}

function isTrackingParam(key: string): boolean {
  const lower = key.toLowerCase();
  return lower.startsWith('utm_') || TRACKING_PARAMS.has(lower);
}

export function parseSmartUrl(trimmed: string): SmartUrl | null {
  if (!trimmed || trimmed.length > 8192 || CONTROL_RE.test(trimmed) || /\s/.test(trimmed) || trimmed.includes('\\')) return null;
  const lower = trimmed.toLowerCase();
  const hasScheme = lower.startsWith('http://') || lower.startsWith('https://');
  const hasWww = !hasScheme && lower.startsWith('www.');
  if (!hasScheme && !hasWww) return null;
  if (hasScheme && trimmed[trimmed.indexOf('://') + 3] === '/') return null;

  let url: URL;
  try {
    url = new URL(hasWww ? `https://${trimmed}` : trimmed);
  } catch {
    return null;
  }
  if (!url.hostname || url.username || url.password) return null;
  if (hasWww && !url.hostname.slice(4).includes('.')) return null;

  const allParams = Array.from(url.searchParams.entries());
  let removedTrackingCount = 0;
  const kept: Array<[string, string]> = [];
  for (const [key, value] of allParams) {
    if (isTrackingParam(key)) removedTrackingCount += 1;
    else kept.push([key, value]);
  }

  let cleanUrl: string | null = null;
  if (removedTrackingCount > 0) {
    const clean = new URL(url.toString());
    clean.search = '';
    for (const [key, value] of kept) clean.searchParams.append(key, value);
    const serialized = clean.toString();
    if (serialized !== trimmed) cleanUrl = serialized;
  }

  let decodedUrl: string | null = null;
  if (/%[0-9A-Fa-f]{2}/.test(trimmed)) {
    try {
      const decoded = decodeURI(trimmed);
      if (decoded !== trimmed && !CONTROL_RE.test(decoded)) decodedUrl = decoded;
    } catch {
      // Ignore malformed percent sequences outside valid UTF-8.
    }
  }

  if (cleanUrl === null && decodedUrl === null && allParams.length === 0) return null;
  return {
    kind: 'url',
    cleanUrl,
    removedTrackingCount,
    decodedUrl,
    params: allParams.slice(0, MAX_URL_PARAMS).map(([key, value]) => ({ key, value })),
    totalParams: allParams.length,
  };
}

function decodeBase64Bytes(input: string, urlSafe: boolean): Uint8Array | null {
  if (input.length === 0 || input.length % 4 === 1) return null;
  const normalized = urlSafe ? input.replace(/-/g, '+').replace(/_/g, '/') : input;
  const padLen = (4 - (normalized.length % 4)) % 4;
  const padded = padLen ? `${normalized}${'='.repeat(padLen)}` : normalized;
  try {
    const binary = atob(padded);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return bytes;
  } catch {
    return null;
  }
}

function decodeUtf8Strict(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

function parseJwtClaimEpochMs(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  if (value < 0 || value > MAX_UNIX_SECONDS) return null;
  return Math.round(value * 1000);
}

export function parseSmartJwt(trimmed: string): SmartJwt | null {
  if (trimmed.length < 20 || trimmed.length > MAX_SMART_TEXT_CHARS || /[\s]/.test(trimmed)) return null;
  const match = /^([A-Za-z0-9_-]{8,})\.([A-Za-z0-9_-]{8,})\.([A-Za-z0-9_-]*)$/.exec(trimmed);
  if (!match) return null;
  const headerBytes = decodeBase64Bytes(match[1], true);
  const payloadBytes = decodeBase64Bytes(match[2], true);
  if (!headerBytes || !payloadBytes) return null;
  const headerText = decodeUtf8Strict(headerBytes);
  const payloadText = decodeUtf8Strict(payloadBytes);
  if (!headerText || !payloadText) return null;
  try {
    const header: unknown = JSON.parse(headerText);
    const payload: unknown = JSON.parse(payloadText);
    if (typeof header !== 'object' || header === null || Array.isArray(header)) return null;
    if (typeof payload !== 'object' || payload === null || Array.isArray(payload)) return null;
    const headerRecord = header as Record<string, unknown>;
    const payloadRecord = payload as Record<string, unknown>;
    if (typeof headerRecord.alg !== 'string' && typeof headerRecord.typ !== 'string') return null;
    return {
      kind: 'jwt',
      algorithm: typeof headerRecord.alg === 'string' ? headerRecord.alg : null,
      headerPretty: JSON.stringify(headerRecord, null, 2),
      payloadPretty: JSON.stringify(payloadRecord, null, 2),
      issuedAtMs: parseJwtClaimEpochMs(payloadRecord.iat),
      notBeforeMs: parseJwtClaimEpochMs(payloadRecord.nbf),
      expiresAtMs: parseJwtClaimEpochMs(payloadRecord.exp),
    };
  } catch {
    return null;
  }
}

function isPrintableDecodedText(text: string, strictReadable = false, minLength = strictReadable ? 3 : 2): boolean {
  if (text.length < minLength || DECODED_CONTROL_RE.test(text)) return false;
  if (!strictReadable) return true;
  let readable = 0;
  let total = 0;
  for (const ch of text) {
    total += 1;
    const code = ch.codePointAt(0)!;
    if (
      code === 0x09 || code === 0x0a || code === 0x0d
      || (code >= 0x20 && code <= 0x7e)
      || (code >= 0x00a0 && code <= 0x024f)
      || (code >= 0x2000 && code <= 0x206f)
      || (code >= 0x3000 && code <= 0x30ff)
      || (code >= 0x4e00 && code <= 0x9fff)
      || (code >= 0xac00 && code <= 0xd7af)
      || (code >= 0xff00 && code <= 0xffef)
      || (code >= 0x1f300 && code <= 0x1faff)
    ) {
      readable += 1;
    }
  }
  return total > 0 && readable / total >= 0.9;
}

function tryPrettyJson(text: string): string | null {
  const json = parseSmartJson(text.trim());
  return json ? json.pretty : null;
}

function isProseIsolatedEscape(input: string, start: number, endExclusive: number): boolean {
  const prevWs = start === 0 || /\s/.test(input[start - 1]);
  const nextCh = input[endExclusive];
  const nextBoundary = nextCh === undefined || /[\s.,;:!?)]/.test(nextCh);
  return prevWs && nextBoundary;
}

function unescapeTextSequences(input: string): {
  text: string;
  unicodeCount: number;
  strongCount: number;
  nonIsolatedStrongCount: number;
  standardCount: number;
  unknownCount: number;
  hasUnescapedDoubleQuote: boolean;
} {
  let out = '';
  let unicodeCount = 0;
  let strongCount = 0;
  let nonIsolatedStrongCount = 0;
  let standardCount = 0;
  let unknownCount = 0;
  let hasUnescapedDoubleQuote = false;
  for (let i = 0; i < input.length; i += 1) {
    const ch = input[i];
    if (ch !== '\\' || i + 1 >= input.length) {
      if (ch === '"') hasUnescapedDoubleQuote = true;
      out += ch;
      continue;
    }
    const next = input[i + 1];
    if (next === 'u') {
      if (input[i + 2] === '{') {
        const end = input.indexOf('}', i + 3);
        const hex = end > i + 3 ? input.slice(i + 3, end) : '';
        if (/^[0-9a-fA-F]{1,6}$/.test(hex)) {
          const cp = parseInt(hex, 16);
          if (cp <= 0x10ffff) {
            out += String.fromCodePoint(cp);
            unicodeCount += 1;
            i = end;
            continue;
          }
        }
      } else {
        const hex = input.slice(i + 2, i + 6);
        if (/^[0-9a-fA-F]{4}$/.test(hex)) {
          out += String.fromCharCode(parseInt(hex, 16));
          unicodeCount += 1;
          i += 5;
          continue;
        }
      }
      unknownCount += 1;
      out += '\\u';
      i += 1;
    } else if (next === 'r' && input[i + 2] === '\\' && input[i + 3] === 'n') {
      out += '\r\n';
      strongCount += 2;
      standardCount += 2;
      if (!isProseIsolatedEscape(input, i, i + 4)) nonIsolatedStrongCount += 2;
      i += 3;
    } else if (next === 'n') {
      out += '\n';
      strongCount += 1;
      standardCount += 1;
      if (!isProseIsolatedEscape(input, i, i + 2)) nonIsolatedStrongCount += 1;
      i += 1;
    } else if (next === 'r') {
      out += '\r';
      strongCount += 1;
      standardCount += 1;
      if (!isProseIsolatedEscape(input, i, i + 2)) nonIsolatedStrongCount += 1;
      i += 1;
    } else if (next === 't') {
      out += '\t';
      strongCount += 1;
      standardCount += 1;
      if (!isProseIsolatedEscape(input, i, i + 2)) nonIsolatedStrongCount += 1;
      i += 1;
    } else if (next === 'b') {
      out += '\b'; standardCount += 1; i += 1;
    } else if (next === 'f') {
      out += '\f'; standardCount += 1; i += 1;
    } else if (next === '"') {
      out += '"'; strongCount += 1; nonIsolatedStrongCount += 1; standardCount += 1; i += 1;
    } else if (next === '\'' || next === '\\' || next === '/') {
      out += next; standardCount += 1; i += 1;
    } else {
      unknownCount += 1;
      out += `\\${next}`;
      i += 1;
    }
  }
  return {
    text: out,
    unicodeCount,
    strongCount,
    nonIsolatedStrongCount,
    standardCount,
    unknownCount,
    hasUnescapedDoubleQuote,
  };
}

export function parseSmartEscape(trimmed: string): SmartDecoded | null {
  if (trimmed.length < 4 || trimmed.length > MAX_SMART_TEXT_CHARS || !trimmed.includes('\\')) return null;

  // 1. Valid JSON-quoted string literal (e.g. "{\"ok\":true,\n\"n\":1}" or "C:\\Users\\Alice").
  if (trimmed.startsWith('"') && trimmed.endsWith('"') && trimmed.length >= 6 && /\\(?:u[0-9a-fA-F]{4}|["\\/bfnrt])/.test(trimmed)) {
    try {
      const parsed: unknown = JSON.parse(trimmed);
      if (typeof parsed === 'string' && parsed !== trimmed.slice(1, -1) && isPrintableDecodedText(parsed)) {
        return { kind: 'decoded', encoding: 'escape', text: parsed, jsonPretty: tryPrettyJson(parsed) };
      }
    } catch {
      // Fall through to unquoted escape handling.
    }
  }

  // Reject Windows/UNC paths (leading or embedded), regex literals, and braced LaTeX commands.
  if (
    /(?:^|[\s"'(=])(?:[A-Za-z]:\\|\\\\[A-Za-z0-9._-]+\\)/.test(trimmed)
    || /^\/.+\/[dgimsuvy]*$/.test(trimmed)
    || /\\(?:frac|text(?:bf|it|tt|rm)?|begin|end|sqrt|sum|int)\{/.test(trimmed)
  ) {
    return null;
  }

  const unescaped = unescapeTextSequences(trimmed);
  const hasRealNewline = /[\r\n]/.test(trimmed);
  const hasEmbeddedSingleQuoted = /(?:^|[\s(=:,])'[^'\r\n]*\\[uUnrtbf"'\\][^'\r\n]*'(?:$|[\s),;])/.test(trimmed);

  // 2. Unicode escape sequences (\uXXXX / \u{XXXXX}).
  if (unescaped.unicodeCount > 0) {
    if (hasRealNewline) {
      if (
        unescaped.unicodeCount < 2
        || unescaped.unknownCount > 0
        || unescaped.hasUnescapedDoubleQuote
        || hasEmbeddedSingleQuoted
        || trimmed.startsWith('#!')
      ) {
        return null;
      }
    } else if (
      unescaped.unicodeCount === 1
      && (unescaped.hasUnescapedDoubleQuote || hasEmbeddedSingleQuoted || unescaped.unknownCount > 0)
    ) {
      return null;
    }
  } else {
    // 3. Unquoted non-Unicode escapes: only single-line escaped payloads without raw quotes/code literals.
    if (
      hasRealNewline
      || unescaped.unknownCount > 0
      || unescaped.hasUnescapedDoubleQuote
      || hasEmbeddedSingleQuoted
      || unescaped.strongCount < 2
      || unescaped.nonIsolatedStrongCount < 1
    ) {
      return null;
    }
  }

  const minLength = unescaped.unicodeCount > 0 ? 1 : 2;
  if (unescaped.text === trimmed || !isPrintableDecodedText(unescaped.text, false, minLength)) return null;
  return { kind: 'decoded', encoding: 'escape', text: unescaped.text, jsonPretty: tryPrettyJson(unescaped.text) };
}

export function parseSmartPercentEncoded(trimmed: string): SmartDecoded | null {
  if (trimmed.length < 3 || trimmed.length > MAX_SMART_TEXT_CHARS || !trimmed.includes('%') || /[\r\n]/.test(trimmed)) return null;
  const matches = trimmed.match(/%[0-9A-Fa-f]{2}/g);
  if (!matches) return null;
  if (matches.length < 2 && !COMMON_URL_ENCODED_TOKENS.has(matches[0].toLowerCase())) return null;
  try {
    const decoded = decodeURIComponent(trimmed);
    if (decoded === trimmed || !isPrintableDecodedText(decoded)) return null;
    return { kind: 'decoded', encoding: 'url', text: decoded, jsonPretty: tryPrettyJson(decoded) };
  } catch {
    return null;
  }
}

export function parseSmartBase64(trimmed: string): SmartDecoded | null {
  if (trimmed.length < 8 || trimmed.length > MAX_SMART_TEXT_CHARS || /[ \t]/.test(trimmed)) return null;
  const compact = trimmed.replace(/[\r\n]+/g, '');
  if (compact.length < 8 || compact.length % 4 === 1) return null;

  const isStandard = /^[A-Za-z0-9+/]+={0,2}$/.test(compact);
  const isUrlSafe = !isStandard && /^[A-Za-z0-9_-]+={0,2}$/.test(compact);
  if (!isStandard && !isUrlSafe) return null;

  const padded = compact.endsWith('=');
  if (!padded) {
    if (compact.length < 16) return null;
    if (/^[0-9a-fA-F]+$/.test(compact)) return null;
    if (/^[a-z]+$/.test(compact) || /^[A-Z]+$/.test(compact)) return null;
    if (/^[a-z0-9]+(?:[-_][a-z0-9]+)+$/.test(compact)) return null;
  }

  const bytes = decodeBase64Bytes(compact, isUrlSafe);
  if (!bytes || bytes.length < 3) return null;
  const decoded = decodeUtf8Strict(bytes);
  if (!decoded || decoded === trimmed || !isPrintableDecodedText(decoded, true)) return null;
  return { kind: 'decoded', encoding: 'base64', text: decoded, jsonPretty: tryPrettyJson(decoded) };
}

const IMAGE_DATA_URI_RE = /^data:image\/(png|jpe?g|gif|webp|svg\+xml|avif|bmp|x-icon|vnd\.microsoft\.icon|ico)((?:;[a-z0-9.+-]+=[a-z0-9.+-]+)*)(;base64)?,([\s\S]+)$/i;

const IMAGE_FORMAT_LABELS: Record<string, string> = {
  png: 'PNG',
  jpg: 'JPEG',
  jpeg: 'JPEG',
  gif: 'GIF',
  webp: 'WebP',
  'svg+xml': 'SVG',
  avif: 'AVIF',
  bmp: 'BMP',
  'x-icon': 'ICO',
  'vnd.microsoft.icon': 'ICO',
  ico: 'ICO',
};

function ensureSvgXmlns(svg: string): string {
  const openEnd = svg.indexOf('>');
  if (openEnd === -1) return svg;
  const openTag = svg.slice(0, openEnd + 1);
  if (/\bxmlns\s*=/i.test(openTag)) return svg;
  return `${svg.slice(0, 4)} xmlns="http://www.w3.org/2000/svg"${svg.slice(4)}`;
}

function stripSvgProlog(raw: string): string {
  let rest = raw.trim();
  while (true) {
    if (rest.startsWith('<?xml')) {
      const end = rest.indexOf('?>');
      if (end === -1) return '';
      rest = rest.slice(end + 2).trimStart();
    } else if (/^<!DOCTYPE\s+svg\b/i.test(rest)) {
      const end = rest.indexOf('>');
      if (end === -1) return '';
      rest = rest.slice(end + 1).trimStart();
    } else if (rest.startsWith('<!--')) {
      const end = rest.indexOf('-->');
      if (end === -1) return '';
      rest = rest.slice(end + 3).trimStart();
    } else {
      break;
    }
  }
  while (rest.endsWith('-->')) {
    const start = rest.lastIndexOf('<!--');
    if (start === -1) break;
    rest = rest.slice(0, start).trimEnd();
  }
  return rest;
}

export function parseSmartImageData(trimmed: string): SmartImageData | null {
  if (trimmed.length < 15 || trimmed.length > MAX_SMART_TEXT_CHARS) return null;

  if (/^data:image\//i.test(trimmed)) {
    const match = IMAGE_DATA_URI_RE.exec(trimmed);
    if (!match) return null;
    const subtype = match[1].toLowerCase();
    const params = match[2] ?? '';
    const isBase64 = Boolean(match[3]);
    const payload = match[4];
    const format = IMAGE_FORMAT_LABELS[subtype] ?? subtype.toUpperCase();

    if (isBase64) {
      const clean = payload.replace(/\s+/g, '');
      if (clean.length < 8 || clean.length % 4 === 1 || !/^[A-Za-z0-9+/]+={0,2}$/.test(clean)) return null;
      const padding = clean.endsWith('==') ? 2 : clean.endsWith('=') ? 1 : 0;
      const estimatedBytes = Math.max(0, Math.floor((clean.length * 3) / 4) - padding);
      return {
        kind: 'imageData',
        sourceKind: 'data-uri',
        format,
        src: `data:image/${subtype}${params};base64,${clean}`,
        estimatedBytes,
      };
    }

    if (subtype !== 'svg+xml') return null;
    let decoded = payload;
    try {
      decoded = decodeURIComponent(payload);
    } catch {
      // Allow unencoded inline SVG inside data:image/svg+xml,...
    }
    const svg = stripSvgProlog(decoded);
    if (!/^<svg(?=[\s>])/i.test(svg) || !/<\/svg>$/i.test(svg)) return null;
    const normalizedSvg = ensureSvgXmlns(svg);
    return {
      kind: 'imageData',
      sourceKind: 'data-uri',
      format: 'SVG',
      src: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(normalizedSvg)}`,
      estimatedBytes: new TextEncoder().encode(decoded).length,
    };
  }

  if (trimmed[0] === '<') {
    const svg = stripSvgProlog(trimmed);
    if (svg.length < 11 || !/^<svg(?=[\s>])/i.test(svg) || !/<\/svg>$/i.test(svg)) return null;
    const openEnd = svg.indexOf('>');
    if (openEnd === -1) return null;
    const normalizedSvg = ensureSvgXmlns(svg);
    return {
      kind: 'imageData',
      sourceKind: 'svg',
      format: 'SVG',
      src: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(normalizedSvg)}`,
      estimatedBytes: new TextEncoder().encode(trimmed).length,
    };
  }

  return null;
}

export function analyzeSmartDetail(raw: string): SmartDetailMatch | null {
  if (!raw || raw.length > MAX_SMART_TEXT_CHARS) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  return parseSmartColor(trimmed)
    ?? parseSmartTimestamp(trimmed)
    ?? parseSmartDateTime(trimmed)
    ?? parseSmartJwt(trimmed)
    ?? parseSmartUrl(trimmed)
    ?? parseSmartImageData(trimmed)
    ?? parseSmartJson(trimmed)
    ?? parseSmartNumber(trimmed)
    ?? parseSmartMath(trimmed)
    ?? parseSmartCron(trimmed)
    ?? parseSmartEscape(trimmed)
    ?? parseSmartPercentEncoded(trimmed)
    ?? parseSmartBase64(trimmed);
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
