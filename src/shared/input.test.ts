import { describe, expect, it } from 'vitest';
import { clipboardSamples, inputWithinLimits } from './input';
import { resolveLanguage } from './i18n';

describe('translation input limits', () => {
  it('counts UTF-16 units, accepts the boundary, and never truncates input', () => {
    expect(inputWithinLimits('🌊'.repeat(2_500))).toBe(true);
    expect(inputWithinLimits('🌊'.repeat(2_501))).toBe(false);
    expect(inputWithinLimits(Array(20).fill('a'.repeat(4_999)).join('\n'))).toBe(true);
    expect(inputWithinLimits(Array(21).fill('a'.repeat(4_999)).join('\n'))).toBe(false);
  });

  it('handles CRLF as one line boundary and includes the trailing blank line', () => {
    expect(inputWithinLimits('a\r\n'.repeat(1_999))).toBe(true);
    expect(inputWithinLimits('a\r\n'.repeat(2_000))).toBe(false);
  });
});

describe('P0 synthetic samples and locale', () => {
  it('preserves multiline and Unicode content', () => {
    expect(clipboardSamples[1].text).toBe('第一行 / First line\n\n第三行 / Third line\n');
    expect(clipboardSamples[2].text).toContain('🌊 — 100%_literal');
  });

  it('follows Simplified Chinese systems and falls back to English', () => {
    expect(resolveLanguage('system', 'zh-Hans-CN')).toBe('zh-CN');
    expect(resolveLanguage('system', 'zh-TW')).toBe('en');
    expect(resolveLanguage('system', 'fr-FR')).toBe('en');
    expect(resolveLanguage('en', 'zh-CN')).toBe('en');
  });
});
