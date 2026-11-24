export interface CronField {
  raw: string;
  wildcard: boolean;
  values: number[];
  valueSet: Set<number>;
}

export interface SmartCron {
  kind: 'cron';
  expression: string;
  minute: CronField;
  hour: CronField;
  dom: CronField;
  month: CronField;
  dow: CronField;
}

const MONTH_ALIASES: Record<string, number> = {
  jan: 1, feb: 2, mar: 3, apr: 4, may: 5, jun: 6,
  jul: 7, aug: 8, sep: 9, oct: 10, nov: 11, dec: 12,
};

const DOW_ALIASES: Record<string, number> = {
  sun: 0, mon: 1, tue: 2, wed: 3, thu: 4, fri: 5, sat: 6,
};

const MAX_DAYS_IN_MONTH = [0, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

const ZH_DOW = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];
const EN_DOW_FULL = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const EN_DOW_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const EN_MONTHS = ['', 'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

function parseFieldAtom(token: string, min: number, max: number, aliases?: Record<string, number>): number | null {
  if (!token) return null;
  if (aliases) {
    const alias = aliases[token.toLowerCase()];
    if (alias !== undefined) return alias;
  }
  if (!/^\d{1,2}$/.test(token)) return null;
  const num = Number(token);
  return num >= min && num <= max ? num : null;
}

function parseCronField(
  raw: string,
  min: number,
  max: number,
  aliases?: Record<string, number>,
  normalizeSevenToZero = false,
): CronField | null {
  if (!raw || raw.length > 48) return null;
  const wildcard = raw === '*';
  const segments = raw.split(',');
  if (segments.length > 24 || segments.some((seg) => !seg)) return null;

  const upperDefault = normalizeSevenToZero ? 6 : max;
  const maxStep = upperDefault - min + 1;
  const set = new Set<number>();

  for (const segment of segments) {
    const slashParts = segment.split('/');
    if (slashParts.length > 2) return null;
    const base = slashParts[0];
    let step = 1;
    if (slashParts.length === 2) {
      if (!/^[1-9]\d{0,1}$/.test(slashParts[1])) return null;
      step = Number(slashParts[1]);
      if (step < 1 || step > maxStep) return null;
    }

    let start: number;
    let end: number;
    if (base === '*') {
      start = min;
      end = upperDefault;
    } else if (base.includes('-')) {
      const rangeParts = base.split('-');
      if (rangeParts.length !== 2) return null;
      const a = parseFieldAtom(rangeParts[0], min, max, aliases);
      let b = parseFieldAtom(rangeParts[1], min, max, aliases);
      if (a === null || b === null) return null;
      if (normalizeSevenToZero && b === 0 && a > 0 && rangeParts[1].toLowerCase() === 'sun') {
        b = 7;
      }
      if (a > b) return null;
      start = a;
      end = b;
    } else {
      const value = parseFieldAtom(base, min, max, aliases);
      if (value === null) return null;
      start = value;
      end = slashParts.length === 2 ? upperDefault : value;
      if (start > end) {
        if (normalizeSevenToZero && start === 7 && slashParts.length === 2) {
          start = 0;
        } else {
          return null;
        }
      }
    }

    for (let current = start; current <= end; current += step) {
      set.add(normalizeSevenToZero && current === 7 ? 0 : current);
    }
  }

  if (set.size === 0) return null;
  const values = Array.from(set).sort((a, b) => a - b);
  return { raw, wildcard, values, valueSet: set };
}

function hasReachableCalendarDay(dom: CronField, month: CronField, dow: CronField): boolean {
  if (dom.wildcard || !dow.wildcard) return true;
  for (const m of month.values) {
    const maxDay = MAX_DAYS_IN_MONTH[m];
    if (dom.values.some((d) => d <= maxDay)) return true;
  }
  return false;
}

export function parseSmartCron(trimmed: string): SmartCron | null {
  if (trimmed.length < 9 || trimmed.length > 120 || /[\r\n]/.test(trimmed)) return null;
  const fields = trimmed.split(/\s+/);
  if (fields.length !== 5) return null;

  // Reject 5 plain integers (e.g. "1 2 3 4 5" table rows) with no cron syntax or aliases.
  if (!fields.some((field) => /[*\/,-]|[A-Za-z]/.test(field))) return null;
  // Reject multiplication expressions such as "2 * 3 * 4".
  if (fields[1] === '*' && fields[3] === '*' && /^\d+$/.test(fields[0]) && /^\d+$/.test(fields[2]) && /^\d+$/.test(fields[4])) {
    return null;
  }

  const minute = parseCronField(fields[0], 0, 59);
  const hour = parseCronField(fields[1], 0, 23);
  const dom = parseCronField(fields[2], 1, 31);
  const month = parseCronField(fields[3], 1, 12, MONTH_ALIASES);
  const dow = parseCronField(fields[4], 0, 7, DOW_ALIASES, true);
  if (!minute || !hour || !dom || !month || !dow) return null;
  if (!hasReachableCalendarDay(dom, month, dow)) return null;

  return {
    kind: 'cron',
    expression: fields.join(' '),
    minute,
    hour,
    dom,
    month,
    dow,
  };
}

function matchesCronDay(cron: SmartCron, date: Date): boolean {
  if (cron.dom.wildcard && cron.dow.wildcard) return true;
  const domMatch = cron.dom.valueSet.has(date.getDate());
  const dowMatch = cron.dow.valueSet.has(date.getDay());
  if (cron.dom.wildcard) return dowMatch;
  if (cron.dow.wildcard) return domMatch;
  return domMatch || dowMatch;
}

export function computeNextCronRuns(cron: SmartCron, nowMs = Date.now(), count = 3): Date[] {
  const results: Date[] = [];
  let cursor = new Date(nowMs);
  cursor.setSeconds(0, 0);
  cursor = new Date(cursor.getTime() + 60_000);
  const maxYear = cursor.getFullYear() + 8;

  for (let steps = 0; steps < 20_000 && results.length < count; steps += 1) {
    if (cursor.getFullYear() > maxYear) break;
    if (!cron.month.valueSet.has(cursor.getMonth() + 1)) {
      cursor = new Date(cursor.getFullYear(), cursor.getMonth() + 1, 1, 0, 0, 0, 0);
      continue;
    }
    if (!matchesCronDay(cron, cursor)) {
      cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate() + 1, 0, 0, 0, 0);
      continue;
    }
    if (!cron.hour.valueSet.has(cursor.getHours())) {
      cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate(), cursor.getHours() + 1, 0, 0, 0);
      continue;
    }
    if (!cron.minute.valueSet.has(cursor.getMinutes())) {
      cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate(), cursor.getHours(), cursor.getMinutes() + 1, 0, 0);
      continue;
    }
    results.push(new Date(cursor.getTime()));
    cursor = new Date(cursor.getTime() + 60_000);
  }
  return results;
}

export function formatCronRunTime(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function pad2(value: number): string {
  return String(value).padStart(2, '0');
}

function parseStepOnly(raw: string): number | null {
  const match = /^\*\/([1-9]\d*)$/.exec(raw);
  return match ? Number(match[1]) : null;
}

function parseSimpleRange(raw: string, aliases?: Record<string, number>): [number, number] | null {
  const parts = raw.split('-');
  if (parts.length !== 2 || raw.includes(',') || raw.includes('/')) return null;
  const a = parseFieldAtom(parts[0], 0, 59, aliases);
  const b = parseFieldAtom(parts[1], 0, 59, aliases);
  if (a === null || b === null) return null;
  return [a, b];
}

function describeDow(dow: CronField, locale: 'zh-CN' | 'en'): string {
  const vals = dow.values;
  if (vals.length === 5 && vals[0] === 1 && vals[4] === 5) {
    return locale === 'zh-CN' ? '工作日' : 'Mon–Fri';
  }
  if (vals.length === 2 && vals[0] === 0 && vals[1] === 6) {
    return locale === 'zh-CN' ? '周末' : 'Weekends';
  }
  if (vals.length === 1) {
    return locale === 'zh-CN' ? `每${ZH_DOW[vals[0]]}` : `Every ${EN_DOW_FULL[vals[0]]}`;
  }
  const range = parseSimpleRange(dow.raw, DOW_ALIASES);
  if (range) {
    const start = range[0] === 7 ? 0 : range[0];
    const end = range[1] === 7 ? 0 : range[1];
    return locale === 'zh-CN' ? `${ZH_DOW[start]}至${ZH_DOW[end]}` : `${EN_DOW_SHORT[start]}–${EN_DOW_SHORT[end]}`;
  }
  return locale === 'zh-CN'
    ? vals.map((d) => ZH_DOW[d]).join('、')
    : vals.map((d) => EN_DOW_SHORT[d]).join(', ');
}

function describeMonth(month: CronField, locale: 'zh-CN' | 'en'): string {
  const step = parseStepOnly(month.raw);
  if (step !== null) return locale === 'zh-CN' ? `每 ${step} 个月` : `every ${step} months`;
  if (month.values.length === 1) {
    return locale === 'zh-CN' ? `${month.values[0]} 月` : EN_MONTHS[month.values[0]];
  }
  const range = parseSimpleRange(month.raw, MONTH_ALIASES);
  if (range) {
    return locale === 'zh-CN' ? `${range[0]}–${range[1]} 月` : `${EN_MONTHS[range[0]]}–${EN_MONTHS[range[1]]}`;
  }
  return locale === 'zh-CN'
    ? `${month.values.join('、')} 月`
    : month.values.map((m) => EN_MONTHS[m]).join(', ');
}

function describeDom(dom: CronField, locale: 'zh-CN' | 'en'): string {
  const step = parseStepOnly(dom.raw);
  if (step !== null) return locale === 'zh-CN' ? `每 ${step} 天` : `every ${step} days`;
  if (dom.values.length === 1) {
    return locale === 'zh-CN' ? `${dom.values[0]} 日` : `day ${dom.values[0]}`;
  }
  const range = parseSimpleRange(dom.raw);
  if (range) {
    return locale === 'zh-CN' ? `${range[0]}–${range[1]} 日` : `days ${range[0]}–${range[1]}`;
  }
  return locale === 'zh-CN'
    ? `${dom.values.join('、')} 日`
    : `days ${dom.values.join(', ')}`;
}

function describeTime(minute: CronField, hour: CronField, locale: 'zh-CN' | 'en'): { text: string; needsDailyPrefix: boolean } {
  const minStep = parseStepOnly(minute.raw);
  const hourStep = parseStepOnly(hour.raw);
  const hourRange = parseSimpleRange(hour.raw);

  if (minute.wildcard && hour.wildcard) {
    return { text: locale === 'zh-CN' ? '每分钟' : 'Every minute', needsDailyPrefix: false };
  }
  if (minStep !== null && hour.wildcard) {
    return {
      text: locale === 'zh-CN' ? `每 ${minStep} 分钟` : (minStep === 1 ? 'Every minute' : `Every ${minStep} minutes`),
      needsDailyPrefix: false,
    };
  }
  if (minStep !== null && hourRange) {
    const span = `${pad2(hourRange[0])}:00–${pad2(hourRange[1])}:59`;
    return {
      text: locale === 'zh-CN' ? `${span} 每 ${minStep} 分钟` : `Every ${minStep} minutes, ${span}`,
      needsDailyPrefix: false,
    };
  }
  if (minute.values.length === 1 && hour.wildcard) {
    const m = minute.values[0];
    if (m === 0) return { text: locale === 'zh-CN' ? '每小时整点' : 'Every hour', needsDailyPrefix: false };
    return {
      text: locale === 'zh-CN' ? `每小时第 ${m} 分` : `At minute ${m} of every hour`,
      needsDailyPrefix: false,
    };
  }
  if (minute.values.length === 1 && hourStep !== null) {
    const m = minute.values[0];
    if (m === 0) {
      return {
        text: locale === 'zh-CN' ? `每 ${hourStep} 小时` : (hourStep === 1 ? 'Every hour' : `Every ${hourStep} hours`),
        needsDailyPrefix: false,
      };
    }
    return {
      text: locale === 'zh-CN' ? `每 ${hourStep} 小时第 ${m} 分` : `At minute ${m}, every ${hourStep} hours`,
      needsDailyPrefix: false,
    };
  }
  if (minute.values.length === 1 && hour.values.length === 1) {
    const hhmm = `${pad2(hour.values[0])}:${pad2(minute.values[0])}`;
    return { text: locale === 'zh-CN' ? hhmm : `at ${hhmm}`, needsDailyPrefix: true };
  }
  if (minute.values.length === 1 && hour.values.length <= 4 && !hour.raw.includes('-') && !hour.raw.includes('/')) {
    const times = hour.values.map((h) => `${pad2(h)}:${pad2(minute.values[0])}`);
    return {
      text: locale === 'zh-CN' ? times.join('、') : `at ${times.join(', ')}`,
      needsDailyPrefix: true,
    };
  }
  const minPart = minute.wildcard
    ? (locale === 'zh-CN' ? '每分钟' : 'every minute')
    : minStep !== null
      ? (locale === 'zh-CN' ? `每 ${minStep} 分钟` : `every ${minStep} minutes`)
      : (locale === 'zh-CN' ? `第 ${minute.raw} 分` : `minute ${minute.raw}`);
  const hourPart = hour.wildcard
    ? ''
    : hourRange
      ? `${pad2(hourRange[0])}:00–${pad2(hourRange[1])}:59`
      : hourStep !== null
        ? (locale === 'zh-CN' ? `每 ${hourStep} 小时` : `every ${hourStep} hours`)
        : (locale === 'zh-CN' ? `${hour.raw} 时` : `hour ${hour.raw}`);
  if (locale === 'zh-CN') {
    return { text: hourPart ? `${hourPart} ${minPart}` : minPart, needsDailyPrefix: false };
  }
  const combined = hourPart ? `${minPart}, ${hourPart}` : minPart;
  return { text: combined.charAt(0).toUpperCase() + combined.slice(1), needsDailyPrefix: false };
}

export function describeCron(cron: SmartCron, locale: 'zh-CN' | 'en'): string {
  const time = describeTime(cron.minute, cron.hour, locale);
  const allDays = cron.dom.wildcard && cron.month.wildcard && cron.dow.wildcard;

  if (allDays) {
    if (!time.needsDailyPrefix) return time.text;
    return locale === 'zh-CN' ? `每天 ${time.text}` : `Every day ${time.text}`;
  }

  if (locale === 'zh-CN') {
    const parts: string[] = [];
    if (!cron.month.wildcard) parts.push(describeMonth(cron.month, 'zh-CN'));
    if (!cron.dom.wildcard && !cron.dow.wildcard) {
      parts.push(`每月 ${describeDom(cron.dom, 'zh-CN')}或${describeDow(cron.dow, 'zh-CN')}`);
    } else if (!cron.dom.wildcard) {
      const domText = describeDom(cron.dom, 'zh-CN');
      parts.push(cron.month.wildcard && !domText.startsWith('每') ? `每月 ${domText}` : domText);
    } else if (!cron.dow.wildcard) {
      parts.push(describeDow(cron.dow, 'zh-CN'));
    }
    parts.push(time.text);
    return parts.join(' ');
  }

  const enParts: string[] = [];
  const timePhrase = time.needsDailyPrefix ? `At ${time.text.slice(3)}` : time.text;
  enParts.push(timePhrase);

  if (!cron.dom.wildcard && !cron.dow.wildcard) {
    enParts.push(`on ${describeDom(cron.dom, 'en')} or ${describeDow(cron.dow, 'en')}`);
  } else if (!cron.dom.wildcard) {
    const domText = describeDom(cron.dom, 'en');
    enParts.push(domText.startsWith('every') ? domText : `on ${domText}`);
  } else if (!cron.dow.wildcard) {
    enParts.push(describeDow(cron.dow, 'en'));
  }

  if (!cron.month.wildcard) {
    const monText = describeMonth(cron.month, 'en');
    enParts.push(monText.startsWith('every') ? monText : `in ${monText}`);
  }

  return enParts.join(', ');
}
