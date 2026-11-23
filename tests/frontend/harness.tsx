import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, vi } from 'vitest';
import type { HistorySnapshot } from '../../src/translation/history';
import { initialSnapshot, type AppSnapshot, type TranslationPreferences, type WindowState } from '../../src/shared/contracts';
import { clipboardOnlyActions, defaultClipboardShortcuts, isClipboardTranslateLaunchKey, sameClipboardBinding, sharedRecordActions, type ClipboardRecordAction } from '../../src/shared/clipboardShortcuts';
import { defaultClipboardBinding, defaultTranslationShortcuts, generalShortcutActions, globalTranslationActions, normalizeBinding, recordWindowActions, shortcutError, translationWindowActions, type KeyBinding, type ShortcutAction, type TranslationShortcuts } from '../../src/shared/shortcuts';

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
  if (!HTMLDialogElement.prototype.showModal) {
    Object.defineProperties(HTMLDialogElement.prototype, {
      showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
      close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
    });
  }
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
  bridge.snapshot.clipboardControls.shortcuts = defaultClipboardShortcuts('meta');
  bridge.snapshot.capabilities = { platform: 'macos', nativeHistory: true, nativeChrome: true, nativeMaterial: true, selection: true, imageOcr: true, clipboardWrite: true, autoPaste: true, clipboardOpenTarget: true, clipboardImageSave: true, accessibilityPermission: true, primaryModifier: 'meta' };
  bridge.snapshot.translationEngine = 'edge';
  bridge.snapshot.translationPreferences = { engine: 'edge', sourceLanguage: 'auto', primaryLanguage: 'en', secondaryLanguage: 'zh-CN', proxyUrl: '', proxyEnabled: false, selectionEnabled: true, disabledShortcuts: [], shortcuts: defaultTranslationShortcuts('meta') };
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
      const request = args as { text: string; source: string; targets: string[]; clipboardSession?: number; reuse?: { recordId: string; targetIndex?: number; focusHistory?: boolean } };
      const previous = bridge.history.records.find(({ id }) => id === request.reuse?.recordId);
      const count = bridge.invoke.mock.calls.filter(([name]) => name === 'translation_run').length;
      const record = { ...historySamples[0], id: count === 1 ? 'translation-test' : `translation-test-${count}`, text: request.text, sourceLanguage: request.source, translation: { requestedSource: request.source, engine: bridge.snapshot.translationEngine, strategyVersion: 1, detections: [] }, targetLanguages: request.targets, translations: request.targets.map((target, index) => request.reuse?.targetIndex !== undefined && index !== request.reuse.targetIndex && previous?.text === request.text && previous.sourceLanguage === request.source && previous.targetLanguages[index] === target ? previous.translations[index] : `Synthetic translation ${index + 1}`), isFavorite: false, isPinned: false, sourceApp: previous?.text === request.text ? previous.sourceApp : undefined };
      bridge.history.records = [...bridge.history.records.filter(({ id }) => id !== record.id), record];
      if (!bridge.snapshot.window.recordsBrowsing || request.reuse?.focusHistory) bridge.history.selectedId = record.id;
      ++bridge.history.revision;
      deliver('history_changed', historySnapshot());
      const window = bridge.snapshot.window;
      if (request.reuse?.focusHistory || request.clipboardSession !== undefined && request.clipboardSession === (window.recordsSession ?? 0) && window.historyVisible && window.recordView === 'translation' && window.translationVisible && !window.recordsBrowsing) deliver('history_refocus', record.id);
      return record;
    }
    if (name === 'translation_focus_input') deliver('translation_focus_requested', null);
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
    if (name === 'history_record_note') {
      const record = bridge.history.records.find(({ id }) => id === (args as { recordId: string }).recordId);
      if (!record) throw new Error('not_found');
      return { title: record.title ?? '', text: record.text, kind: 'text' };
    }
    if (name === 'history_record_update') {
      const { recordId, action, title } = args as { recordId: string; action: string; title?: string };
      const record = bridge.history.records.find(({ id }) => id === recordId);
      if (!record) throw new Error('not_found');
      if (action === 'delete') {
        if (record.isFavorite || record.isPinned) throw new Error('history_protected');
        bridge.history.records = bridge.history.records.filter(({ id }) => id !== recordId);
        if (bridge.history.selectedId === recordId) bridge.history.selectedId = null;
      } else if (action === 'favorite') record.isFavorite = !record.isFavorite;
      else if (action === 'pin') record.isPinned = !record.isPinned;
      else if (action === 'note' && title !== undefined) record.title = title.trim();
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
      const recordId = (args as { recordId?: string } | undefined)?.recordId;
      const state = bridge.snapshot.window;
      state.recordsBrowsing = !recordId;
      state.translationVisible = !state.translationVisible || Boolean(recordId && state.clipboardDetailId !== recordId);
      if (recordId) state.clipboardDetailId = recordId;
      state.translationExtent = state.translationVisible ? state.translationWidth : 0;
      deliver('window_state', structuredClone(state));
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
      const { action, binding, enabled } = args as { action: ShortcutAction; binding: TranslationShortcuts[ShortcutAction]; enabled?: boolean };
      const preferences = bridge.snapshot.translationPreferences;
      const candidate = enabled === true ? preferences.shortcuts[action] : binding;
      if (candidate && Object.entries(bridge.snapshot.clipboardControls.shortcuts).some(([recordAction, shortcut]) =>
        (generalShortcutActions.some((entry) => entry === action) || sharedRecordActions.some((entry) => entry === recordAction))
        && !(bridge.snapshot.capabilities.platform === 'macos' && recordAction === 'translate' && globalTranslationActions.some((entry) => entry === action) && isClipboardTranslateLaunchKey(candidate))
        && shortcut.binding && sameClipboardBinding(shortcut.binding, candidate))) throw new Error('shortcut_clipboard_panel');
      if (enabled !== undefined) {
        if (action === 'selection') preferences.selectionEnabled = enabled;
        else preferences.disabledShortcuts = enabled ? preferences.disabledShortcuts.filter((item) => item !== action) : [...new Set([...preferences.disabledShortcuts, action])];
      } else {
        const error = shortcutError(action, binding, preferences.shortcuts, bridge.snapshot.clipboardBinding);
        if (error) throw new Error(error);
        preferences.shortcuts = { ...preferences.shortcuts, [action]: binding ? normalizeBinding(binding) : null };
      }
      deliver('translation_preferences_changed', structuredClone(bridge.snapshot.translationPreferences));
      return;
    }
    if (name === 'translation_shortcuts_restore') {
      const general = (args as { general?: boolean } | undefined)?.general;
      const preferences = bridge.snapshot.translationPreferences;
      const primary = bridge.snapshot.capabilities.primaryModifier;
      const defaults = defaultTranslationShortcuts(primary);
      const actions: readonly ShortcutAction[] = general ? recordWindowActions : [...globalTranslationActions, ...translationWindowActions];
      for (const action of actions) preferences.shortcuts[action] = defaults[action];
      preferences.disabledShortcuts = preferences.disabledShortcuts.filter((action) => !actions.includes(action));
      if (!general) preferences.selectionEnabled = true;
      if (general) {
        const defaults = defaultClipboardShortcuts(primary);
        for (const action of sharedRecordActions) bridge.snapshot.clipboardControls.shortcuts[action] = defaults[action];
        deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      }
      deliver('translation_preferences_changed', structuredClone(bridge.snapshot.translationPreferences));
      return;
    }
    if (name === 'clipboard_recording_type_set') {
      const { kind, enabled } = args as { kind: keyof AppSnapshot['clipboardRecordingTypes']; enabled: boolean };
      bridge.snapshot.clipboardRecordingTypes = { ...bridge.snapshot.clipboardRecordingTypes, [kind]: enabled };
      deliver('clipboard_recording_types_changed', structuredClone(bridge.snapshot.clipboardRecordingTypes));
      return structuredClone(bridge.snapshot.clipboardRecordingTypes);
    }
    if (name === 'clipboard_record_shortcut_set') {
      const { action, enabled, binding } = args as { action: ClipboardRecordAction; enabled?: boolean; binding?: KeyBinding | null };
      const shortcuts = bridge.snapshot.clipboardControls.shortcuts;
      const candidate = enabled === true ? shortcuts[action].binding : binding;
      const reserved = sharedRecordActions.some((entry) => entry === action)
        ? [...generalShortcutActions, ...translationWindowActions] : generalShortcutActions;
      if (candidate && [...Object.entries(shortcuts).flatMap(([key, shortcut]) => key === action ? [] : [shortcut.binding]), bridge.snapshot.clipboardBinding, ...reserved.filter((entry) => !(bridge.snapshot.capabilities.platform === 'macos' && action === 'translate' && isClipboardTranslateLaunchKey(candidate) && globalTranslationActions.some((launch) => launch === entry))).map((entry) => bridge.snapshot.translationPreferences.shortcuts[entry])]
        .some((current) => current && sameClipboardBinding(current, candidate))) throw new Error('shortcut_clipboard_panel');
      if (enabled !== undefined) shortcuts[action].enabled = enabled;
      else shortcuts[action].binding = binding ?? null;
      deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      return;
    }
    if (name === 'records_preview_lines_set') {
      const { view, lines } = args as { view: 'clipboard' | 'translation'; lines: number };
      bridge.snapshot.clipboardControls.previewLines = { ...bridge.snapshot.clipboardControls.previewLines!, [view]: lines };
      deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      return;
    }
    if (name === 'records_open_focus_set') {
      bridge.snapshot.clipboardControls.recordsOpenFocus = (args as { focus: 'search' | 'list' }).focus;
      deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      return;
    }
    if (name === 'clipboard_search_set') {
      bridge.snapshot.clipboardControls.search = (args as { search: NonNullable<AppSnapshot['clipboardControls']['search']> }).search;
      deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      return;
    }
    if (name === 'clipboard_behavior_set') {
      const { activation } = args as { activation?: 'copy' | 'paste' };
      if (activation !== undefined) {
        bridge.snapshot.clipboardControls.activation = activation;
        deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      }
      return;
    }
    if (name === 'clipboard_shortcut_set') {
      const { binding, enabled, restore } = args as { binding: KeyBinding | null; enabled?: boolean; restore?: boolean };
      if (restore) {
        bridge.snapshot.clipboardBinding = defaultClipboardBinding;
        bridge.snapshot.clipboardShortcutEnabled = true;
        const defaults = defaultClipboardShortcuts(bridge.snapshot.capabilities.primaryModifier);
        for (const action of clipboardOnlyActions) bridge.snapshot.clipboardControls.shortcuts[action] = defaults[action];
        deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      } else if (enabled !== undefined) bridge.snapshot.clipboardShortcutEnabled = enabled;
      else {
        const error = shortcutError('clipboard', binding, bridge.snapshot.translationPreferences.shortcuts, null);
        if (error) throw new Error(error);
        bridge.snapshot.clipboardBinding = binding ? normalizeBinding(binding) : null;
      }
      deliver('clipboard_binding_changed', { binding: bridge.snapshot.clipboardBinding, enabled: bridge.snapshot.clipboardShortcutEnabled, shortcutErrors: bridge.snapshot.shortcutErrors });
      return bridge.snapshot.clipboardBinding;
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
