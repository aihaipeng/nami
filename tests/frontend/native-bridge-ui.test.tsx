// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';

const source = () => screen.getByRole('textbox', { name: 'Source text' });

describe('native view connection and feedback', () => {
  it('reports a failed native layout command and keeps the editable draft', async () => {
    const user = userEvent.setup(); await mount();
    fireEvent.change(source(), { target: { value: 'Keep my draft after an error' } });
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'history_toggle') throw new Error('window_geometry');
      return invoke(name, args);
    });
    await user.click(screen.getByRole('button', { name: 'Translation history' }));
    expect(screen.getByRole('alert').textContent).toContain('The native operation did not complete');
    expect((source() as HTMLTextAreaElement).value).toBe('Keep my draft after an error');
    bridge.invoke.mockImplementation(invoke);
    await user.click(screen.getByRole('button', { name: 'Translation history' }));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('receives asynchronous native layout failures without losing the source', async () => {
    await mount(); fireEvent.change(source(), { target: { value: 'A local draft' } });
    await emit('layout_error', 'window_geometry');
    expect(screen.getByRole('alert')).toBeTruthy();
    expect((source() as HTMLTextAreaElement).value).toBe('A local draft');
  });

  it('shows only an animated progress indicator during an in-flight copy', async () => {
    await mount(); await emit('history_sample_selected', { sampleId: 'sample-01', retranslate: false });
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_sample_copy' ? pending : original(name, args));
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(screen.getByRole('progressbar', { name: 'Working' }).textContent).toBe('');
    expect(screen.queryByText('Working')).toBeNull();
    await act(async () => { finish(); });
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('does not display loading UI for an in-flight history toggle', async () => {
    const user = userEvent.setup(); await mount();
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_toggle' ? pending : original(name, args));
    await user.click(screen.getByRole('button', { name: 'Translation history' }));
    expect(screen.queryByRole('progressbar')).toBeNull();
    await act(async () => { finish(); });
  });

  it('updates labels immediately on native locale events and closes an open dropdown', async () => {
    const user = userEvent.setup(); await mount();
    await user.click(screen.getByRole('button', { name: 'Source language: Auto-Detect' }));
    expect(screen.getByRole('dialog')).toBeTruthy();
    await emit('locale_changed', 'zh-CN');
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('textbox', { name: '原文' })).toBeTruthy();
    expect(document.documentElement.lang).toBe('zh-CN');
  });

  it.each(['resize', 'blur', 'nami:window-moved'])('closes a dropdown on %s', async (type) => {
    const user = userEvent.setup(); await mount();
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    fireEvent(window, new Event(type));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('closes dropdowns when the history size changes and keeps the translation source', async () => {
    const user = userEvent.setup(); await mount();
    fireEvent.change(source(), { target: { value: 'Preserve during resize' } });
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    await windowState({ historyWidth: 430, historyExtent: 430 });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect((source() as HTMLTextAreaElement).value).toBe('Preserve during resize');
  });

  it('unsubscribes all native listeners when a view is removed', async () => {
    const view = await mount();
    expect(bridge.listeners.get('storage_error')?.size).toBe(1);
    expect(bridge.listeners.get('clipboard_capture_changed')?.size).toBe(1);
    expect(bridge.listeners.get('clipboard_translation_ready')?.size).toBe(1);
    expect([...bridge.listeners.values()].reduce((count, listeners) => count + listeners.size, 0)).toBe(10);
    view.unmount(); await act(async () => {});
    expect([...bridge.listeners.values()].reduce((count, listeners) => count + listeners.size, 0)).toBe(0);
    const count = calls('window_show').length;
    await emit('history_sample_selected', { sampleId: 'sample-01', retranslate: false });
    expect(calls('window_show')).toHaveLength(count);
  });

  it('uses a window event received during connection instead of an older snapshot', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    const stale = structuredClone(bridge.snapshot);
    let finish!: (snapshot: typeof stale) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'app_snapshot'
      ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
    await mount();
    await windowState({ historyVisible: false, historyExtent: 0 });
    await act(async () => { finish(stale); });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Translation history' }).getAttribute('aria-expanded')).toBe('false'));
  });
});
