import { describe, expect, it } from 'vitest';
import { clipboardSamples } from './input';
import { resolveLanguage } from './i18n';

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
