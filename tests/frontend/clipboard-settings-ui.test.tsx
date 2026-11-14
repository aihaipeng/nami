// @vitest-environment jsdom
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount } from './harness';

describe('clipboard recording settings', () => {
  it('shows the fixed Control and backquote shortcut', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect([...document.querySelectorAll('.keycaps kbd')].map((key) => key.textContent)).toEqual(['⌃', '`']);
  });

  it('starts disabled and changes only through an explicit setting command', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const toggle = screen.getByRole('switch', { name: 'Clipboard recording' }) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    expect(toggle.disabled).toBe(false);
    fireEvent.click(toggle);
    expect(calls('clipboard_capture_set')[0][1]).toEqual({ enabled: true });
    bridge.snapshot.clipboardCaptureEnabled = true;
    await emit('clipboard_capture_changed', true);
    await waitFor(() => expect(toggle.checked).toBe(true));
    expect(screen.getByRole('note').textContent).toContain('path');
  });

  it('disables recording on unsupported platforms and storage failure', async () => {
    bridge.snapshot.capabilities.clipboardWrite = false;
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect((screen.getByRole('switch') as HTMLInputElement).disabled).toBe(true);
    expect(calls('clipboard_capture_set')).toHaveLength(0);
  });

  it('preserves a recording event delivered before the initial app snapshot', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'app_snapshot') {
        await emit('clipboard_capture_changed', true);
        return structuredClone(bridge.snapshot);
      }
      return invoke(name, args);
    });
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect((screen.getByRole('switch') as HTMLInputElement).checked).toBe(true);
  });
});
