import type { CSSProperties } from 'react';

const symbols = import.meta.glob<string>('../assets/history/*.png', { eager: true, query: '?url', import: 'default' });
type HistorySymbol = 'clock' | 'list.bullet' | 'star' | 'star.fill' | 'pin' | 'pin.fill' | 'arrow.clockwise' | 'doc.on.doc' | 'trash' | 'magnifyingglass' | 'xmark.circle.fill';
const reference: Partial<Record<HistorySymbol, string>> = {
  clock: 'reference-clock',
  'list.bullet': 'reference-list',
  trash: 'reference-trash',
  magnifyingglass: 'reference-search',
  'xmark.circle.fill': 'reference-clear',
  'arrow.clockwise': 'reference-refresh',
  'doc.on.doc': 'reference-copy',
  star: 'reference-star',
  'star.fill': 'reference-star-filled',
  pin: 'reference-pin',
  'pin.fill': 'reference-pin-filled',
};

export function HistoryIcon({ name }: { name: HistorySymbol }) {
  return <span className="history-symbol" data-symbol={name} aria-hidden="true"
    style={{ '--symbol-url': `url("${symbols[`../assets/history/${reference[name] ?? name}.png`]}")` } as CSSProperties} />;
}
