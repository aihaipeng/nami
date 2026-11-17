export const TRANSLATION_STRATEGY_VERSION = 1;

export interface TranslationMetadata {
  requestedSource: string;
  engine: string;
  strategyVersion: number;
  detections: { rawLanguage: string; language: string; score: number | null }[];
}

export interface HistoryRecord {
  id: string;
  text: string;
  sourceLanguage: string;
  translation?: TranslationMetadata;
  targetLanguages: string[];
  translations: string[];
  date: string;
  isFavorite: boolean;
  isPinned: boolean;
  sourceApp?: { bundleId: string; name: string; iconDataUrl: string | null } | null;
}

export interface HistorySnapshot {
  revision: number;
  records: HistoryRecord[];
  selectedId: string | null;
  totalRecords: number;
  hasMore: boolean;
  generation: number;
}

export interface HistorySelection {
  revision: number;
  sampleId: string | null;
  retranslate: boolean;
  clearDetail?: boolean;
  record?: HistoryRecord | null;
}

export function requestedSource(record: HistoryRecord) {
  return record.translation?.requestedSource ?? record.sourceLanguage;
}

export const historySamples: HistoryRecord[] = [];

export function filterHistory(records: HistoryRecord[], query: string, favoritesOnly: boolean) {
  const needle = query.trim().toLocaleLowerCase();
  return records.filter((record) => (!favoritesOnly || record.isFavorite)
    && [record.text, ...record.translations].some((text) => text.toLocaleLowerCase().includes(needle)))
    .sort((a, b) => Number(b.isPinned) - Number(a.isPinned) || Date.parse(b.date) - Date.parse(a.date) || b.id.localeCompare(a.id));
}

export function historyPreview(text: string) {
  return [...text.replace(/\r\n/g, '\n').trim()].slice(0, 256).join('');
}

export function historyScrollTarget(scrollTop: number, viewport: { top: number; bottom: number }, row: { top: number; bottom: number }) {
  if (row.top < viewport.top) return Math.max(0, scrollTop + row.top - viewport.top);
  if (row.bottom > viewport.bottom) return Math.max(0, scrollTop + row.bottom - viewport.bottom);
  return null;
}
