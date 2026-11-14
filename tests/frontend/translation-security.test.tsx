// @vitest-environment jsdom
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount } from './harness';

const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
const submit = (text: string) => {
  fireEvent.change(source(), { target: { value: text } });
  fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
};
describe('translation security', () => {
  it('clears old output and does not translate a non-text capture', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    await mount(); submit('Prior synthetic text');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'selection_take' ? Promise.resolve({ text: '', sourceApp: null, error: null }) : invoke(name, args));
    await emit('selection_ready', null);
    expect(source().value).toBe('');
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('');
    expect(calls('translation_run')).toHaveLength(1);
  });

  it('does not submit after a clipboard capture or restoration failure', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    await mount();
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'selection_take' ? Promise.resolve({ text: '', error: 'clipboard_restore_failed' }) : invoke(name, args));
    await emit('selection_ready', null);
    expect(source().value).toBe('');
    expect(screen.getAllByRole('alert')[0].textContent).toContain('restore the clipboard');
    expect(calls('translation_run')).toHaveLength(0);
  });

  it('shows an actionable permission error and sends no text when AX access is denied', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    await mount();
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'selection_take' ? Promise.resolve({ text: '', error: 'permission_required' }) : invoke(name, args));
    await emit('selection_ready', null);
    expect(screen.getAllByRole('alert')[0].textContent).toContain('Accessibility');
    expect(calls('translation_run')).toHaveLength(0);
  });
});
