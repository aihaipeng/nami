import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ActivityBeam } from './ActivityBeam';
import { initialSnapshot } from './contracts';
import { messages, translator } from './i18n';
import { panelHorizontalPosition } from './FloatingPanel';

it('keeps dropdowns inside the visible native window within the larger fixed canvas', () => {
  expect(panelHorizontalPosition(441, 360, 441, 920)).toEqual({ left: 449, maxWidth: 360 });
  expect(panelHorizontalPosition(1300, 310, 441, 920)).toEqual({ left: 1043, maxWidth: 310 });
  expect(panelHorizontalPosition(0, 360, 0, 1361)).toEqual({ left: 8, maxWidth: 360 });
  expect(panelHorizontalPosition(250, 360, 0, 280)).toEqual({ left: 8, maxWidth: 264 });
});

describe('English presentation and nonverbal activity', () => {
  it('starts in English and retains translations for both languages', () => {
    expect(initialSnapshot.locale).toBe('en');
    expect(translator(initialSnapshot.locale)('translation')).toBe('Translate');
    for (const pair of Object.values(messages)) {
      expect(pair[0].length).toBeGreaterThan(0);
      expect(pair[1].length).toBeGreaterThan(0);
    }
  });

  it('names activity for assistive technology without visible loading text', () => {
    const markup = renderToStaticMarkup(<ActivityBeam label="Working" />);
    expect(markup).toContain('role="progressbar"');
    expect(markup).toContain('aria-label="Working"');
    expect(markup).not.toContain('>Working<');
    expect(markup).not.toContain('aria-valuenow');
  });
});

describe('language choices and provider capabilities', () => {
  it('retains the existing labels and accent-insensitive language search', async () => {
    const { languages, matchesLanguage } = await import('./languages');
    expect(languages).toHaveLength(17);
    expect(languages.map(([code]) => code)).toEqual(['ar', 'zh-CN', 'zh-TW', 'nl', 'en', 'fr', 'de', 'id', 'it', 'ja', 'ko', 'pl', 'pt', 'ru', 'es', 'tr', 'vi']);
    expect(matchesLanguage('zh-CN', 'Chinese (Simplified)', 'ZH-cn')).toBe(true);
    expect(matchesLanguage('fr', 'French', '  french  ')).toBe(true);
    expect(matchesLanguage('pt', 'Portuguese (Brazil)', 'BRAZIL')).toBe(true);
    expect(matchesLanguage('fr', 'French', 'German')).toBe(false);
  });
});

describe('provider-specific language catalogs', () => {
  it('keeps each provider capability separate and does not send display summaries as language codes', async () => {
    const { providerLanguages, supportsLanguage, supportsTranslation, languageName } = await import('./languages');
    expect(providerLanguages('google', 'target').some(({ code }) => code === 'eo')).toBe(true);
    expect(providerLanguages('edge', 'target').some(({ code }) => code === 'eo')).toBe(false);
    expect(providerLanguages('edge', 'source').some(({ code }) => code === 'lzh')).toBe(true);
    expect(providerLanguages('google', 'source').some(({ code }) => code === 'lzh')).toBe(false);
    expect(supportsLanguage('google', 'source', 'zh-TW')).toBe(true);
    expect(supportsTranslation('google', 'auto', ['en', 'eo'])).toBe(true);
    expect(supportsTranslation('edge', 'auto', ['en', 'eo'])).toBe(false);
    for (const engine of ['google', 'edge'] as const) {
      expect(supportsLanguage(engine, 'source', 'auto')).toBe(true);
      for (const code of ['auto', 'mul', 'und', 'not-a-language']) expect(supportsLanguage(engine, 'target', code)).toBe(false);
    }
    expect(languageName('mul')).toBe('Multiple languages');
    expect(languageName('und')).toBe('Unknown language');
    expect(languageName('unknown-provider-code')).toBe('unknown-provider-code');
    expect(languageName('pt')).toBe('Portuguese (Brazil)');
  });
});
