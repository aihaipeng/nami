// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';

const source = () => screen.getByRole('textbox', { name: 'Source text' });
const submit = async (text: string) => {
  fireEvent.change(source(), { target: { value: text } });
  fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
  await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
  await act(async () => {});
};

describe('native view connection and feedback', () => {
  it('reports a failed native layout command and keeps the editable draft', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: 'Keep my draft after an error' } });
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'window_pin') throw new Error('window_geometry');
      return invoke(name, args);
    });
    fireEvent.keyDown(source(), { key: 'p', code: 'KeyP', metaKey: true, shiftKey: true });
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('The native operation did not complete'));
    expect((source() as HTMLTextAreaElement).value).toBe('Keep my draft after an error');
    bridge.invoke.mockImplementation(invoke);
    fireEvent.keyDown(source(), { key: 'p', code: 'KeyP', metaKey: true, shiftKey: true });
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('receives asynchronous native layout failures without losing the source', async () => {
    await mount(); fireEvent.change(source(), { target: { value: 'A local draft' } });
    await emit('layout_error', 'window_geometry');
    expect(screen.getByRole('alert')).toBeTruthy();
    expect((source() as HTMLTextAreaElement).value).toBe('A local draft');
  });

  it('keeps translation copy silent while pending and ignores repeated shortcuts', async () => {
    await mount(); await submit('Copy silent test');
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_copy' ? pending : original(name, args));
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(screen.queryByRole('progressbar')).toBeNull();
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(calls('translation_copy')).toHaveLength(1);
    await act(async () => { finish(); });
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('allows different result parts while suppressing repeats of each pending copy', async () => {
    await mount(); await submit('Copy both parts');
    const original = bridge.invoke.getMockImplementation()!;
    const finish: (() => void)[] = [];
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_copy'
      ? new Promise<void>((resolve) => finish.push(resolve)) : original(name, args));
    for (const key of ['1', '1', '2', '2']) fireEvent.keyDown(source(), { key, metaKey: true });
    expect(calls('translation_copy').map(([, args]) => args)).toEqual([
      { scope: 'standalone', part: 'primary' },
      { scope: 'standalone', part: 'secondary' },
    ]);
    expect(screen.queryByRole('progressbar')).toBeNull();
    await act(async () => { finish.forEach((resolve) => resolve()); });
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(calls('translation_copy')).toHaveLength(3);
    await act(async () => finish[2]());
  });

  it('discards pending copy failure when standalone translation hides', async () => {
    await mount(); await submit('Copy context change');
    const original = bridge.invoke.getMockImplementation()!;
    let fail!: (error: Error) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_copy'
      ? new Promise((_resolve, reject) => { fail = reject; }) : original(name, args));
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    await windowState({ standaloneTranslationVisible: false });
    await act(async () => fail(new Error('clipboard_write')));
    expect(document.querySelector('.record-feedback')).toBeNull();
    expect(document.querySelector('.global-error')).toBeNull();
  });

  it('reports copy failure locally without moving or clearing the draft', async () => {
    await mount(); await submit('Copy failure local');
    const draft = (source() as HTMLTextAreaElement).value;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'translation_copy') throw new Error('clipboard_write');
      return original(name, args);
    });
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('Content was not copied'));
    expect(document.querySelector('.global-error')).toBeNull();
    expect((source() as HTMLTextAreaElement).value).toBe(draft);
  });

  it('does not display loading UI for an in-flight window pin command', async () => {
    await mount();
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'window_pin' ? pending : original(name, args));
    fireEvent.keyDown(source(), { key: 'p', code: 'KeyP', metaKey: true, shiftKey: true });
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

  it('closes dropdowns when the clipboard width changes and keeps the translation source', async () => {
    const user = userEvent.setup(); await mount();
    fireEvent.change(source(), { target: { value: 'Preserve during resize' } });
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    await windowState({ clipboardWidth: 430 });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect((source() as HTMLTextAreaElement).value).toBe('Preserve during resize');
  });

  it('unsubscribes all native listeners when a view is removed', async () => {
    const view = await mount();
    expect(bridge.listeners.get('storage_error')?.size).toBe(1);
    expect(bridge.listeners.get('clipboard_capture_changed')?.size).toBe(1);
    expect(bridge.listeners.get('launch_at_login_changed')?.size).toBe(1);
    expect(bridge.listeners.get('clipboard_binding_changed')?.size).toBe(1);
    expect(bridge.listeners.get('clipboard_controls_changed')?.size).toBe(1);
    expect(bridge.listeners.get('window_position_mode_changed')?.size).toBe(1);
    expect(bridge.listeners.get('translation_preferences_changed')?.size).toBe(2);
    expect([...bridge.listeners.values()].reduce((count, listeners) => count + listeners.size, 0)).toBe(14);
    view.unmount(); await act(async () => {});
    expect([...bridge.listeners.values()].reduce((count, listeners) => count + listeners.size, 0)).toBe(0);
  });

  it('uses a window event received during connection instead of an older snapshot', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    const stale = structuredClone(bridge.snapshot);
    let finish!: (snapshot: typeof stale) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'app_snapshot'
      ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
    await mount();
    await windowState({ standaloneTranslationVisible: false, clipboardVisible: true, clipboardDetailVisible: true, clipboardExtent: 360, rightExtent: 480 });
    await act(async () => { finish(stale); });
    await waitFor(() => expect(screen.getByRole('region', { name: 'Clipboard details' })).toBeTruthy());
  });
});
