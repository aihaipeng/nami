import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, vi } from 'vitest';
import { initialSnapshot, type AppSnapshot, type WindowState } from '../../src/shared/contracts';

const bridge = vi.hoisted(() => ({
  label: 'translation',
  snapshot: undefined as unknown as AppSnapshot,
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
import { historySamples } from '../../src/translation/history';

const frames = new Map<number, FrameRequestCallback>();
let nextFrame = 0;
export const scrollTo = vi.fn(function (this: HTMLElement, first?: number | ScrollToOptions, second?: number) {
  this.scrollTop = typeof first === 'number' ? second ?? 0 : first?.top ?? this.scrollTop;
});
export const scrollIntoView = vi.fn();
export const setPointerCapture = vi.fn();
export const releasePointerCapture = vi.fn();
export let reducedMotion = false;

beforeAll(() => {
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
  bridge.snapshot.window.historyVisible = true;
  bridge.snapshot.window.historyExtent = bridge.snapshot.window.historyWidth;
  bridge.listeners.clear(); frames.clear(); reducedMotion = false;
  bridge.invoke.mockReset().mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'selection_take') return null;
    if (name === 'translation_run') {
      const request = args as { text: string; source: string; targets: string[] };
      return { ...historySamples[0], id: 'translation-test', text: request.text, sourceLanguage: request.source, targetLanguages: request.targets, translations: request.targets.map((_, index) => `Synthetic translation ${index + 1}`) };
    }
    if (name === 'app_snapshot') return structuredClone(bridge.snapshot);
    if (name === 'history_sample_select') deliver('history_sample_selected', args);
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
  await act(async () => { deliver(name, payload); });
}

export async function windowState(patch: Partial<WindowState>) {
  Object.assign(bridge.snapshot.window, patch);
  await emit('window_state', structuredClone(bridge.snapshot.window));
}

export async function mount(label: 'translation' | 'history' | 'settings' = 'translation') {
  bridge.label = label;
  const previous = bridge.invoke.mock.calls.filter(([name]) => name === 'app_snapshot').length;
  const result = render(<App />);
  await waitFor(() => {
    if (bridge.invoke.mock.calls.filter(([name]) => name === 'app_snapshot').length <= previous) throw new Error('App has not connected to its native snapshot');
  });
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
