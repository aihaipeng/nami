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

describe('FinderX language choices', () => {
  it('keeps the same shared provider set and alphabetical English names', async () => {
    const { languages, matchesLanguage } = await import('./languages');
    expect(languages).toHaveLength(17);
    expect(languages.map(([code]) => code)).toEqual(['ar', 'zh-CN', 'zh-TW', 'nl', 'en', 'fr', 'de', 'id', 'it', 'ja', 'ko', 'pl', 'pt', 'ru', 'es', 'tr', 'vi']);
    expect(matchesLanguage('zh-CN', 'Chinese (Simplified)', 'ZH-cn')).toBe(true);
    expect(matchesLanguage('fr', 'French', '  french  ')).toBe(true);
    expect(matchesLanguage('pt', 'Portuguese (Brazil)', 'BRAZIL')).toBe(true);
    expect(matchesLanguage('fr', 'French', 'German')).toBe(false);
  });
});
