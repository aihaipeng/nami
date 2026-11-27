// @vitest-environment jsdom
import { fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, mount, windowState } from './harness';
import { defaultTranslationShortcuts } from '../../src/shared/shortcuts';

function portable(platform: 'windows' | 'linux' = 'windows') {
  bridge.snapshot.capabilities = {
    platform, nativeHistory: false, nativeChrome: false, nativeMaterial: false,
    selection: false, imageOcr: false, appleTranslation: false, clipboardWrite: false, autoPaste: false, clipboardOpenTarget: false, clipboardDrag: false, localBackup: false, storageLocations: false, launchAtLogin: false, accessibilityPermission: false,
    primaryModifier: 'control',
  };
  bridge.snapshot.translationPreferences = { ...bridge.snapshot.translationPreferences, shortcuts: defaultTranslationShortcuts('control') };
}

function portableClipboard(platform: 'windows' | 'linux' = 'windows') {
  portable(platform);
  bridge.snapshot.window.standaloneTranslationVisible = false;
  bridge.snapshot.window.clipboardVisible = true;
  bridge.snapshot.window.clipboardDetailVisible = true;
  bridge.snapshot.window.clipboardExtent = bridge.snapshot.window.clipboardWidth;
  bridge.snapshot.window.rightExtent = bridge.snapshot.window.clipboardDetailWidth;
  const invoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation((name: string, args?: unknown) => {
    if (name === 'clipboard_query') {
      const page = {
        records: [{ id: 'clip-1', kind: 'text', title: '', text: 'Portable clip', date: '2026-04-08T10:00:00Z', isFavorite: false, isPinned: false, sourceAppId: null }],
        sources: {}, totalRecords: 1, hasMore: false, cursor: null,
      };
      return Promise.resolve({ page, alternate: { ...page, records: [], totalRecords: 0 } });
    }
    if (name === 'clipboard_record_detail') {
      return Promise.resolve({
        id: 'clip-1', kind: 'text', text: 'Portable clip', date: '2026-04-08T10:00:00Z', isFavorite: false, isPinned: false,
        width: null, height: null, size: 13, characterCount: 13, files: [],
      });
    }
    return invoke(name, args);
  });
}

const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;

describe('shared clipboard host and platform capabilities', () => {
  it.each(['windows', 'linux'] as const)('renders portable clipboard list and details divider in a %s Tauri host', async (platform) => {
    portableClipboard(platform); await mount();
    await screen.findByRole('button', { name: 'Clipboard: Portable clip' });
    expect(screen.getByRole('separator', { name: 'Resize clipboard list' })).toBeTruthy();
    expect(calls('translation_chrome_update')).toHaveLength(0);
    expect(calls('selection_take')).toHaveLength(0);
    expect(document.documentElement.dataset.nativeMaterial).toBe('false');
    expect(document.documentElement.dataset.nativeHistory).toBe('false');
  });

  it('keeps the macOS native history path free of a duplicate React clipboard list in translation webview', async () => {
    await mount();
    expect(screen.queryByRole('region', { name: 'Clipboard' })).toBeNull();
    expect(document.documentElement.dataset.nativeMaterial).toBe('true');
    expect(document.documentElement.dataset.nativeHistory).toBe('true');
  });

  it('applies divider widths immediately and leaves visibility changes eligible for opening animation', async () => {
    portableClipboard(); await mount();
    const clip = document.querySelector<HTMLElement>('.history-clip')!;
    expect(clip.style.transition).toBe('');
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize clipboard list' }), { key: 'ArrowRight' });
    expect(calls('clipboard_width_set').at(-1)?.[1]).toEqual({ width: 384, initialize: false });
    await windowState({ clipboardWidth: 384, clipboardExtent: 384 });
    expect(clip.style.transition).toBe('none');
    await windowState({ clipboardWidth: 440, clipboardExtent: 440 });
    expect(clip.style.transition).toBe('none');
    await windowState({ clipboardVisible: false, clipboardExtent: 0 });
    expect(clip.style.transition).toBe('');
    await windowState({ clipboardVisible: true, clipboardExtent: 440 });
    expect(clip.style.transition).toBe('');
    expect(clip.hidden).toBe(false);
  });

  it('uses Control shortcuts without treating Super as Control', async () => {
    portable(); await mount();
    fireEvent.change(source(), { target: { value: 'Portable synthetic text' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    expect(calls('translation_run')).toHaveLength(0);
    fireEvent.keyDown(source(), { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(screen.queryByRole('button', { name: 'Translate' })).toBeNull();
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
});
