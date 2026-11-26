import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, vi } from 'vitest';
import { initialSnapshot, type AppSnapshot, type TranslationOutput, type TranslationPreferences, type TranslationScope, type WindowState } from '../../src/shared/contracts';
import { clipboardOnlyActions, defaultClipboardShortcuts, isClipboardTranslateLaunchKey, sameClipboardBinding, sharedRecordActions, type ClipboardRecordAction } from '../../src/shared/clipboardShortcuts';
import { defaultClipboardBinding, defaultTranslationShortcuts, generalShortcutActions, globalTranslationActions, normalizeBinding, recordWindowActions, shortcutError, translationWindowActions, type KeyBinding, type ShortcutAction, type TranslationShortcuts } from '../../src/shared/shortcuts';

const bridge = vi.hoisted(() => ({
  label: 'translation',
  snapshot: undefined as unknown as AppSnapshot,
  translationOutputs: { standalone: null as TranslationOutput | null, clipboard: null as TranslationOutput | null },
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  invoke: vi.fn(),
  listen: vi.fn(),
  moved: vi.fn(),
}));

export { bridge };

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: bridge.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: bridge.listen }));
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ label: bridge.label }) }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'translation', onMoved: bridge.moved }) }));

import App from '../../src/App';

const frames = new Map<number, FrameRequestCallback>();
let nextFrame = 0;

function rightExtentFor(window: WindowState) {
  if (window.standaloneTranslationVisible) return window.translationWidth;
  if (!window.clipboardVisible) return 0;
  return (window.clipboardDetailVisible ? window.clipboardDetailWidth : 0)
    + (window.clipboardTranslationVisible ? window.clipboardTranslationWidth : 0);
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
  bridge.snapshot.capabilities = { platform: 'macos', nativeHistory: true, nativeChrome: true, nativeMaterial: true, selection: true, imageOcr: true, appleTranslation: false, clipboardWrite: true, autoPaste: true, clipboardOpenTarget: true, clipboardImageSave: true, clipboardDrag: true, localBackup: true, storageLocations: true, launchAtLogin: true, accessibilityPermission: true, primaryModifier: 'meta' };
  bridge.snapshot.translationEngine = 'edge';
  bridge.snapshot.translationPreferences = { engine: 'edge', sourceLanguage: 'auto', primaryLanguage: 'en', secondaryLanguage: 'zh-CN', proxyUrl: '', proxyEnabled: false, selectionEnabled: true, disabledShortcuts: [], shortcuts: defaultTranslationShortcuts('meta') };
  bridge.translationOutputs = { standalone: null, clipboard: null };
  bridge.snapshot.window.standaloneTranslationVisible = true;
  bridge.snapshot.window.clipboardVisible = false;
  bridge.snapshot.window.clipboardDetailVisible = false;
  bridge.snapshot.window.clipboardTranslationVisible = false;
  bridge.snapshot.window.clipboardTranslationRecordId = null;
  bridge.snapshot.window.clipboardExtent = 0;
  bridge.snapshot.window.rightExtent = bridge.snapshot.window.translationWidth;
  bridge.listeners.clear(); frames.clear(); reducedMotion = false;
  bridge.invoke.mockReset().mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'selection_take' || name === 'clipboard_translation_take') return null;
    if (name === 'translation_cancel') return undefined;
    if (name === 'translation_run') {
      const request = args as { text: string; source: string; targets: string[]; scope?: TranslationScope; reuseTargetIndex?: number };
      const scope: TranslationScope = request.scope ?? 'standalone';
      const previous = bridge.translationOutputs[scope];
      const count = bridge.invoke.mock.calls.filter(([cmd]) => cmd === 'translation_run').length;
      const output: TranslationOutput = {
        id: count === 1 ? 'translation-test' : `translation-test-${count}`,
        text: request.text,
        sourceLanguage: request.source,
        translation: { requestedSource: request.source, engine: bridge.snapshot.translationEngine, strategyVersion: 1, detections: [] },
        targetLanguages: request.targets,
        translations: request.targets.map((target, index) => request.reuseTargetIndex !== undefined && index !== request.reuseTargetIndex && previous?.text === request.text && previous.translation.requestedSource === request.source && previous.targetLanguages[index] === target ? previous.translations[index] : `Synthetic translation ${index + 1}`),
      };
      bridge.translationOutputs[scope] = output;
      return output;
    }
    if (name === 'translation_copy') return undefined;
    if (name === 'clipboard_capture_set') {
      bridge.snapshot.clipboardCaptureEnabled = Boolean((args as { enabled: boolean }).enabled);
      deliver('clipboard_capture_changed', bridge.snapshot.clipboardCaptureEnabled);
      return;
    }
    if (name === 'launch_at_login_set') {
      bridge.snapshot.launchAtLogin = Boolean((args as { enabled: boolean }).enabled);
      deliver('launch_at_login_changed', bridge.snapshot.launchAtLogin);
      return bridge.snapshot.launchAtLogin;
    }
    if (name === 'window_position_mode_set') {
      const mode = (args as { mode: typeof bridge.snapshot.windowPositionMode }).mode;
      bridge.snapshot.windowPositionMode = mode;
      deliver('window_position_mode_changed', mode);
      return;
    }
    if (name === 'clipboard_details_select') {
      const browsing = (args as { browsing?: boolean }).browsing;
      if (browsing !== undefined) bridge.snapshot.window.recordsBrowsing = browsing;
      else if (bridge.snapshot.window.recordsBrowsing === false) return;
      bridge.snapshot.window.clipboardDetailId = (args as { recordId: string | null }).recordId;
      deliver('window_state', structuredClone(bridge.snapshot.window));
    }
    if (name === 'records_editor_set' && (args as { open?: boolean }).open !== undefined) {
      bridge.snapshot.window.recordsEditorOpen = (args as { open: boolean }).open;
      deliver('window_state', structuredClone(bridge.snapshot.window));
    }
    if (name === 'clipboard_note_open') deliver('clipboard_note_requested', (args as { recordId: string }).recordId);
    if (name === 'records_details_toggle') {
      const { recordId, returnFocus } = (args ?? {}) as { recordId?: string; returnFocus?: boolean };
      const state = bridge.snapshot.window;
      state.recordsBrowsing = true;
      state.clipboardVisible = true;
      state.standaloneTranslationVisible = false;
      state.clipboardDetailVisible = !state.clipboardDetailVisible || Boolean(recordId && state.clipboardDetailId !== recordId);
      if (recordId) state.clipboardDetailId = recordId;
      state.clipboardExtent = state.clipboardWidth;
      state.rightExtent = rightExtentFor(state);
      deliver('window_state', structuredClone(state));
      if (returnFocus && !state.clipboardDetailVisible) deliver('clipboard_focus_requested', state.clipboardDetailId ?? null);
    }
    if (name === 'clipboard_translation_close') {
      const state = bridge.snapshot.window;
      const focusId = state.clipboardDetailVisible ? state.clipboardDetailId : state.clipboardTranslationRecordId;
      state.clipboardTranslationVisible = false;
      state.clipboardTranslationRecordId = null;
      state.rightExtent = rightExtentFor(state);
      deliver('window_state', structuredClone(state));
      if ((args as { returnFocus?: boolean } | undefined)?.returnFocus) deliver('clipboard_focus_requested', focusId ?? null);
    }
    if (name === 'clipboard_translation_clear_record') {
      bridge.snapshot.window.clipboardTranslationRecordId = null;
      deliver('window_state', structuredClone(bridge.snapshot.window));
    }
    if (name === 'clipboard_width_set') return undefined;
    if (name === 'clipboard_detail_width_set') {
      const width = Math.min(920, Math.max(320, Number((args as { width: number }).width)));
      bridge.snapshot.window.clipboardDetailWidth = width;
      bridge.snapshot.window.rightExtent = rightExtentFor(bridge.snapshot.window);
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
      const { general, group } = (args as { general?: boolean; group?: 'global' | 'window' | 'records' | 'translation' } | undefined) ?? {};
      const preferences = bridge.snapshot.translationPreferences;
      const primary = bridge.snapshot.capabilities.primaryModifier;
      const defaults = defaultTranslationShortcuts(primary);
      const recordDefaults = defaultClipboardShortcuts(primary);
      if (group === 'global') {
        bridge.snapshot.clipboardBinding = defaultClipboardBinding;
        bridge.snapshot.clipboardShortcutEnabled = true;
        for (const action of globalTranslationActions) preferences.shortcuts[action] = defaults[action];
        preferences.disabledShortcuts = preferences.disabledShortcuts.filter((action) => !globalTranslationActions.some((entry) => entry === action));
        preferences.selectionEnabled = true;
        deliver('translation_preferences_changed', structuredClone(bridge.snapshot.translationPreferences));
        deliver('clipboard_binding_changed', { binding: bridge.snapshot.clipboardBinding, enabled: bridge.snapshot.clipboardShortcutEnabled, shortcutErrors: bridge.snapshot.shortcutErrors });
        return;
      }
      if (group === 'window') {
        for (const action of recordWindowActions) preferences.shortcuts[action] = defaults[action];
        preferences.disabledShortcuts = preferences.disabledShortcuts.filter((action) => !recordWindowActions.some((entry) => entry === action));
        bridge.snapshot.clipboardControls.shortcuts.switchCategory = recordDefaults.switchCategory;
        deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
        deliver('translation_preferences_changed', structuredClone(bridge.snapshot.translationPreferences));
        return;
      }
      if (group === 'records') {
        const currentSwitch = bridge.snapshot.clipboardControls.shortcuts.switchCategory;
        Object.assign(bridge.snapshot.clipboardControls.shortcuts, recordDefaults, { switchCategory: currentSwitch });
        deliver('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
        return;
      }
      if (group === 'translation') {
        for (const action of translationWindowActions) preferences.shortcuts[action] = defaults[action];
        preferences.disabledShortcuts = preferences.disabledShortcuts.filter((action) => !translationWindowActions.some((entry) => entry === action));
        deliver('translation_preferences_changed', structuredClone(bridge.snapshot.translationPreferences));
        return;
      }
      const actions: readonly ShortcutAction[] = general ? recordWindowActions : [...globalTranslationActions, ...translationWindowActions];
      for (const action of actions) preferences.shortcuts[action] = defaults[action];
      preferences.disabledShortcuts = preferences.disabledShortcuts.filter((action) => !actions.includes(action));
      if (!general) preferences.selectionEnabled = true;
      if (general) {
        for (const action of sharedRecordActions) bridge.snapshot.clipboardControls.shortcuts[action] = recordDefaults[action];
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
      const { lines } = args as { lines: number };
      bridge.snapshot.clipboardControls.previewLines = { clipboard: lines };
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
      const { activation, hoverPreviewEnabled, hoverPreviewDelayMs } = args as { activation?: 'copy' | 'paste'; hoverPreviewEnabled?: boolean; hoverPreviewDelayMs?: number };
      if (activation !== undefined) bridge.snapshot.clipboardControls.activation = activation;
      if (hoverPreviewEnabled !== undefined) bridge.snapshot.clipboardControls.hoverPreviewEnabled = hoverPreviewEnabled;
      if (hoverPreviewDelayMs !== undefined) bridge.snapshot.clipboardControls.hoverPreviewDelayMs = hoverPreviewDelayMs;
      if (activation !== undefined || hoverPreviewEnabled !== undefined || hoverPreviewDelayMs !== undefined) {
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
    deliver(name, payload);
  });
}

export async function windowState(patch: Partial<WindowState>) {
  Object.assign(bridge.snapshot.window, patch);
  await emit('window_state', structuredClone(bridge.snapshot.window));
}

export async function mount(label: 'translation' | 'clipboard' | 'settings' = 'translation') {
  bridge.label = label;
  if (label === 'clipboard') {
    bridge.snapshot.window.standaloneTranslationVisible = false;
    bridge.snapshot.window.clipboardVisible = true;
    bridge.snapshot.window.clipboardExtent = bridge.snapshot.window.clipboardWidth;
    bridge.snapshot.window.rightExtent = rightExtentFor(bridge.snapshot.window);
  }
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
