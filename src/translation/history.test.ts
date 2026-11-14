import historySamples from '../../tests/fixtures/translation-history.json';
import { expect, it } from 'vitest';
import { filterHistory, historyPreview, historySamples as initialHistory, historyScrollTarget } from './history';

it('starts without preloaded translation history', () => {
  expect(initialHistory).toEqual([]);
});

it('provides 20 distinct synthetic records with complete cached translations', () => {
  expect(historySamples).toHaveLength(20);
  expect(new Set(historySamples.map(({ id }) => id)).size).toBe(20);
  expect(historySamples.filter(({ isPinned }) => isPinned)).toHaveLength(2);
  expect(historySamples.filter(({ isFavorite }) => isFavorite)).toHaveLength(5);
  for (const record of historySamples) {
    expect(record.translations).toHaveLength(record.targetLanguages.length);
    expect(record.text.length).toBeGreaterThan(0);
    expect(record.translations.every(Boolean)).toBe(true);
    expect(Number.isFinite(Date.parse(record.date))).toBe(true);
  }
});

it('searches source and cached translations literally, preserving pin and time order', () => {
  expect(filterHistory(historySamples, '100%_literal', false).map(({ id }) => id)).toEqual(['sample-18']);
  expect(filterHistory(historySamples, '安静', false).map(({ id }) => id)).toEqual(['sample-07']);
  expect(filterHistory(historySamples, 'BONJOUR', false).map(({ id }) => id)).toEqual(['sample-01']);
  expect(filterHistory(historySamples, '', true)).toHaveLength(5);
  expect(filterHistory([...historySamples].reverse(), '', false).map(({ id }) => id)).toEqual(historySamples.map(({ id }) => id));
  expect(filterHistory(historySamples, 'not present', false)).toEqual([]);
});

it('normalizes and truncates the preview by Unicode scalars, keeping the full source intact', () => {
  const text = ` \r\n${'🌊'.repeat(300)}\r\n `;
  expect([...historyPreview(text)]).toHaveLength(256);
  expect(historyPreview(' first\r\nsecond\n ')).toBe('first\nsecond');
  expect(text).toContain('🌊'.repeat(300));
});

it('reveals only clipped records in their own scroll region without moving visible rows', () => {
  const viewport = { top: 180, bottom: 580 };
  expect(historyScrollTarget(100, viewport, { top: 150, bottom: 230 })).toBe(70);
  expect(historyScrollTarget(100, viewport, { top: 530, bottom: 610 })).toBe(130);
  expect(historyScrollTarget(100, viewport, { top: 190, bottom: 270 })).toBeNull();
  expect(historyScrollTarget(0, viewport, { top: 178, bottom: 258 })).toBe(0);
});
