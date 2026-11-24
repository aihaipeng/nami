import { describe, expect, it } from 'vitest';
import { encodeQrCode, MAX_QR_UTF8_BYTES } from './qrEncoder';
import { analyzeSmartDetail, canEncodeQr, formatRelativeTime, isOpenableQrLink } from './smartDetail';

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
});
