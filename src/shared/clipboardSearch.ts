export type ClipboardSearchMode = 'exact' | 'fuzzy' | 'regex' | 'mixed';
export type ClipboardSearchOptions = { mode: ClipboardSearchMode; body: boolean; note: boolean; imageText: boolean };

export const defaultClipboardSearch: ClipboardSearchOptions = { mode: 'exact', body: true, note: true, imageText: true };
