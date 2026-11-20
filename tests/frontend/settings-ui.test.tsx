// @vitest-environment jsdom
import { fireEvent, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount } from './harness';

describe('Settings', () => {
  it('uses the same leading title and group structure across all four pages', async () => {
    await mount('settings');
    for (const page of ['General', 'Translation', 'Clipboard', 'Privacy & data']) {
      fireEvent.click(screen.getByRole('button', { name: page }));
      const region = screen.getByRole('region', { name: page });
      expect(within(region).getByRole('heading', { level: 1 })).toBe(region.querySelector('.settings-page-heading h1'));
      expect(region.querySelector('.settings-group')).not.toBeNull();
      expect(region.querySelector('.settings-page-heading p')).toBeNull();
    }
  });

  it('keeps the clipboard-based selection fallback off until the user opts in', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Translation' }));
    const fallback = screen.getByRole('checkbox', { name: 'Try copying when selected text is unavailable' }) as HTMLInputElement;
    expect(fallback.checked).toBe(false);
    expect(fallback.getAttribute('aria-description')).toContain('may be overwritten');
    fireEvent.click(screen.getByRole('button', { name: 'About the selection copy fallback' }));
    expect(screen.getByRole('tooltip').textContent).toContain('may be overwritten');
    fireEvent.click(fallback);
    expect(calls('translation_preferences_set').at(-1)?.[1]).toEqual({ selectionCopyFallback: true });
    await emit('translation_preferences_changed', { ...bridge.snapshot.translationPreferences, selectionCopyFallback: true });
    expect(fallback.checked).toBe(true);
  });

  it.each([
    ['Open Translation', 'translation_shortcut_set', 'record'],
    ['Preview', 'clipboard_record_shortcut_set', 'choose'],
    ['Open Clipboard History', 'clipboard_shortcut_set', 'record'],
  ])('explains why Command-W cannot replace the %s shortcut (%s, %s)', async (label, command, mode) => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const recorder = screen.getByRole('button', { name: `${label} shortcut` });
    const previous = recorder.textContent;
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name, args) => name === command
      ? Promise.reject('shortcut_reserved_close_window') : invoke(name, args));

    fireEvent.click(recorder);
    if (mode === 'choose') {
      fireEvent.click(screen.getByRole('button', { name: 'Choose Keys' }));
      const manual = within(screen.getByRole('dialog', { name: 'Choose Keys' }));
      fireEvent.change(manual.getByRole('combobox', { name: 'Key' }), { target: { value: 'KeyW' } });
      fireEvent.click(manual.getByRole('checkbox', { name: '⌘ Command' }));
      fireEvent.click(manual.getByRole('button', { name: 'Save' }));
    } else {
      fireEvent.keyDown(recorder, { key: 'w', code: 'KeyW', metaKey: true });
    }

    expect(await screen.findByText('⌘W is reserved for closing the current window.')).toBeTruthy();
    expect(calls(command)).toHaveLength(1);
    expect(calls(command)[0][1]).toMatchObject({ binding: { code: 'KeyW', modifiers: ['meta'] } });
    expect(recorder.textContent).toBe(previous);
  });

  it('dismisses a settings picker when its anchor scrolls and returns focus without closing on menu scroll', async () => {
    await mount('settings');
    const button = screen.getByRole('button', { name: /^Interface language:/i });
    fireEvent.click(button);
    const options = screen.getByRole('listbox', { name: 'Interface language' });
    fireEvent.scroll(options);
    expect(button.getAttribute('aria-expanded')).toBe('true');
    fireEvent.scroll(screen.getByRole('region', { name: 'General' }));
    expect(screen.queryByRole('listbox', { name: 'Interface language' })).toBeNull();
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(button);
  });
});
