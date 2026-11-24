export type RecognizedText =
  | { kind: 'plain' }
  | { kind: 'color'; cssColor: string; hasAlpha: boolean }
  | { kind: 'link'; prefix: string; host: string; rest: string }
  | { kind: 'email'; local: string; domain: string }
  | { kind: 'code' };

export interface TextRecognitionResult {
  leading: string;
  trimmed: string;
  trailing: string;
  match: RecognizedText;
}

const NUMBER_RE = /^[+-]?(?:\d+\.?\d*|\.\d+)$/;
const PERCENT_RE = /^[+-]?(?:\d+\.?\d*|\.\d+)%$/;
const ANGLE_RE = /^([+-]?(?:\d+\.?\d*|\.\d+))(deg|rad|grad|turn)?$/i;
const HEX_RE = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
const CONTROL_RE = /[\x00-\x1f\x7f\u2028\u2029]/;
const EMAIL_LOCAL_PART_RE = /^[A-Za-z0-9!#$%&'*+\-/=?^_`{|}~]+$/;
const EMAIL_DOMAIN_LABEL_RE = /^[A-Za-z0-9-]+$/;

const CLI_PATTERNS = [
  /^#!/,
  /^(?:\$\s+)?(?:sudo\s+)?git\s+(?:add|bisect|branch|checkout|cherry-pick|clean|clone|commit|config|diff|fetch|grep|init|log|merge|mv|pull|push|rebase|reflog|remote|reset|restore|revert|rm|show|stash|status|submodule|switch|tag|worktree)\b/,
  /^(?:\$\s+)?(?:sudo\s+)?(?:npm|pnpm|yarn|bun)\s+(?:add|audit|build|ci|create|dev|dlx|exec|i|init|install|link|ls|outdated|pack|publish|remove|rm|run|start|test|uninstall|update|upgrade|why)\b/,
  /^(?:\$\s+)?(?:sudo\s+)?npx\s+\S+/,
  /^(?:\$\s+)?(?:sudo\s+)?cargo\s+(?:add|bench|build|check|clean|clippy|doc|fetch|fix|fmt|init|install|metadata|new|publish|remove|run|test|tree|update)\b/,
  /^(?:\$\s+)?(?:sudo\s+)?docker(?:\s+compose)?\s+(?:attach|build|compose|cp|create|down|exec|images|inspect|kill|logs|network|ps|pull|push|restart|rm|rmi|run|start|stats|stop|system|tag|up|volume)\b/,
  /^(?:\$\s+)?(?:sudo\s+)?(?:kubectl|helm|brew|curl|wget|ssh|scp|rsync|chmod|chown|sqlite3|psql|mysql|redis-cli|ffmpeg|xcodebuild|rustup|rustc|rg|grep|sed|awk|tar|terraform|gh|pytest|uv|pip3?|python3?)\s+(?:-[A-Za-z0-9]|--[A-Za-z0-9]|\S*[/@=.~:+])/,
  /^(?:\$\s+)?(?:sudo\s+)?(?:brew|kubectl|helm|gh|terraform|rustup)\s+(?:apply|auth|create|delete|describe|destroy|edit|exec|fmt|get|info|init|install|issue|list|logs|outdated|plan|pr|release|repo|search|show|status|tap|uninstall|update|upgrade|validate|workflow)\b/,
  /^(?:\$\s+)?(?:sudo\s+)?go\s+(?:build|doc|env|fmt|generate|get|install|list|mod|run|test|tool|vet|work)\b/,
  /^(?:\$\s+)?(?:sudo\s+)?(?:find|ls|rm|cp|mv|mkdir|cat|head|tail)\s+(?:-[A-Za-z]+|[./~])/,
];

const SQL_RE = /^(?:SELECT\b[\s\S]+\bFROM\b|INSERT\s+INTO\b|UPDATE\s+\S+\s+SET\b|DELETE\s+FROM\b|CREATE\s+(?:OR\s+REPLACE\s+)?(?:TABLE|INDEX|VIEW|FUNCTION|TRIGGER)\b|ALTER\s+TABLE\b|DROP\s+(?:TABLE|INDEX|VIEW)\b|WITH\s+[A-Za-z_][A-Za-z0-9_]*\s+AS\s*\()/i;

const CODE_START_RE = /^(?:import\s+(?:type\s+)?[\w*{]|export\s+(?:default\s+|const\s+|let\s+|function\s+|async\s+|class\s+|type\s+|interface\s+|enum\s+)|(?:export\s+)?(?:async\s+)?function\s+[A-Za-z_$]|(?:const|let|var)\s+[A-Za-z_${[][\w$,\s{}[\]:]*=\s*\S|(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|impl|mod|type)\s+[A-Za-z_]|use\s+[A-Za-z_][A-Za-z0-9_]*::|def\s+[A-Za-z_]\w*\s*\(|from\s+[A-Za-z_][\w.]*\s+import\s+|func\s+(?:\([^)]+\)\s+)?[A-Za-z_]\w*\s*\(|#include\s+[<"]|package\s+[a-z_][\w.]*\s*;?|@(?:media|keyframes|supports|layer)\s+)/;

const SINGLE_LINE_CSS_RE = /^[.#@:[\w][^{\n]{0,80}\{\s*[a-z-]+\s*:\s*[^;\n]+;?\s*\}$/;
const HTML_START_RE = /^(?:<!DOCTYPE\s+html\b|<!doctype\s+html\b|<\?xml\b|<[A-Za-z][A-Za-z0-9:-]*(?:\s+[^<>]*?)?>)/;

function trimBounds(raw: string) {
  let start = 0;
  let end = raw.length;
  while (start < end && /\s/.test(raw[start])) start += 1;
  while (end > start && /\s/.test(raw[end - 1])) end -= 1;
  return {
    leading: raw.slice(0, start),
    trimmed: raw.slice(start, end),
    trailing: raw.slice(end),
  };
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

function parseAlpha(token: string): { css: string; value: number } | null {
  const percent = parseBoundedPercent(token);
  if (percent !== null) return { css: `${percent}%`, value: percent / 100 };
  const number = parseBoundedNumber(token, 0, 1);
  if (number !== null) return { css: `${number}`, value: number };
  return null;
}

function parseAngle(token: string): string | null {
  const match = ANGLE_RE.exec(token);
  if (!match) return null;
  const value = Number(match[1]);
  if (!Number.isFinite(value)) return null;
  return `${value}${match[2] ? match[2].toLowerCase() : 'deg'}`;
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

function parseColor(value: string): Extract<RecognizedText, { kind: 'color' }> | null {
  if (value.length < 4 || value.length > 96 || /[\r\n]/.test(value)) return null;
  const hexMatch = HEX_RE.exec(value);
  if (hexMatch) {
    const hex = hexMatch[1];
    // Avoid 4-digit numeric references such as #2026 or #8080 unless hex letters appear.
    if (hex.length === 4 && /^[0-9]{4}$/.test(hex)) return null;
    const lower = hex.toLowerCase();
    const hasAlpha = lower.length === 4 ? lower[3] !== 'f' : lower.length === 8 ? lower.slice(6, 8) !== 'ff' : false;
    return { kind: 'color', cssColor: `#${lower}`, hasAlpha };
  }

  const fnMatch = /^(rgba?|hsla?|oklch)\(\s*([^()]+)\s*\)$/i.exec(value);
  if (!fnMatch) return null;
  const fn = fnMatch[1].toLowerCase();
  const parsedArgs = splitColorArgs(fnMatch[2]);
  if (!parsedArgs) return null;
  const { channels, alpha: alphaToken } = parsedArgs;
  const alpha = alphaToken !== null ? parseAlpha(alphaToken) : null;
  if (alphaToken !== null && !alpha) return null;
  const hasAlpha = alpha !== null && alpha.value < 1;

  if (fn === 'rgb' || fn === 'rgba') {
    const nums = channels.map((token) => parseBoundedNumber(token, 0, 255));
    if (nums.every((n) => n !== null)) {
      const [r, g, b] = nums;
      const cssColor = alpha ? `rgb(${r} ${g} ${b} / ${alpha.css})` : `rgb(${r} ${g} ${b})`;
      return { kind: 'color', cssColor, hasAlpha };
    }
    const pcts = channels.map((token) => parseBoundedPercent(token));
    if (pcts.every((n) => n !== null)) {
      const [r, g, b] = pcts;
      const cssColor = alpha ? `rgb(${r}% ${g}% ${b}% / ${alpha.css})` : `rgb(${r}% ${g}% ${b}%)`;
      return { kind: 'color', cssColor, hasAlpha };
    }
    return null;
  }

  if (fn === 'hsl' || fn === 'hsla') {
    const hue = parseAngle(channels[0]);
    const sat = parseBoundedPercent(channels[1]);
    const light = parseBoundedPercent(channels[2]);
    if (!hue || sat === null || light === null) return null;
    const cssColor = alpha ? `hsl(${hue} ${sat}% ${light}% / ${alpha.css})` : `hsl(${hue} ${sat}% ${light}%)`;
    return { kind: 'color', cssColor, hasAlpha };
  }

  if (fn === 'oklch') {
    if (fnMatch[2].includes(',')) return null;
    const lPct = parseBoundedPercent(channels[0]);
    const lNum = lPct === null ? parseBoundedNumber(channels[0], 0, 1) : null;
    const lCss = lPct !== null ? `${lPct}%` : lNum !== null ? `${lNum}` : null;
    const cPct = parseBoundedPercent(channels[1]);
    const cNum = cPct === null ? parseBoundedNumber(channels[1], 0, 1) : null;
    const cCss = cPct !== null ? `${cPct}%` : cNum !== null ? `${cNum}` : null;
    const hue = parseAngle(channels[2]);
    if (!lCss || !cCss || !hue) return null;
    const cssColor = alpha ? `oklch(${lCss} ${cCss} ${hue} / ${alpha.css})` : `oklch(${lCss} ${cCss} ${hue})`;
    return { kind: 'color', cssColor, hasAlpha };
  }

  return null;
}

function parseLinkOrEmail(value: string): Extract<RecognizedText, { kind: 'link' | 'email' }> | null {
  if (!value || value.length > 4096 || CONTROL_RE.test(value) || /\s/.test(value) || value.includes('\\')) return null;

  const lower = value.toLowerCase();
  const hasScheme = lower.startsWith('http://') || lower.startsWith('https://');
  const hasWww = !hasScheme && lower.startsWith('www.');
  if (hasScheme || hasWww) {
    const schemeEnd = hasScheme ? value.indexOf('://') + 3 : 0;
    if (hasScheme && value[schemeEnd] === '/') return null;
    const candidate = hasWww ? `https://${value}` : value;
    try {
      const url = new URL(candidate);
      if (!url.hostname || url.username || url.password) return null;
      if (hasWww && !url.hostname.slice(4).includes('.')) return null;
      const prefix = value.slice(0, schemeEnd);
      const afterScheme = value.slice(schemeEnd);
      let authorityEnd = afterScheme.length;
      for (let i = 0; i < afterScheme.length; i += 1) {
        const ch = afterScheme[i];
        if (ch === '/' || ch === '?' || ch === '#') {
          authorityEnd = i;
          break;
        }
      }
      const host = afterScheme.slice(0, authorityEnd);
      if (!host) return null;
      return { kind: 'link', prefix, host, rest: afterScheme.slice(authorityEnd) };
    } catch {
      return null;
    }
  }

  const at = value.indexOf('@');
  if (at <= 0 || value.indexOf('@', at + 1) !== -1) return null;
  const local = value.slice(0, at);
  const domain = value.slice(at + 1);
  if (local.length > 64 || domain.length > 253 || !domain.includes('.')) return null;
  if (!local.split('.').every((part) => part.length > 0 && EMAIL_LOCAL_PART_RE.test(part))) return null;
  if (!domain.split('.').every((label) => label.length > 0 && label.length <= 63 && !label.startsWith('-') && !label.endsWith('-') && EMAIL_DOMAIN_LABEL_RE.test(label))) return null;
  return { kind: 'email', local, domain };
}

function isJsonText(value: string, sample: string): boolean {
  const first = sample[0];
  if (first !== '{' && first !== '[') return false;
  if (value.length <= 4096) {
    const last = value[value.length - 1];
    if ((first === '{' && last === '}') || (first === '[' && last === ']')) {
      try {
        const parsed: unknown = JSON.parse(value);
        if (Array.isArray(parsed)) {
          return parsed.length >= 2 || (parsed.length === 1 && (typeof parsed[0] === 'object' && parsed[0] !== null || typeof parsed[0] === 'string'));
        }
        return typeof parsed === 'object' && parsed !== null;
      } catch {
        // Fall through to truncated-preview check below when backend summary truncated long JSON.
      }
    }
  }
  return /^\{\s*"[^"\r\n]+"\s*:/.test(sample) || /^\[\s*\{\s*"[^"\r\n]+"\s*:/.test(sample);
}

function isCodeText(value: string): boolean {
  if (value.length < 3) return false;
  const sample = value.length > 2048 ? value.slice(0, 2048) : value;
  if (isJsonText(value, sample)) return true;
  if (HTML_START_RE.test(sample) && (/<\/[A-Za-z]/.test(sample) || /\/>/.test(sample) || /^<!DOCTYPE\b|^<\?xml\b/i.test(sample) || /\s[A-Za-z_:][\w:.-]*\s*=\s*["']/.test(sample))) return true;
  if (CLI_PATTERNS.some((pattern) => pattern.test(sample))) return true;
  if (SQL_RE.test(sample)) return true;
  if (CODE_START_RE.test(sample)) return true;
  if (!sample.includes('\n') && SINGLE_LINE_CSS_RE.test(sample)) return true;

  const lines = sample.split('\n').filter((line) => line.trim().length > 0);
  if (lines.length < 2) return false;

  let signals = 0;
  const indented = lines.filter((line) => /^(?:  +|\t)\S/.test(line)).length;
  if (indented >= 1 && (lines.some((line) => /[{}();]$/.test(line.trim())) || indented >= 2 && lines.some((line) => /:\s*\S/.test(line)))) signals += 1;
  if (lines.some((line) => /\{\s*$/.test(line)) && lines.some((line) => /^\s*\}[;,]?\s*$/.test(line))) signals += 1;
  if (lines.filter((line) => /;\s*$/.test(line)).length >= 2) signals += 1;
  if (/(?:=>|->|::|===|!==|\&\&|\|\|)/.test(sample)) signals += 1;
  if (lines.some((line) => /^\s*(?:\/\/|\/\*|\*\/)/.test(line))) signals += 1;
  if (/\b[A-Za-z_$]\w*\([^)\n]*\)\s*(?:\{|;|\.)/.test(sample)) signals += 1;
  return signals >= 2;
}

export function recognizeClipboardText(raw: string): TextRecognitionResult {
  const { leading, trimmed, trailing } = trimBounds(raw);
  if (!trimmed) return { leading, trimmed, trailing, match: { kind: 'plain' } };
  const color = parseColor(trimmed);
  if (color) return { leading, trimmed, trailing, match: color };
  const target = parseLinkOrEmail(trimmed);
  if (target) return { leading, trimmed, trailing, match: target };
  if (isCodeText(trimmed)) return { leading, trimmed, trailing, match: { kind: 'code' } };
  return { leading, trimmed, trailing, match: { kind: 'plain' } };
}
