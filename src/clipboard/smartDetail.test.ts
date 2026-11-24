import { describe, expect, it } from 'vitest';
import { encodeQrCode, MAX_QR_UTF8_BYTES } from './qrEncoder';
import {
  analyzeSmartDetail,
  canEncodeQr,
  computeNextCronRuns,
  describeCron,
  formatCronRunTime,
  formatRelativeTime,
  isOpenableQrLink,
} from './smartDetail';

describe('smartDetail', () => {
  it('parses and converts hex, rgb, hsl, and oklch colors including alpha', () => {
    const hex = analyzeSmartDetail('  #0ea5e9  ');
    expect(hex).toMatchObject({ kind: 'color', hex: '#0EA5E9', rgb: 'rgb(14, 165, 233)', hsl: 'hsl(199, 89%, 48%)', hasAlpha: false });

    const shortAlpha = analyzeSmartDetail('#f088');
    expect(shortAlpha).toMatchObject({ kind: 'color', hex: '#FF008888', hasAlpha: true });
    expect(shortAlpha && shortAlpha.kind === 'color' ? shortAlpha.rgb : '').toContain('rgba(255, 0, 136,');

    const rgb = analyzeSmartDetail('rgb(255, 0, 0)');
    expect(rgb).toMatchObject({ kind: 'color', hex: '#FF0000', rgb: 'rgb(255, 0, 0)', hsl: 'hsl(0, 100%, 50%)' });

    const hsl = analyzeSmartDetail('hsl(120deg 100% 50%)');
    expect(hsl).toMatchObject({ kind: 'color', hex: '#00FF00', rgb: 'rgb(0, 255, 0)' });

    const oklch = analyzeSmartDetail('oklch(0.65 0.2 250)');
    expect(oklch?.kind).toBe('color');

    expect(analyzeSmartDetail('#2026')).toBeNull();
  });

  it('parses valid JSON objects and arrays while skipping invalid or oversized input', () => {
    const json = analyzeSmartDetail('{"b":2,"a":[1,true]}');
    expect(json).toEqual({
      kind: 'json',
      pretty: '{\n  "b": 2,\n  "a": [\n    1,\n    true\n  ]\n}',
      minified: '{"b":2,"a":[1,true]}',
      alreadyPretty: false,
    });
    expect(analyzeSmartDetail('{"broken":')).toBeNull();
    expect(analyzeSmartDetail('"just a string"')).toBeNull();
    expect(analyzeSmartDetail(`{"big":"${'a'.repeat(200_005)}"}`)).toBeNull();
  });

  it('recognizes Unix timestamps in seconds and milliseconds without matching ordinary numbers', () => {
    const sec = analyzeSmartDetail('1700000000');
    expect(sec?.kind).toBe('timestamp');
    if (sec?.kind === 'timestamp') {
      expect(sec.unit).toBe('seconds');
      expect(sec.iso).toBe('2023-11-14T22:13:20.000Z');
      expect(sec.local).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
    }

    const ms = analyzeSmartDetail('1700000000500');
    expect(ms?.kind).toBe('timestamp');
    if (ms?.kind === 'timestamp') {
      expect(ms.unit).toBe('milliseconds');
      expect(ms.iso).toBe('2023-11-14T22:13:20.500Z');
    }

    for (const nonTimestamp of ['2026', '8080', '123456789', '9999999999', '13800138000']) {
      expect(analyzeSmartDetail(nonTimestamp)).toBeNull();
    }
  });

  it('encodes valid QR matrices locally across versions and enforces byte bounds', () => {
    const short = encodeQrCode('https://example.test');
    expect(short).not.toBeNull();
    expect(short!.size).toBe(short!.version * 4 + 17);
    expect(short!.modules).toHaveLength(short!.size * short!.size);
    // Top-left finder corners and dark module at (size - 8, 8).
    expect(short!.modules[0]).toBe(true);
    expect(short!.modules[(short!.size - 8) * short!.size + 8]).toBe(true);

    const longUrl = `https://example.test/${'a'.repeat(160)}`;
    const higher = encodeQrCode(longUrl);
    expect(higher).not.toBeNull();
    expect(higher!.version).toBeGreaterThanOrEqual(7);
    // Alignment patterns centered on timing lines (e.g. row 6 or col 6) must still be drawn.
    const mid = Math.floor((higher!.size - 1) / 2);
    expect(higher!.modules[6 * higher!.size + mid]).toBe(true);
    expect(higher!.modules[mid * higher!.size + 6]).toBe(true);

    expect(canEncodeQr('你好 Nami')).toBe(true);
    expect(encodeQrCode('你好 Nami')).not.toBeNull();
    expect(canEncodeQr('x'.repeat(MAX_QR_UTF8_BYTES + 1))).toBe(false);
    expect(encodeQrCode('x'.repeat(MAX_QR_UTF8_BYTES + 1))).toBeNull();
  });

  it('restricts openable QR links to safe web URLs only', () => {
    expect(isOpenableQrLink('https://example.test/path?q=1')).toBe(true);
    expect(isOpenableQrLink('www.example.test/docs')).toBe(true);
    expect(isOpenableQrLink('http://localhost:8080')).toBe(true);
    for (const unsafe of ['javascript:alert(1)', 'file:///etc/passwd', 'mailto:a@b.test', 'https://user@example.test', 'plain text']) {
      expect(isOpenableQrLink(unsafe)).toBe(false);
    }
  });

  it('parses URLs with query params, percent-encoding, and tracking parameters while skipping bare URLs', () => {
    const tracked = analyzeSmartDetail('https://example.test/video?id=42&utm_source=share&fbclid=abc&si=xyz&tab=info');
    expect(tracked).toEqual({
      kind: 'url',
      cleanUrl: 'https://example.test/video?id=42&tab=info',
      removedTrackingCount: 3,
      decodedUrl: null,
      params: [
        { key: 'id', value: '42' },
        { key: 'utm_source', value: 'share' },
        { key: 'fbclid', value: 'abc' },
        { key: 'si', value: 'xyz' },
        { key: 'tab', value: 'info' },
      ],
      totalParams: 5,
    });

    const onlyTracking = analyzeSmartDetail('https://example.test/post?utm_campaign=launch&spm=1.2');
    expect(onlyTracking).toMatchObject({
      kind: 'url',
      cleanUrl: 'https://example.test/post',
      removedTrackingCount: 2,
    });

    const encoded = analyzeSmartDetail('https://example.test/wiki/%E4%B8%AD%E6%96%87?q=%E6%B5%8B%E8%AF%95');
    expect(encoded).toMatchObject({
      kind: 'url',
      cleanUrl: null,
      decodedUrl: 'https://example.test/wiki/中文?q=测试',
      params: [{ key: 'q', value: '测试' }],
      totalParams: 1,
    });

    const manyParams = Array.from({ length: 30 }, (_, i) => `k${i}=v${i}`).join('&');
    const capped = analyzeSmartDetail(`https://example.test/api?${manyParams}`);
    expect(capped?.kind).toBe('url');
    if (capped?.kind === 'url') {
      expect(capped.params).toHaveLength(24);
      expect(capped.totalParams).toBe(30);
    }

    expect(analyzeSmartDetail('https://example.test/docs')).toBeNull();
    expect(analyzeSmartDetail('https://user:pass@example.test/?q=1')).toBeNull();
  });

  it('parses date-time strings into timestamps and formats localized relative times', () => {
    const expectedSec = String(Date.UTC(2026, 10, 25, 0, 22, 55) / 1000);
    const expectedMs = String(Date.UTC(2026, 10, 25, 0, 22, 55));

    const isoUtc = analyzeSmartDetail('2026-11-25T00:22:55Z');
    expect(isoUtc).toMatchObject({
      kind: 'datetime',
      unixSeconds: expectedSec,
      unixMilliseconds: expectedMs,
      showLocal: true,
      showIso: false,
    });

    const isoOffset = analyzeSmartDetail('2026-11-25T08:22:55+08:00');
    expect(isoOffset).toMatchObject({
      kind: 'datetime',
      unixSeconds: expectedSec,
      iso: '2026-11-25T00:22:55.000Z',
      showIso: true,
    });

    const slashLocal = analyzeSmartDetail('2026/11/25 08:30:00');
    expect(slashLocal?.kind).toBe('datetime');

    const dateOnly = analyzeSmartDetail('2026-11-25');
    expect(dateOnly?.kind).toBe('datetime');

    const rfc = analyzeSmartDetail('Wed, 25 Nov 2026 00:22:55 GMT');
    expect(rfc).toMatchObject({
      kind: 'datetime',
      unixSeconds: expectedSec,
      iso: '2026-11-25T00:22:55.000Z',
    });

    for (const invalid of ['2026-02-30', '2026-13-01', '2026-11-25 25:00:00', '2026-11-25T00:00:00+25:00']) {
      expect(analyzeSmartDetail(invalid)).toBeNull();
    }

    const base = Date.UTC(2026, 10, 25, 0, 22, 55);
    expect(formatRelativeTime(base - 2_000, 'en', base)).toBe('just now');
    expect(formatRelativeTime(base - 2_000, 'zh-CN', base)).toBe('刚刚');
    expect(formatRelativeTime(base - 3_600_000 * 2, 'en', base)).toMatch(/2 hours ago/);
    expect(formatRelativeTime(base + 86_400_000 * 3, 'zh-CN', base)).toMatch(/3天后/);
  });

  it('decodes valid JWTs locally and rejects malformed or ordinary dotted strings', () => {
    const toBase64Url = (obj: object) => {
      const bytes = new TextEncoder().encode(JSON.stringify(obj));
      let bin = '';
      for (const byte of bytes) bin += String.fromCharCode(byte);
      return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
    };
    const token = `${toBase64Url({ alg: 'HS256', typ: 'JWT' })}.${toBase64Url({ sub: 'u_1', name: '林海', iat: 1700000000, exp: 1800000000 })}.sig_part_here`;
    const jwt = analyzeSmartDetail(token);
    expect(jwt?.kind).toBe('jwt');
    if (jwt?.kind === 'jwt') {
      expect(jwt.algorithm).toBe('HS256');
      expect(jwt.issuedAtMs).toBe(1_700_000_000_000);
      expect(jwt.expiresAtMs).toBe(1_800_000_000_000);
      expect(jwt.payloadPretty).toContain('"name": "林海"');
      expect(jwt.headerPretty).toContain('"alg": "HS256"');
    }

    expect(analyzeSmartDetail('not.a.jwt')).toBeNull();
    expect(analyzeSmartDetail(`${toBase64Url({ foo: 'bar' })}.${toBase64Url({ x: 1 })}.sig`)).toBeNull();
  });

  it('decodes escaped strings, percent-encoded fragments, and Base64 text without false positives', () => {
    const unicode = analyzeSmartDetail('\\u4f60\\u597d\\u4e16\\u754c');
    expect(unicode).toEqual({ kind: 'decoded', encoding: 'escape', text: '你好世界', jsonPretty: null });

    const quotedJson = analyzeSmartDetail('"{\\"ok\\":true,\\n\\"n\\":1}"');
    expect(quotedJson?.kind).toBe('decoded');
    if (quotedJson?.kind === 'decoded') {
      expect(quotedJson.encoding).toBe('escape');
      expect(quotedJson.text).toBe('{"ok":true,\n"n":1}');
      expect(quotedJson.jsonPretty).toBe('{\n  "ok": true,\n  "n": 1\n}');
    }

    const logEscape = analyzeSmartDetail('line1\\nline2\\t\\"ok\\"');
    expect(logEscape).toMatchObject({ kind: 'decoded', encoding: 'escape', text: 'line1\nline2\t"ok"' });

    for (const nonEscape of ['C:\\Users\\new\\test', '\\\\server\\share\\file', '/^\\d+\\s+\\w+$/', 'foo\\\\bar\\\\baz']) {
      expect(analyzeSmartDetail(nonEscape)).toBeNull();
    }

    const pct = analyzeSmartDetail('%E4%B8%AD%E6%96%87%20Nami');
    expect(pct).toEqual({ kind: 'decoded', encoding: 'url', text: '中文 Nami', jsonPretty: null });
    expect(analyzeSmartDetail('50% off, 20% discount')).toBeNull();

    const b64 = analyzeSmartDetail('SGVsbG8gV29ybGQ=');
    expect(b64).toEqual({ kind: 'decoded', encoding: 'base64', text: 'Hello World', jsonPretty: null });

    const b64Json = analyzeSmartDetail('eyJ1c2VyIjoibmFtaSIsInJvbGUiOiJhZG1pbiJ9');
    expect(b64Json).toEqual({
      kind: 'decoded',
      encoding: 'base64',
      text: '{"user":"nami","role":"admin"}',
      jsonPretty: '{\n  "user": "nami",\n  "role": "admin"\n}',
    });

    for (const nonB64 of [
      'ClipboardSmartDetail',
      'a1b2c3d4e5f60718293a4b5c6d7e8f90',
      'my-feature-branch-name',
      'hello_world_test_case',
      '/////wAAAQID',
    ]) {
      expect(analyzeSmartDetail(nonB64)).toBeNull();
    }
  });

  it('recognizes inline SVG and data:image/* URIs safely while rejecting non-image data URIs', () => {
    const rawSvg = '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/></svg>';
    const svg = analyzeSmartDetail(rawSvg);
    expect(svg?.kind).toBe('imageData');
    if (svg?.kind === 'imageData') {
      expect(svg.sourceKind).toBe('svg');
      expect(svg.format).toBe('SVG');
      expect(svg.src.startsWith('data:image/svg+xml;charset=utf-8,')).toBe(true);
      expect(decodeURIComponent(svg.src)).toContain('xmlns="http://www.w3.org/2000/svg"');
      expect(svg.estimatedBytes).toBe(new TextEncoder().encode(rawSvg).length);
    }

    const withProlog = '<?xml version="1.0"?><!-- icon --><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16"/></svg>';
    const svgWithXmlns = analyzeSmartDetail(withProlog);
    expect(svgWithXmlns?.kind).toBe('imageData');
    if (svgWithXmlns?.kind === 'imageData') {
      const decoded = decodeURIComponent(svgWithXmlns.src);
      expect(decoded.match(/xmlns=/g)).toHaveLength(1);
    }

    const pngUri = 'data:image/png;base64,iVBORw0KGgo=\n';
    const png = analyzeSmartDetail(pngUri);
    expect(png).toEqual({
      kind: 'imageData',
      sourceKind: 'data-uri',
      format: 'PNG',
      src: 'data:image/png;base64,iVBORw0KGgo=',
      estimatedBytes: 8,
    });

    const encodedSvgUri = 'data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%2010%2010%22%3E%3Crect%20width%3D%2210%22%20height%3D%2210%22%2F%3E%3C%2Fsvg%3E';
    const svgUri = analyzeSmartDetail(encodedSvgUri);
    expect(svgUri?.kind).toBe('imageData');
    if (svgUri?.kind === 'imageData') {
      expect(svgUri.format).toBe('SVG');
      expect(decodeURIComponent(svgUri.src)).toContain('xmlns="http://www.w3.org/2000/svg"');
    }

    for (const nonImage of [
      'data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==',
      'data:image/png,not-base64',
      '<div class="svg">not an svg</div>',
      '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/>',
    ]) {
      expect(analyzeSmartDetail(nonImage)).toBeNull();
    }
  });

  it('parses strict 5-field Cron expressions, describes them bilingually, and computes future runs accurately', () => {
    const cron = analyzeSmartDetail('*/15 9-18 * * 1-5');
    expect(cron?.kind).toBe('cron');
    if (cron?.kind === 'cron') {
      expect(describeCron(cron, 'zh-CN')).toBe('工作日 09:00–18:59 每 15 分钟');
      expect(describeCron(cron, 'en')).toBe('Every 15 minutes, 09:00–18:59, Mon–Fri');

      // Wednesday 2026-11-25 08:50 local -> 09:00, 09:15, 09:30
      const wedMorning = new Date(2026, 10, 25, 8, 50, 0).getTime();
      expect(computeNextCronRuns(cron, wedMorning, 3).map(formatCronRunTime)).toEqual([
        '2026-11-25 09:00',
        '2026-11-25 09:15',
        '2026-11-25 09:30',
      ]);

      // Friday 2026-11-27 18:50 local -> skips weekend to Monday 2026-11-30 09:00
      const friEvening = new Date(2026, 10, 27, 18, 50, 0).getTime();
      expect(computeNextCronRuns(cron, friEvening, 2).map(formatCronRunTime)).toEqual([
        '2026-11-30 09:00',
        '2026-11-30 09:15',
      ]);
    }

    const daily = analyzeSmartDetail('30 8 * * *');
    expect(daily?.kind).toBe('cron');
    if (daily?.kind === 'cron') {
      expect(describeCron(daily, 'zh-CN')).toBe('每天 08:30');
      expect(describeCron(daily, 'en')).toBe('Every day at 08:30');
    }

    const leapDay = analyzeSmartDetail('0 0 29 2 *');
    expect(leapDay?.kind).toBe('cron');
    if (leapDay?.kind === 'cron') {
      const base = new Date(2026, 0, 1, 0, 0, 0).getTime();
      expect(computeNextCronRuns(leapDay, base, 1).map(formatCronRunTime)).toEqual(['2028-02-29 00:00']);
    }

    const aliased = analyzeSmartDetail('0 10 * JAN-MAR FRI-SUN');
    expect(aliased?.kind).toBe('cron');
    if (aliased?.kind === 'cron') {
      expect(aliased.dow.values).toEqual([0, 5, 6]);
      expect(describeCron(aliased, 'zh-CN')).toContain('1–3 月');
      expect(describeCron(aliased, 'en')).toContain('Jan–Mar');
    }

    for (const nonCron of [
      '1 2 3 4 5',
      '60 * * * *',
      '0 24 * * *',
      '0 0 31 2 *',
      '*/0 * * * *',
      '0 9 * * * *',
      'hello world * * *',
    ]) {
      expect(analyzeSmartDetail(nonCron)).toBeNull();
    }
  });

  it('evaluates safe arithmetic expressions without eval and avoids dates, IDs, and overflows', () => {
    expect(analyzeSmartDetail('(1920 * 1080) / 1024')).toEqual({
      kind: 'math',
      result: '2025',
      formatted: '2,025',
    });
    expect(analyzeSmartDetail('3980 * 0.85')).toEqual({
      kind: 'math',
      result: '3383',
      formatted: '3,383',
    });
    expect(analyzeSmartDetail('0.1 + 0.2')).toEqual({
      kind: 'math',
      result: '0.3',
      formatted: null,
    });
    expect(analyzeSmartDetail('-(3 + 4) * 2 + 100 % 7')).toEqual({
      kind: 'math',
      result: '-12',
      formatted: null,
    });
    expect(analyzeSmartDetail('2 ** 10')).toEqual({
      kind: 'math',
      result: '1024',
      formatted: '1,024',
    });
    expect(analyzeSmartDetail('1920 × 1080')).toEqual({
      kind: 'math',
      result: '2073600',
      formatted: '2,073,600',
    });
    // Multiplication with 3 operands must be recognized as math, not 5-field Cron.
    expect(analyzeSmartDetail('2 * 3 * 4')).toEqual({
      kind: 'math',
      result: '24',
      formatted: null,
    });

    for (const nonMath of [
      '42',
      '(123)',
      '10 / 0',
      '10 % 0',
      '2026-11',
      '11-25',
      '400-800-1234',
      '01-02',
      '2026/11/25',
      '11/25/2026',
      '9 ** 9 ** 9',
      'alert(1) + 2',
      `${'('.repeat(40)}1+1${')'.repeat(40)}`,
    ]) {
      expect(analyzeSmartDetail(nonMath)).toBeNull();
    }
  });

  it('converts radix literals and Unix octal permissions cleanly', () => {
    expect(analyzeSmartDetail('0xFF')).toEqual({
      kind: 'number',
      dec: '255',
      hex: '0xFF',
      bin: '0b11111111',
      oct: '0o377',
      permissions: null,
      preferPermissionsFirst: false,
    });
    expect(analyzeSmartDetail('0b1010')).toEqual({
      kind: 'number',
      dec: '10',
      hex: '0xA',
      bin: '0b1010',
      oct: '0o12',
      permissions: null,
      preferPermissionsFirst: false,
    });
    expect(analyzeSmartDetail('0o755')).toEqual({
      kind: 'number',
      dec: '493',
      hex: '0x1ED',
      bin: '0b111101101',
      oct: '0o755',
      permissions: 'rwxr-xr-x',
      preferPermissionsFirst: false,
    });
    expect(analyzeSmartDetail('0755')).toEqual({
      kind: 'number',
      dec: '493',
      hex: '0x1ED',
      bin: '0b111101101',
      oct: '0o755',
      permissions: 'rwxr-xr-x',
      preferPermissionsFirst: true,
    });
    expect(analyzeSmartDetail('chmod 755')).toMatchObject({
      kind: 'number',
      oct: '0o755',
      permissions: 'rwxr-xr-x',
      preferPermissionsFirst: true,
    });
    expect(analyzeSmartDetail('sudo chmod -R 0644 ./keys')).toMatchObject({
      kind: 'number',
      oct: '0o644',
      permissions: 'rw-r--r--',
      preferPermissionsFirst: true,
    });
    expect(analyzeSmartDetail('0o4755')).toMatchObject({ kind: 'number', permissions: 'rwsr-xr-x' });
    expect(analyzeSmartDetail('0o2755')).toMatchObject({ kind: 'number', permissions: 'rwxr-sr-x' });
    expect(analyzeSmartDetail('0o1777')).toMatchObject({ kind: 'number', permissions: 'rwxrwxrwt' });
    expect(analyzeSmartDetail('0o4644')).toMatchObject({ kind: 'number', permissions: 'rwSr--r--' });
    expect(analyzeSmartDetail('0xFFFFFFFFFFFFFFFF')).toMatchObject({
      kind: 'number',
      dec: '18446744073709551615',
    });

    for (const nonNumber of ['0xZZ', '0b102', '0o89', '0888', 'chmod 999', 'chmod +x script.sh', '1234']) {
      expect(analyzeSmartDetail(nonNumber)).toBeNull();
    }
  });

  it('decodes genuine escaped payloads while rejecting shell commands, code snippets, prose, paths, and LaTeX', () => {
    const positiveCases: Array<{ label: string; input: string; expectedText: string; expectJson?: boolean }> = [
      { label: 'pure unicode', input: '\\u4f60\\u597d\\u4e16\\u754c', expectedText: '你好世界' },
      { label: 'single unicode', input: '\\u4f60', expectedText: '你' },
      { label: 'es6 codepoint', input: '\\u{1F600}\\u{1F680}', expectedText: '😀🚀' },
      { label: 'mixed ascii + unicode', input: 'Hello \\u4e16\\u754c!', expectedText: 'Hello 世界!' },
      { label: 'multiline java properties unicode', input: 'app.title=\\u6d6a\\u6f6e\napp.subtitle=\\u526a\\u8d34\\u677f', expectedText: 'app.title=浪潮\napp.subtitle=剪贴板' },
      { label: 'quoted JSON string containing JSON', input: '"{\\"ok\\":true,\\n\\"n\\":1}"', expectedText: '{"ok":true,\n"n":1}', expectJson: true },
      { label: 'quoted JSON string with quotes and newline', input: '"He said \\"hello\\"\\nLine 2"', expectedText: 'He said "hello"\nLine 2' },
      { label: 'quoted JSON string with unicode', input: '"\\u4f60\\u597d\\n\\u4e16\\u754c"', expectedText: '你好\n世界' },
      { label: 'quoted JSON Windows path', input: '"C:\\\\Users\\\\Alice\\\\notes.txt"', expectedText: 'C:\\Users\\Alice\\notes.txt' },
      { label: 'unquoted escaped JSON object', input: '{\\"user\\":\\"nami\\",\\"role\\":\\"admin\\"}', expectedText: '{"user":"nami","role":"admin"}', expectJson: true },
      { label: 'unquoted escaped JSON array', input: '[{\\"id\\":1},{\\"id\\":2}]', expectedText: '[{"id":1},{"id":2}]', expectJson: true },
      { label: 'unquoted escaped log with n/t/quote', input: 'line1\\nline2\\t\\"ok\\"', expectedText: 'line1\nline2\t"ok"' },
      { label: 'unquoted single-line stack trace', input: 'Error: boom\\n    at foo (a.ts:10)\\n    at bar (b.ts:20)', expectedText: 'Error: boom\n    at foo (a.ts:10)\n    at bar (b.ts:20)' },
      { label: 'unquoted CRLF log', input: '2026-11-25 INFO start\\r\\n2026-11-25 INFO done', expectedText: '2026-11-25 INFO start\r\n2026-11-25 INFO done' },
      { label: 'unquoted prose with apostrophes and newlines', input: "Don't panic\\nIt's working\\nAll good", expectedText: "Don't panic\nIt's working\nAll good" },
      { label: 'unquoted escaped SQL', input: 'SELECT *\\nFROM users\\nWHERE active = 1', expectedText: 'SELECT *\nFROM users\nWHERE active = 1' },
      { label: 'unquoted escaped HTML fragment', input: '<p class=\\"title\\">Hello</p>\\n<p>World</p>', expectedText: '<p class="title">Hello</p>\n<p>World</p>' },
      { label: 'unquoted key-value escaped quotes', input: 'title=\\"Nami\\" status=\\"ok\\"', expectedText: 'title="Nami" status="ok"' },
    ];

    for (const item of positiveCases) {
      const result = analyzeSmartDetail(item.input);
      expect(result, `expected positive escape match for: ${item.label}`).toMatchObject({
        kind: 'decoded',
        encoding: 'escape',
        text: item.expectedText,
      });
      if (item.expectJson && result?.kind === 'decoded') {
        expect(result.jsonPretty).not.toBeNull();
      }
    }

    const negativeCases: Array<{ label: string; input: string }> = [
      {
        label: 'single-line osascript command with escaped HTML quotes',
        input: 'osascript -l JavaScript -e \'ObjC.import("AppKit"); const pb = $.NSPasteboard.generalPasteboard; pb.clearContents; pb.setStringForType("Nami HTML 富文本验证", "public.utf8-plain-text"); pb.setStringForType("<p><strong>Nami</strong> <em>HTML 富文本</em>验证 — <a href=\\"https://example.com\\">示例链接</a></p>", "public.html");\'',
      },
      {
        label: 'multi-line zsh script with escaped quotes and RTF',
        input: [
          '#!/bin/zsh',
          'set -e',
          'echo "Writing 4 exact text types..."',
          'osascript -l JavaScript -e \'item.setStringForType("<a href=\\"https://example.com\\">link</a>", "public.html");\'',
          'osascript -l JavaScript -e \'item.setStringForType("{\\\\rtf1\\\\ansi {\\\\b Nami} Sample}", "public.rtf");\'',
          'echo "Done!"',
        ].join('\n'),
      },
      { label: 'curl command with escaped JSON payload', input: 'curl -X POST https://api.example.com/v1 -H "Content-Type: application/json" -d "{\\"name\\":\\"nami\\"}"' },
      { label: 'JS console.log with escaped quote and newline', input: 'console.log("Hello \\"world\\"\\nDone\\n");' },
      { label: 'TS const with double-quoted escapes', input: 'const msg = "line1\\nline2\\tvalue";' },
      { label: 'TS const with single-quoted escapes', input: "const msg = 'line1\\nline2\\tvalue';" },
      { label: 'shell printf double-quoted', input: 'printf "line1\\nline2\\n"' },
      { label: 'shell printf single-quoted', input: "printf 'line1\\nline2\\n'" },
      { label: 'shell echo -e single-quoted', input: "echo -e 'a\\nb\\tc'" },
      { label: 'python3 -c command', input: 'python3 -c \'print("a\\nb\\tc")\'' },
      { label: 'git commit with escaped quotes', input: 'git commit -m "Fix \\"quote\\" and \\n handling"' },
      { label: 'awk command with tab and newline', input: 'awk \'{print $1 "\\t" $2 "\\n"}\'' },
      { label: 'function call with single-quoted literal', input: "notify('Step 1\\nStep 2\\nDone');" },
      { label: 'standalone single-quoted literal', input: "'hello\\nworld\\n'" },
      { label: 'multi-line TypeScript function with escapes', input: 'function greet() {\n  const s = "a\\nb\\n\\"c\\"";\n  return s;\n}' },
      { label: 'multi-line JS file with unicode constants', input: 'const NBSP = "\\u00a0";\nconst ZWSP = "\\u200b";\nexport const clean = (s: string) => s.replace(NBSP, " ");' },
      { label: 'single-line JS const with single unicode escape (single-quoted)', input: "const NBSP = '\\u00a0';" },
      { label: 'single-line JS const with single unicode escape (double-quoted)', input: 'const NBSP = "\\u00a0";' },
      { label: 'multi-line shebang script with unicode escapes', input: '#!/bin/zsh\necho \\u4f60\\u597d' },
      { label: 'multi-line markdown mentioning escapes', input: 'Formatting rules:\nUse \\n for line break\nUse \\t for tab' },
      { label: 'single-line prose mentioning \\n and \\t', input: 'Use \\n for newline and \\t for tab.' },
      { label: 'single-line prose mentioning \\r\\n and \\n', input: 'Convert \\r\\n to \\n before saving.' },
      { label: 'Windows path with standard escape letters', input: 'C:\\new\\temp\\file.txt' },
      { label: 'embedded Windows path with standard escape letters', input: 'path=C:\\new\\temp\\file.txt' },
      { label: 'UNC path', input: '\\\\server\\share\\file' },
      { label: 'regex literal', input: '/^\\d+\\s+\\w+$/' },
      { label: 'plain double backslashes', input: 'foo\\\\bar\\\\baz' },
      { label: 'LaTeX frac and text', input: '\\frac{1}{2} + \\text{hello} + \\text{world}' },
      { label: 'raw RTF source', input: '{\\rtf1\\ansi {\\b Nami} \\tab ok}' },
      { label: 'single weak escape only', input: 'foo\\nbar' },
      { label: 'single quote escape plus backslash', input: 'a \\" b \\\\ c' },
      { label: 'non-printable control unicode escape', input: '\\u0007' },
    ];

    for (const item of negativeCases) {
      const result = analyzeSmartDetail(item.input);
      expect(result?.kind === 'decoded' && result.encoding === 'escape', `unexpected escape match for: ${item.label}`).toBe(false);
    }

    // Valid JSON containing escapes stays classified as JSON, not Unescaped.
    expect(analyzeSmartDetail('{"msg":"line1\\nline2"}')?.kind).toBe('json');
  });
});
