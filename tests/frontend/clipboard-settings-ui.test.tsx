// @vitest-environment jsdom
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount } from './harness';

describe('clipboard recording settings', () => {
  it('shows the default Control and backquote shortcut', async () => {
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
    const capture = screen.getByRole('region', { name: 'Capture' });
    expect(capture.querySelector('[role=status]')).toBeNull();
    const images = screen.getByRole('switch', { name: 'Images' });
    expect(images.getAttribute('aria-description')).toBe('Includes supported single image files.');
  });

  it('preserves enabled content types while paused and shows when no types are selected', async () => {
    bridge.snapshot.clipboardCaptureEnabled = true;
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const image = screen.getByRole<HTMLInputElement>('switch', { name: 'Images' });
    expect(image.checked).toBe(true);
    fireEvent.click(image);
    await waitFor(() => { expect(image.checked).toBe(false); expect(image.hasAttribute('aria-disabled')).toBe(false); });
    expect(calls('clipboard_recording_type_set')[0][1]).toEqual({ kind: 'image', enabled: false });
    fireEvent.click(screen.getByRole('switch', { name: 'Clipboard recording' }));
    await emit('clipboard_capture_changed', false);
    expect(image.matches(':disabled')).toBe(true);
    expect(image.checked).toBe(false);
    await emit('clipboard_capture_changed', true);
    expect(image.matches(':disabled')).toBe(false);
    fireEvent.click(screen.getByRole('switch', { name: 'Text' }));
    await waitFor(() => {
      const text = screen.getByRole<HTMLInputElement>('switch', { name: 'Text' });
      expect(text.checked).toBe(false);
      expect(text.hasAttribute('aria-disabled')).toBe(false);
    });
    fireEvent.click(screen.getByRole('switch', { name: 'Files and folders' }));
    await screen.findByText('No content types selected. New copies won’t be recorded.');
  });

  it('records, cancels, clears and restores the shortcut, retaining it on conflict', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const control = () => screen.getByRole('button', { name: /Open Clipboard History shortcut/ });
    fireEvent.click(control());
    fireEvent.keyDown(window, { key: 't', code: 'KeyT', metaKey: true });
    await screen.findByText('Already assigned to Translate Selected Text.');
    expect(control().textContent).toBe('⌃`');
    fireEvent.click(control());
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(control().textContent).toBe('⌃⇧J'));
    fireEvent.click(control());
    fireEvent.keyDown(window, { key: 'Escape', code: 'Escape' });
    expect(control().textContent).toBe('⌃⇧J');
    fireEvent.click(control());
    fireEvent.keyDown(window, { key: 'Backspace', code: 'Backspace' });
    await waitFor(() => expect(control().textContent).toBe('Add Hotkey'));
    fireEvent.click(screen.getByRole('button', { name: 'Restore Default' }));
    await waitFor(() => expect(control().textContent).toBe('⌃`'));
  });

  it('disables recording on unsupported platforms and storage failure', async () => {
    bridge.snapshot.capabilities.clipboardWrite = false;
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect((screen.getByRole('switch', { name: 'Clipboard recording' }) as HTMLInputElement).disabled).toBe(true);
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
    fireEvent.click(await screen.findByRole('button', { name: 'Clipboard' }));
    expect((screen.getByRole('switch', { name: 'Clipboard recording' }) as HTMLInputElement).checked).toBe(true);
  });
});

describe('clipboard privacy, retention and behavior', () => {
  const excludedApps = [
    { bundleId: 'com.apple.Passwords', name: 'Passwords' },
    { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' },
  ];
  async function openSettings(apps = excludedApps) {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_options_get') return {
        excludedApps: apps,
        historyPolicy: { days: null, limit: null }, activation: 'paste',
      };
      if (name === 'clipboard_excluded_app_pick') return [{ bundleId: 'com.apple.Passwords', name: 'Passwords' }, { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' }, { bundleId: 'com.example.synthetic', name: 'Synthetic App' }];
      if (name === 'clipboard_excluded_app_remove') return [{ bundleId: 'com.apple.Passwords', name: 'Passwords' }, { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' }];
      if (name === 'clipboard_storage_info') return { bytes: 4000, unprotectedCount: 4 };
      if (name === 'clipboard_cleanup_preview') return { count: 4, token: 'preview-token' };
      if (name === 'clipboard_cleanup_apply') return { removed: 4, mediaCleanupFailed: false };
      return invoke(name, args);
    });
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    await screen.findByText('4 kB');
  }

  it('organizes the page into four named groups', async () => {
    await openSettings();
    for (const name of ['Capture', 'History & Storage', 'Behavior', 'Keyboard Shortcuts']) {
      expect(screen.getByRole('region', { name })).toBeTruthy();
    }
  });

  it('shows and manages excluded apps directly in Capture even when recording is off', async () => {
    await openSettings();
    const list = await screen.findByRole('list', { name: 'Excluded Apps' });
    expect(list.textContent).toContain('Passwords');
    expect(list.textContent).toContain('Keychain Access');
    expect(list.textContent).not.toContain('com.apple.Passwords');
    expect(screen.getByText('Passwords').title).toBe('com.apple.Passwords');
    expect(screen.queryByRole('dialog', { name: 'Excluded Apps' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Add App…' }));
    await screen.findByText('Synthetic App');
    fireEvent.click(screen.getByRole('button', { name: 'Remove Synthetic App' }));
    await waitFor(() => expect(screen.queryByText('Synthetic App')).toBeNull());
    expect(calls('clipboard_excluded_app_remove')[0][1]).toEqual({ bundleId: 'com.example.synthetic' });
  });

  it('shows identifiers only when app names are ambiguous', async () => {
    await openSettings([...excludedApps, { bundleId: 'com.example.passwords', name: 'Passwords' }]);
    for (const bundleId of ['com.apple.Passwords', 'com.example.passwords']) {
      expect(screen.getByText(bundleId)).toBeTruthy();
      expect(screen.getByRole('button', { name: `Remove Passwords (${bundleId})` })).toBeTruthy();
    }
    expect(screen.queryByText('com.apple.keychainaccess')).toBeNull();
  });

  it('moves retention help to the control and preserves the deletion warning at confirmation', async () => {
    await openSettings();
    const control = () => screen.getByRole('button', { name: /^Keep History For:/ });
    expect(screen.getByText('Estimated Storage')).toBeTruthy();
    expect(screen.queryByText('Based on when an item was last copied or reused.')).toBeNull();
    expect(control().getAttribute('aria-description')).toBe('Based on when an item was last copied or reused.');
    fireEvent.click(control());
    fireEvent.click(screen.getByRole('option', { name: '7 days' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('permanently removes 4 unprotected clipboard items');
    expect(dialog.textContent).toContain('Favorites, pinned items, and the system clipboard are unaffected.');
    expect(document.activeElement?.textContent).toBe('Cancel');
    expect(calls('clipboard_cleanup_preview')[0][1]).toEqual({ policy: { days: 7, limit: null } });
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(control().textContent).toBe('Forever');
    expect(document.activeElement).toBe(control());
    expect(calls('clipboard_cleanup_apply')).toHaveLength(0);
  });

  it('saves a retention policy without confirmation when the preview affects no items', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_cleanup_preview') return { count: 0, token: 'empty-preview' };
      if (name === 'clipboard_cleanup_apply') return { removed: 0, mediaCleanupFailed: false };
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'History Limit: Unlimited' }));
    fireEvent.click(screen.getByRole('option', { name: '500' }));
    await screen.findByRole('button', { name: 'History Limit: 500' });
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(calls('clipboard_cleanup_apply')[0][1]).toEqual({ policy: { days: null, limit: 500 }, token: 'empty-preview' });
  });

  it('applies only the cleanup that was previewed and refreshes the storage estimate', async () => {
    await openSettings();
    fireEvent.click(screen.getByRole('button', { name: 'Clear Unprotected History…' }));
    await screen.findByRole('alertdialog');
    expect(calls('clipboard_cleanup_apply')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Clear History' }));
    await screen.findByText('Removed 4 clipboard items.');
    expect(calls('clipboard_cleanup_apply')[0][1]).toEqual({ policy: null, token: 'preview-token' });
    expect(calls('clipboard_storage_info')).toHaveLength(2);
  });

  it('requires another preview if the history changed and never retries deletion on its own', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_cleanup_apply') throw 'clipboard_cleanup_changed';
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Clear Unprotected History…' }));
    await screen.findByRole('alertdialog');
    fireEvent.click(screen.getByRole('button', { name: 'Clear History' }));
    await screen.findByText(/Clipboard history changed/);
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(calls('clipboard_cleanup_preview')).toHaveLength(1);
    expect(calls('clipboard_cleanup_apply')).toHaveLength(1);
  });

  it('saves Copy Only as the row activation behavior', async () => {
    await openSettings();
    fireEvent.click(screen.getByRole('button', { name: 'When Activating an Item: Paste into Previous App' }));
    fireEvent.click(screen.getByRole('option', { name: 'Copy Only' }));
    await screen.findByRole('button', { name: 'When Activating an Item: Copy Only' });
    expect(calls('clipboard_behavior_set')[0][1]).toEqual({ activation: 'copy' });
  });
});
