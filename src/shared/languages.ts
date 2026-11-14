export const languages = [
  ['ar', 'Arabic'], ['zh-CN', 'Chinese (Simplified)'], ['zh-TW', 'Chinese (Traditional)'],
  ['nl', 'Dutch'], ['en', 'English'], ['fr', 'French'], ['de', 'German'],
  ['id', 'Indonesian'], ['it', 'Italian'], ['ja', 'Japanese'], ['ko', 'Korean'],
  ['pl', 'Polish'], ['pt', 'Portuguese (Brazil)'], ['ru', 'Russian'],
  ['es', 'Spanish'], ['tr', 'Turkish'], ['vi', 'Vietnamese'],
] as const;

export function matchesLanguage(code: string, name: string, query: string) {
  const normalize = (value: string) => value.normalize('NFKD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  const term = normalize(query.trim());
  return normalize(name).includes(term) || normalize(code).includes(term);
}
