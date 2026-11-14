// @vitest-environment jsdom
import { fireEvent, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, flushFrames, mount, preferReducedMotion, releasePointerCapture, scrollTo, setPointerCapture, windowState } from './harness';
import { historySamples } from '../../src/translation/history';

const records = () => screen.queryAllByRole('button', { name: /^Translation:/ });
const row = (id: string) => {
  const record = historySamples.find((record) => record.id === id)!;
  return screen.getByRole('button', { name: `Translation: ${record.text.replace(/\s+/g, ' ').trim()}` }).closest('article')!;
};

describe('history list and session actions', () => {
  it('loads all 20 synthetic records with the pinned section separate', async () => {
    await mount('history');
    expect(records()).toHaveLength(20);
    expect(document.querySelectorAll('.history-pinned article')).toHaveLength(2);
    expect(document.querySelectorAll('.history-list article')).toHaveLength(18);
    expect(screen.queryByRole('textbox', { name: 'Source text' })).toBeNull();
    expect(screen.getByRole('button', { name: 'All history' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('searches cached translations and clears the query back to all records', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.type(screen.getByRole('searchbox'), '安静');
    expect(records()).toHaveLength(1);
    expect(records()[0].getAttribute('aria-label')).toContain(historySamples[6].text);
    await user.click(screen.getByRole('button', { name: 'Clear history search' }));
    expect(records()).toHaveLength(20);
    expect(document.activeElement).toBe(screen.getByRole('searchbox'));
  });

  it('treats search symbols literally and shows a useful empty state', async () => {
    await mount('history');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: '100%_literal' } });
    expect(records()).toHaveLength(1);
    expect(records()[0].getAttribute('aria-label')).toContain('Numbers and symbols');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'never present in these samples' } });
    expect(records()).toHaveLength(0);
    expect(screen.getByRole('status').textContent).toBe('No matching translations');
  });

  it('filters the five favorites, permits a session favorite change and restores All', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(screen.getByRole('button', { name: 'Favorite history' }));
    expect(records()).toHaveLength(5);
    const target = row('sample-04');
    await user.click(within(target).getByRole('button', { name: 'Remove favorite' }));
    expect(records()).toHaveLength(4);
    await user.click(screen.getByRole('button', { name: 'All history' }));
    expect(records()).toHaveLength(20);
    expect(within(row('sample-04')).getByRole('button', { name: 'Favorite' }).getAttribute('aria-pressed')).toBe('false');
    expect(historySamples.find(({ id }) => id === 'sample-04')!.isFavorite).toBe(true);
  });

  it('moves pinned records between independent lists without losing their source', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Pin record' }));
    expect(document.querySelectorAll('.history-pinned article')).toHaveLength(3);
    expect(row('sample-03').closest('.history-pinned')).toBeTruthy();
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Unpin record' }));
    expect(document.querySelectorAll('.history-pinned article')).toHaveLength(2);
    expect(row('sample-03').closest('.history-list')).toBeTruthy();
    expect(records()).toHaveLength(20);
  });

  it('uses FinderX SF Symbols and switches filled record flags without rotating record pins', async () => {
    const user = userEvent.setup(); await mount('history');
    expect(screen.getByRole('button', { name: 'All history' }).querySelector('[data-symbol="list.bullet"]')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Pin history region' }).classList.contains('history-pin')).toBe(true);
    const target = row('sample-03');
    expect(target.querySelector('[data-symbol="arrow.clockwise"]')).toBeTruthy();
    expect(target.querySelector('[data-symbol="doc.on.doc"]')).toBeTruthy();
    expect(target.querySelector('[data-symbol="trash"]')).toBeTruthy();
    await user.click(within(target).getByRole('button', { name: 'Favorite' }));
    expect(target.querySelector('[data-symbol="star.fill"]')).toBeTruthy();
    await user.click(within(target).getByRole('button', { name: 'Pin record' }));
    const pinned = row('sample-03');
    expect(pinned.classList.contains('pinned')).toBe(true);
    const pin = within(pinned).getByRole('button', { name: 'Unpin record' });
    expect(pin.querySelector('[data-symbol="pin.fill"]')).toBeTruthy();
    expect(pin.classList.contains('history-pin')).toBe(false);
    await user.click(pin);
    expect(row('sample-03').classList.contains('pinned')).toBe(false);
  });

  it('allows deleting only unprotected records and retains backend state on remount', async () => {
    const user = userEvent.setup(); const view = await mount('history');
    expect(within(row('sample-01')).queryByRole('button', { name: 'Delete history record' })).toBeNull();
    expect(within(row('sample-04')).queryByRole('button', { name: 'Delete history record' })).toBeNull();
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Delete history record' }));
    expect(records()).toHaveLength(19);
    view.unmount(); await mount('history');
    expect(records()).toHaveLength(19);
  });

  it('pins the history region through the native contract and reflects returned state', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(screen.getByRole('button', { name: 'Pin history region' }));
    expect(calls('window_pin').at(-1)).toEqual(['window_pin', { region: 'history', pinned: true }]);
    await windowState({ historyPinned: true });
    await user.click(screen.getByRole('button', { name: 'Unpin window' }));
    expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'history', pinned: false });
  });

  it('sends only a fixture ID for normal selection, retranslation and source copying', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    expect(calls('history_sample_select').at(-1)).toEqual(['history_sample_select', { sampleId: 'sample-03', retranslate: false }]);
    expect(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }).getAttribute('aria-pressed')).toBe('true');
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Retranslate' }));
    expect(calls('history_sample_select').at(-1)).toEqual(['history_sample_select', { sampleId: 'sample-03', retranslate: true }]);
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Copy source text' }));
    expect(calls('translation_sample_copy').at(-1)).toEqual(['translation_sample_copy', { sampleId: 'sample-03', part: 'source' }]);
  });

  it('retains query and session favorites while hidden and shown again', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.type(screen.getByRole('searchbox'), 'train');
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Favorite' }));
    await windowState({ historyVisible: false, historyExtent: 0 });
    expect(records()).toHaveLength(0);
    expect(document.querySelector('.history-view')?.hasAttribute('inert')).toBe(true);
    await windowState({ historyVisible: true, historyExtent: 360 });
    expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('train');
    expect(records()).toHaveLength(1);
    expect(within(row('sample-03')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy();
  });

  it('accepts cross-view selection events and clears highlight without erasing records', async () => {
    await mount('history');
    await emit('history_sample_selected', { sampleId: 'sample-03', retranslate: false });
    expect(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }).getAttribute('aria-pressed')).toBe('true');
    await emit('history_sample_selected', { sampleId: null, retranslate: false });
    expect(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }).getAttribute('aria-pressed')).toBe('false');
    expect(records()).toHaveLength(20);
  });

  it('dismisses with Escape or Command-W and suppresses dismissal during IME input', async () => {
    await mount('history'); const search = screen.getByRole('searchbox');
    fireEvent.keyDown(search, { key: 'Escape', isComposing: true });
    expect(calls('window_hide')).toHaveLength(0);
    fireEvent.keyDown(search, { key: 'Escape' });
    fireEvent.keyDown(search, { key: 'w', metaKey: true });
    expect(calls('window_hide')).toEqual([
      ['window_hide', { region: 'translation' }], ['window_hide', { region: 'translation' }],
    ]);
  });

  it('reveals a top-clipped record using only its own scroll region', async () => {
    const user = userEvent.setup(); await mount('history');
    const article = row('sample-03'); const list = article.closest<HTMLElement>('.history-list')!;
    list.scrollTop = 100;
    vi.spyOn(list, 'getBoundingClientRect').mockReturnValue({ top: 100, bottom: 500 } as DOMRect);
    vi.spyOn(article, 'getBoundingClientRect').mockReturnValue({ top: 80, bottom: 160 } as DOMRect);
    scrollTo.mockClear();
    await user.click(within(article).getByRole('button', { name: /^Translation:/ }));
    expect(scrollTo).toHaveBeenCalledWith({ top: 76, behavior: 'smooth' });
    expect(scrollTo.mock.contexts.at(-1)).toBe(list);
    expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-03', retranslate: false });
  });

  it('does not scroll a fully visible row and respects reduced motion for clipped rows', async () => {
    const user = userEvent.setup(); await mount('history');
    const article = row('sample-01'); const pinned = article.closest<HTMLElement>('.history-pinned')!;
    vi.spyOn(pinned, 'getBoundingClientRect').mockReturnValue({ top: 100, bottom: 300 } as DOMRect);
    const measure = vi.spyOn(article, 'getBoundingClientRect').mockReturnValue({ top: 110, bottom: 200 } as DOMRect);
    scrollTo.mockClear();
    await user.click(within(article).getByRole('button', { name: /^Translation:/ }));
    expect(scrollTo).not.toHaveBeenCalled();
    preferReducedMotion(); measure.mockReturnValue({ top: 250, bottom: 320 } as DOMRect);
    await user.click(within(article).getByRole('button', { name: /^Translation:/ }));
    expect(scrollTo).toHaveBeenCalledWith({ top: 24, behavior: 'instant' });
    expect(scrollTo.mock.contexts.at(-1)).toBe(pinned);
  });
});

describe('history divider interaction contract', () => {
  it.each([
    ['ArrowLeft', 336], ['ArrowRight', 384], ['Home', 280], ['End', 720],
  ])('resizes using %s to request width %i', async (key, expected) => {
    await mount('history');
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize translation history' }), { key });
    expect(calls('history_width_set').at(-1)).toEqual(['history_width_set', { width: expected, initialize: false }]);
  });

  it('uses screen coordinates despite moving local coordinates, and commits the final release width', async () => {
    await mount('history'); const divider = screen.getByRole('separator');
    fireEvent.pointerDown(divider, { button: 0, pointerId: 3, screenX: 500, clientX: 350 });
    expect(setPointerCapture).toHaveBeenCalledWith(3);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 560, clientX: 340 });
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 580, clientX: 325 });
    await flushFrames();
    expect(calls('history_width_set').at(-1)).toEqual(['history_width_set', { width: 440, initialize: false }]);
    fireEvent.pointerUp(divider, { pointerId: 3, screenX: 590, clientX: 320 });
    expect(calls('history_width_set').at(-1)).toEqual(['history_width_set', { width: 450, initialize: false }]);
    expect(releasePointerCapture).toHaveBeenCalledWith(3);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 650 }); await flushFrames();
    expect(calls('history_width_set')).toHaveLength(2);
  });

  it('cancels queued resizing when pointer capture is lost and ignores non-primary buttons', async () => {
    await mount('history'); const divider = screen.getByRole('separator');
    fireEvent.pointerDown(divider, { button: 2, pointerId: 3, screenX: 500 });
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 580 }); await flushFrames();
    expect(calls('history_width_set')).toHaveLength(0);
    fireEvent.pointerDown(divider, { button: 0, pointerId: 4, screenX: 500 });
    fireEvent.pointerMove(divider, { pointerId: 4, screenX: 580 });
    fireEvent.lostPointerCapture(divider, { pointerId: 4 }); await flushFrames();
    expect(calls('history_width_set')).toHaveLength(0);
  });

  it('responds immediately when reversing after reaching either width limit, as FinderX does', async () => {
    await mount('history'); const divider = screen.getByRole('separator');
    fireEvent.pointerDown(divider, { button: 0, pointerId: 3, screenX: 500 });
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 1000 }); await flushFrames();
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(720);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 990 }); await flushFrames();
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(710);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 0 }); await flushFrames();
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(280);
    fireEvent.pointerUp(divider, { pointerId: 3, screenX: 10 });
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(290);
  });

  it('hides the internal divider when only history is shown and retains records', async () => {
    await mount('history'); await windowState({ translationVisible: false, translationExtent: 0 });
    expect(screen.queryByRole('separator')).toBeNull(); expect(records()).toHaveLength(20);
  });
});
