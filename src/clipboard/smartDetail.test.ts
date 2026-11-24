import { describe, expect, it } from 'vitest';
import { encodeQrCode, MAX_QR_UTF8_BYTES } from './qrEncoder';
import { analyzeSmartDetail, canEncodeQr, isOpenableQrLink } from './smartDetail';

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
});
