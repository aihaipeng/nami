// @vitest-environment jsdom
import { fireEvent, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { mount } from './harness';

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
