// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';
import type { HistorySnapshot } from '../../src/translation/history';
import { defaultTranslationShortcuts } from '../../src/shared/shortcuts';

function portable(platform: 'windows' | 'linux' = 'windows') {
  bridge.snapshot.window.recordView = 'translation';
  bridge.snapshot.capabilities = {
    platform, nativeHistory: false, nativeChrome: false, nativeMaterial: false,
    selection: false, imageOcr: false, clipboardWrite: false, autoPaste: false, accessibilityPermission: false,
    primaryModifier: 'control',
  };
  bridge.snapshot.translationPreferences = { ...bridge.snapshot.translationPreferences, shortcuts: defaultTranslationShortcuts('control') };
}

const rows = () => screen.queryAllByRole('button', { name: /^Translation:/ });
const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;

describe('shared history and platform capabilities', () => {
  it.each(['windows', 'linux'] as const)('renders backend history and DOM controls in a %s Tauri host', async (platform) => {
    portable(platform); await mount();
    expect(rows()).toHaveLength(20);
    expect(screen.getByRole('separator', { name: 'Resize translation history' })).toBeTruthy();
    expect(calls('translation_chrome_update')).toHaveLength(0);
    expect(calls('selection_take')).toHaveLength(0);
    expect(document.documentElement.dataset.nativeMaterial).toBe('false');
    expect(document.documentElement.dataset.nativeHistory).toBe('false');
    expect(screen.getAllByRole('button', { name: 'Copy source text' }).every((button) => (button as HTMLButtonElement).disabled)).toBe(true);
  });

  it('keeps the macOS native history path free of a duplicate React list', async () => {
    await mount();
    expect(rows()).toHaveLength(0);
    expect(calls('history_snapshot')).toHaveLength(0);
    expect(document.documentElement.dataset.nativeMaterial).toBe('true');
    expect(document.documentElement.dataset.nativeHistory).toBe('true');
  });

  it('applies divider widths immediately and leaves visibility changes eligible for opening animation', async () => {
    portable(); await mount();
    const clip = document.querySelector<HTMLElement>('.history-clip')!;
    expect(clip.style.transition).toBe('');
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize translation history' }), { key: 'ArrowRight' });
    expect(calls('history_width_set').at(-1)?.[1]).toEqual({ width: 384, initialize: false });
    await windowState({ historyWidth: 384, historyExtent: 384 });
    expect(clip.style.transition).toBe('none');
    await windowState({ historyWidth: 440, historyExtent: 440 });
    expect(clip.style.transition).toBe('none');
    await windowState({ historyVisible: false, historyExtent: 0 });
    expect(clip.style.transition).toBe('');
    await windowState({ historyVisible: true, historyExtent: 440 });
    expect(clip.style.transition).toBe('');
    expect(clip.hidden).toBe(false);
  });

  it('uses Control shortcuts and key labels without treating Super as Control', async () => {
    portable(); await mount();
    fireEvent.change(source(), { target: { value: 'Portable synthetic text' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    expect(calls('translation_run')).toHaveLength(0);
    fireEvent.keyDown(source(), { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    fireEvent.keyDown(source(), { key: 'k', ctrlKey: true });
    const menu = screen.getByRole('menu', { name: 'Actions' });
    expect(menu.textContent).toContain('Ctrl');
    expect(menu.textContent).not.toContain('⌘');
    expect((within(menu).getByRole('menuitem', { name: /Copy Primary Translation/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('does not replace newer history events with a delayed initial snapshot', async () => {
    portable();
    let finish!: (value: HistorySnapshot) => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_snapshot'
      ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
    const stale = structuredClone(bridge.history);
    await mount();
    await waitFor(() => expect(calls('history_snapshot')).toHaveLength(1));
    const newest = { ...stale, revision: 5, records: [stale.records[2]], selectedId: stale.records[2].id };
    await emit('history_changed', newest);
    expect(rows()).toHaveLength(1);
    expect(rows()[0].getAttribute('aria-pressed')).toBe('true');
    await act(async () => finish(stale));
    expect(rows()).toHaveLength(1);
    await emit('history_changed', { ...stale, revision: 4 });
    expect(rows()).toHaveLength(1);
  });

  it('receives records created by another view and displays their source application icon', async () => {
    portable(); await mount();
    const record = {
      ...bridge.history.records[2], id: 'shared-runtime-record', text: 'A new shared record',
      sourceApp: { bundleId: 'synthetic.application', name: 'Synthetic App', iconDataUrl: 'data:image/png;base64,c3ludGhldGlj' },
    };
    await emit('history_changed', { revision: 1, records: [...bridge.history.records, record], selectedId: record.id });
    expect(rows()).toHaveLength(21);
    const row = screen.getByRole('button', { name: 'Translation: A new shared record' });
    expect(row.getAttribute('aria-pressed')).toBe('true');
    expect(within(row).getByRole('img', { name: 'Synthetic App' }).getAttribute('src')).toBe(record.sourceApp.iconDataUrl);
  });

  it('adds a successful translation to shared history and restores a previous record through the backend', async () => {
    const user = userEvent.setup(); portable(); await mount();
    fireEvent.change(source(), { target: { value: 'A portable runtime translation' } });
    fireEvent.keyDown(source(), { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(rows()).toHaveLength(21));
    expect(screen.getByRole('button', { name: 'Translation: A portable runtime translation' }).getAttribute('aria-pressed')).toBe('true');
    const record = bridge.history.records.find(({ id }) => id === 'sample-03')!;
    await user.click(screen.getByRole('button', { name: `Translation: ${record.text}` }));
    expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: record.id, retranslate: false });
    expect(source().value).toBe(record.text);
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe(record.translations[0]);
  });

  it('keeps an unprotected record visible when the backend rejects deletion after a concurrent favorite', async () => {
    const user = userEvent.setup(); portable(); await mount();
    const record = bridge.history.records.find(({ id }) => id === 'sample-03')!;
    const row = screen.getByRole('button', { name: `Translation: ${record.text}` }).closest('article')!;
    record.isFavorite = true;
    await user.click(within(row).getByRole('button', { name: 'Delete history record' }));
    await user.click(within(row).getByRole('button', { name: 'Delete history record' }));
    expect(calls('history_record_update').at(-1)?.[1]).toEqual({ recordId: record.id, action: 'delete' });
    expect(screen.getByRole('alert').textContent).toContain('The native operation did not complete');
    expect(rows()).toHaveLength(20);
    expect(bridge.history.records.some(({ id }) => id === record.id)).toBe(true);
  });

  it('reports failed history loading and removes its listener on unmount', async () => {
    portable();
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_snapshot'
      ? Promise.reject(new Error('unavailable')) : original(name, args));
    const view = await mount();
    expect(screen.getByRole('alert')).toBeTruthy();
    expect(bridge.listeners.get('history_changed')?.size).toBe(1);
    view.unmount(); await act(async () => {});
    expect(bridge.listeners.get('history_changed')?.size).toBe(0);
  });

  it('shows platform capability information instead of macOS permission actions', async () => {
    const user = userEvent.setup(); portable('linux'); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'Privacy & data' }));
    expect(screen.queryByRole('button', { name: 'Request permission' })).toBeNull();
    expect(screen.getByText(/Selected text capture and automatic paste aren’t available/)).toBeTruthy();
    expect(calls('permission_request')).toHaveLength(0);
  });

  it('updates material and permission UI when a refreshed capability snapshot changes', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'Privacy & data' }));
    expect(screen.getByRole('button', { name: 'Request permission' })).toBeTruthy();
    portable();
    await user.click(screen.getByRole('button', { name: 'Refresh permission status' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Request permission' })).toBeNull());
    expect(document.documentElement.dataset.nativeMaterial).toBe('false');
    expect(document.documentElement.dataset.nativeHistory).toBe('false');
  });

  it('cleans up a history subscription that finishes after the view unmounts', async () => {
    portable();
    const original = bridge.listen.getMockImplementation()!;
    let finish!: (remove: () => void) => void;
    bridge.listen.mockImplementation((name: string, callback: (event: { payload: unknown }) => void) => name === 'history_changed'
      ? new Promise((resolve) => { finish = resolve; }) : original(name, callback));
    const view = await mount();
    view.unmount();
    let removed = false;
    await act(async () => { finish(() => { removed = true; }); });
    expect(removed).toBe(true);
    expect(calls('history_snapshot')).toHaveLength(0);
  });
});
