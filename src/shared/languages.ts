import google from './languages/google.json';
import edge from './languages/edge.json';
import apple from './languages/apple.json';

const catalog = { google, edge, apple };

export const languages = [
  ['ar', 'Arabic'], ['zh-CN', 'Chinese (Simplified)'], ['zh-TW', 'Chinese (Traditional)'],
  ['nl', 'Dutch'], ['en', 'English'], ['fr', 'French'], ['de', 'German'],
  ['id', 'Indonesian'], ['it', 'Italian'], ['ja', 'Japanese'], ['ko', 'Korean'],
  ['pl', 'Polish'], ['pt', 'Portuguese (Brazil)'], ['ru', 'Russian'],
  ['es', 'Spanish'], ['tr', 'Turkish'], ['vi', 'Vietnamese'],
] as const;

const names = new Map<string, string>([...catalog.apple.map(({ code, name }) => [code, name] as const), ...catalog.edge.map(({ code, name }) => [code, name] as const), ...catalog.google.map(({ code, name }) => [code, name] as const), ...languages]);

export type TranslationEngine = keyof typeof catalog;
export type LanguageSide = 'source' | 'target';

export function providerLanguages(engine: TranslationEngine, side: LanguageSide) {
  return catalog[engine].filter((language) => language[side]).map(({ code, name }) => ({ code, name: engine === 'apple' ? name : languageName(code) }));
}

export function languageName(code: string) {
  if (code === 'auto') return 'Auto-Detect';
  if (code === 'mul') return 'Multiple languages';
  if (code === 'und') return 'Unknown language';
  return names.get(code) ?? code;
}

export function supportsLanguage(engine: TranslationEngine, side: LanguageSide, code: string) {
  return side === 'source' && code === 'auto' || catalog[engine].some((language) => language.code === code && language[side]);
}

export function supportsTranslation(engine: TranslationEngine, source: string, targets: string[]) {
  return supportsLanguage(engine, 'source', source) && targets.every((target) => supportsLanguage(engine, 'target', target));
}

export function matchesLanguage(code: string, name: string, query: string) {
  const normalize = (value: string) => value.normalize('NFKD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  const term = normalize(query.trim());
  return normalize(name).includes(term) || normalize(code).includes(term);
}
