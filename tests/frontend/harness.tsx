import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, vi } from 'vitest';
import type { HistorySnapshot } from '../../src/translation/history';
import { initialSnapshot, type AppSnapshot, type TranslationPreferences, type WindowState } from '../../src/shared/contracts';
import { defaultTranslationShortcuts, normalizeBinding, shortcutError, type ShortcutAction, type TranslationShortcuts } from '../../src/shared/shortcuts';

const bridge = vi.hoisted(() => ({
  label: 'translation',
  snapshot: undefined as unknown as AppSnapshot,
  history: undefined as unknown as HistorySnapshot,
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  invoke: vi.fn(),
  listen: vi.fn(),
  moved: vi.fn(),
}));

export { bridge };

vi.mock('../../src/translation/history', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../src/translation/history')>(),
  historySamples: (await import('../fixtures/translation-history.json')).default,
}));

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: bridge.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: bridge.listen }));
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ label: bridge.label }) }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'translation', onMoved: bridge.moved }) }));

import App from '../../src/App';
import { filterHistory, historySamples } from '../../src/translation/history';

const frames = new Map<number, FrameRequestCallback>();
let nextFrame = 0;
let historyQuery = '';
let historyFavorites = false;
let historyLoaded = 100;
function historySnapshot(): HistorySnapshot {
  const matching = filterHistory(bridge.history.records, historyQuery, historyFavorites);
  return structuredClone({ ...bridge.history, records: matching.slice(0, historyLoaded), totalRecords: matching.length, hasMore: matching.length > historyLoaded });
}
export const scrollTo = vi.fn(function (this: HTMLElement, first?: number | ScrollToOptions, second?: number) {
  this.scrollTop = typeof first === 'number' ? second ?? 0 : first?.top ?? this.scrollTop;
});
export const scrollIntoView = vi.fn();
export const setPointerCapture = vi.fn();
export const releasePointerCapture = vi.fn();
export let reducedMotion = false;

beforeAll(() => {
  window.scrollTo = vi.fn();
  Object.defineProperty(document, 'fonts', { configurable: true, value: { ready: Promise.resolve() } });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({
    font: '', measureText: (text: string) => ({ width: text.length * 6 }),
  }) as unknown as CanvasRenderingContext2D);
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: reducedMotion && query.includes('prefers-reduced-motion'), media: query, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++nextFrame, callback); return nextFrame; });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
  if (!globalThis.PointerEvent) vi.stubGlobal('PointerEvent', class extends MouseEvent {
    readonly pointerId: number;
    constructor(type: string, options: PointerEventInit = {}) { super(type, options); this.pointerId = options.pointerId ?? 0; }
  });
  HTMLElement.prototype.scrollTo = scrollTo;
  HTMLElement.prototype.scrollIntoView = scrollIntoView;
  HTMLElement.prototype.setPointerCapture = setPointerCapture;
  HTMLElement.prototype.releasePointerCapture = releasePointerCapture;
});

beforeEach(() => {
  cleanup();
  bridge.label = 'translation';
  bridge.snapshot = structuredClone(initialSnapshot);
  bridge.snapshot.capabilities = { platform: 'macos', nativeHistory: true, nativeChrome: true, nativeMaterial: true, selection: true, imageOcr: true, clipboardWrite: true, autoPaste: true, accessibilityPermission: true, primaryModifier: 'meta' };
  bridge.snapshot.translationEngine = 'edge';
  bridge.snapshot.translationPreferences = { engine: 'edge', sourceLanguage: 'auto', primaryLanguage: 'en', secondaryLanguage: 'zh-CN', proxyUrl: '', proxyEnabled: false, selectionEnabled: true, shortcuts: defaultTranslationShortcuts('meta') };
  bridge.history = { revision: 0, records: structuredClone(historySamples), selectedId: null, totalRecords: historySamples.length, hasMore: false, generation: 0 };
  historyQuery = ''; historyFavorites = false; historyLoaded = 100;
  bridge.snapshot.window.recordView = 'translation';
  bridge.snapshot.window.historyVisible = true;
  bridge.snapshot.window.historyExtent = bridge.snapshot.window.historyWidth;
  bridge.listeners.clear(); frames.clear(); reducedMotion = false;
  bridge.invoke.mockReset().mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'selection_take') return null;
    if (name === 'translation_cancel') {
      bridge.snapshot.window.recordsBrowsing = false;
      deliver('window_state', structuredClone(bridge.snapshot.window));
    }
    if (name === 'translation_run') {
      const request = args as { text: string; source: string; targets: string[]; reuse?: { recordId: string; targetIndex?: number } };
      const previous = bridge.history.records.find(({ id }) => id === request.reuse?.recordId);
      const count = bridge.invoke.mock.calls.filter(([name]) => name === 'translation_run').length;
      const record = { ...historySamples[0], id: count === 1 ? 'translation-test' : `translation-test-${count}`, text: request.text, sourceLanguage: request.source, translation: { requestedSource: request.source, engine: bridge.snapshot.translationEngine, strategyVersion: 1, detections: [] }, targetLanguages: request.targets, translations: request.targets.map((target, index) => request.reuse?.targetIndex !== undefined && index !== request.reuse.targetIndex && previous?.text === request.text && previous.sourceLanguage === request.source && previous.targetLanguages[index] === target ? previous.translations[index] : `Synthetic translation ${index + 1}`), isFavorite: false, isPinned: false, sourceApp: previous?.text === request.text ? previous.sourceApp : undefined };
      bridge.history.records = [...bridge.history.records.filter(({ id }) => id !== record.id), record];
      if (!bridge.snapshot.window.recordsBrowsing) bridge.history.selectedId = record.id;
      ++bridge.history.revision;
      deliver('history_changed', historySnapshot());
      return record;
    }
    if (name === 'history_snapshot') return historySnapshot();
    if (name === 'history_query') {
      const { query, favoritesOnly } = args as { query: string; favoritesOnly: boolean };
      historyQuery = query; historyFavorites = favoritesOnly; historyLoaded = 100;
      ++bridge.history.generation; ++bridge.history.revision;
      const snapshot = historySnapshot(); deliver('history_changed', snapshot); return snapshot;
    }
    if (name === 'history_load_more') {
      historyLoaded += 100; ++bridge.history.revision;
      const snapshot = historySnapshot(); deliver('history_changed', snapshot); return snapshot;
    }
    if (name === 'history_record_update') {
      const { recordId, action } = args as { recordId: string; action: string };
      const record = bridge.history.records.find(({ id }) => id === recordId);
      if (!record) throw new Error('not_found');
      if (action === 'delete') {
        if (record.isFavorite || record.isPinned) throw new Error('history_protected');
        bridge.history.records = bridge.history.records.filter(({ id }) => id !== recordId);
        if (bridge.history.selectedId === recordId) bridge.history.selectedId = null;
      } else if (action === 'favorite') record.isFavorite = !record.isFavorite;
      else if (action === 'pin') record.isPinned = !record.isPinned;
      else throw new Error('invalid_argument');
      ++bridge.history.revision; deliver('history_changed', historySnapshot());
      return historySnapshot();
    }
    if (name === 'records_view_set') {
      const { view } = args as { view: 'clipboard' | 'translation' };
      const keep = bridge.snapshot.window.historyVisible && bridge.snapshot.window.translationVisible;
      bridge.snapshot.window.recordView = view;
      bridge.snapshot.window.historyVisible = true;
      bridge.snapshot.window.historyExtent = bridge.snapshot.window.historyWidth;
      if (!keep) {
        bridge.snapshot.window.translationVisible = false; bridge.snapshot.window.translationExtent = 0; bridge.snapshot.window.clipboardDetailId = null;
      }
      deliver('window_state', structuredClone(bridge.snapshot.window));
    }
    if (name === 'clipboard_details_select') {
      const browsing = (args as { browsing?: boolean }).browsing;
      if (browsing !== undefined) bridge.snapshot.window.recordsBrowsing = browsing;
      else if (bridge.snapshot.window.recordsBrowsing === false) return;
      bridge.snapshot.window.clipboardDetailId = (args as { recordId: string | null }).recordId;
      deliver('window_state', structuredClone(bridge.snapshot.window));
    }
    if (name === 'records_editor_set' && (args as { open?: boolean }).open !== undefined) { bridge.snapshot.window.recordsEditorOpen = (args as { open: boolean }).open; deliver('window_state', structuredClone(bridge.snapshot.window)); }
    if (name === 'clipboard_note_open') deliver('clipboard_note_requested', (args as { recordId: string }).recordId);
    if (name === 'records_details_toggle') {
      bridge.snapshot.window.recordsBrowsing = true;
      bridge.snapshot.window.translationVisible = !bridge.snapshot.window.translationVisible;
      bridge.snapshot.window.translationExtent = bridge.snapshot.window.translationVisible ? bridge.snapshot.window.translationWidth : 0;
      deliver('window_state', structuredClone(bridge.snapshot.window));
    }
    if (name === 'translation_preferences_set') {
      const patch = args as Partial<TranslationPreferences>;
      const next = { ...bridge.snapshot.translationPreferences, ...patch };
      bridge.snapshot.translationPreferences = next;
      bridge.snapshot.translationEngine = next.engine;
      deliver('translation_preferences_changed', structuredClone(next));
      deliver('translation_engine_changed', next.engine);
      return;
    }
    if (name === 'translation_shortcut_set') {
      const { action, binding } = args as { action: ShortcutAction; binding: TranslationShortcuts[ShortcutAction] };
      const error = shortcutError(action, binding, bridge.snapshot.translationPreferences.shortcuts);
      if (error) throw new Error(error);
      const shortcuts = { ...bridge.snapshot.translationPreferences.shortcuts, [action]: binding ? normalizeBinding(binding) : null };
      bridge.snapshot.translationPreferences = { ...bridge.snapshot.translationPreferences, shortcuts };
      deliver('translation_preferences_changed', structuredClone(bridge.snapshot.translationPreferences));
      return;
    }
    if (name === 'translation_shortcuts_restore') {
      bridge.snapshot.translationPreferences = { ...bridge.snapshot.translationPreferences, shortcuts: defaultTranslationShortcuts(bridge.snapshot.capabilities.primaryModifier) };
      deliver('translation_preferences_changed', structuredClone(bridge.snapshot.translationPreferences));
      return;
    }
    if (name === 'translation_shortcut_capture') return;
    if (name === 'app_snapshot') return structuredClone(bridge.snapshot);
    if (name === 'history_sample_select') {
      const selected = args as { sampleId: string | null; retranslate: boolean; preview?: boolean };
      if (selected.preview && bridge.snapshot.window.recordsBrowsing === false) return;
      const record = bridge.history.records.find(({ id }) => id === selected.sampleId);
      if (selected.sampleId !== null && !record) throw new Error('invalid_argument');
      if (selected.sampleId !== null) {
        bridge.snapshot.window.recordsBrowsing = true;
        bridge.snapshot.window.translationVisible = true;
        bridge.snapshot.window.translationExtent = bridge.snapshot.window.translationWidth;
        bridge.snapshot.window.historyVisible = true;
        bridge.snapshot.window.historyExtent = bridge.snapshot.window.historyWidth;
        deliver('window_state', structuredClone(bridge.snapshot.window));
      }
      bridge.history.selectedId = selected.sampleId; ++bridge.history.revision;
      deliver('history_sample_selected', { ...selected, record, revision: bridge.history.revision });
    }
    return undefined;
  });
  bridge.listen.mockReset().mockImplementation(async (name: string, callback: (event: { payload: unknown }) => void) => {
    const listeners = bridge.listeners.get(name) ?? new Set();
    listeners.add(callback); bridge.listeners.set(name, listeners);
    return () => listeners.delete(callback);
  });
  bridge.moved.mockReset().mockResolvedValue(() => {});
  scrollTo.mockClear(); scrollIntoView.mockClear(); setPointerCapture.mockClear(); releasePointerCapture.mockClear();
});

afterEach(() => { cleanup(); frames.clear(); });

function deliver(name: string, payload: unknown) {
  bridge.listeners.get(name)?.forEach((callback) => callback({ payload }));
}

export async function emit(name: string, payload: unknown) {
  await act(async () => {
    if (name === 'history_sample_selected') {
      bridge.history.selectedId = (payload as { sampleId: string | null }).sampleId; ++bridge.history.revision;
      payload = { revision: bridge.history.revision, ...payload as object };
    }
    deliver(name, payload);
  });
}

export async function windowState(patch: Partial<WindowState>) {
  Object.assign(bridge.snapshot.window, patch);
  await emit('window_state', structuredClone(bridge.snapshot.window));
}

export async function mount(label: 'translation' | 'history' | 'clipboard' | 'settings' = 'translation') {
  bridge.label = label;
  if (label === 'clipboard') { bridge.snapshot.window.recordView = 'clipboard'; bridge.snapshot.window.translationVisible = false; bridge.snapshot.window.translationExtent = 0; }
  if (label === 'history') bridge.snapshot.window.recordView = 'translation';
  const previous = bridge.invoke.mock.calls.filter(([name]) => name === 'app_snapshot').length;
  const result = render(<App />);
  await waitFor(() => {
    if (bridge.invoke.mock.calls.filter(([name]) => name === 'app_snapshot').length <= previous) throw new Error('App has not connected to its native snapshot');
  });
  await act(async () => {});
  return result;
}

export function calls(name: string) { return bridge.invoke.mock.calls.filter(([command]) => command === name); }

export async function flushFrames() {
  await act(async () => {
    const pending = [...frames.values()]; frames.clear();
    pending.forEach((callback) => callback(performance.now()));
  });
}

export function preferReducedMotion() { reducedMotion = true; }
