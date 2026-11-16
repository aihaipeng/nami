import type { CSSProperties } from 'react';
import noteIcon from '../assets/history/reference/note.svg?url';

const symbols = import.meta.glob<string>('../assets/history/*.png', { eager: true, query: '?url', import: 'default' });
type HistorySymbol = 'note' | 'keyboard' | 'clock' | 'list.bullet' | 'star' | 'star.fill' | 'pin' | 'pin.fill' | 'languages' | 'doc.on.doc' | 'trash' | 'magnifyingglass' | 'xmark.circle.fill' | 'chevron.left' | 'chevron.right' | 'text' | 'image' | 'files' | 'ocr';
const reference: Partial<Record<HistorySymbol, string>> = {
  text: 'reference-text',
  image: 'reference-image',
  files: 'reference-file',
  ocr: 'reference-ocr',
  clock: 'reference-clock',
  keyboard: 'reference-keyboard',
  'list.bullet': 'reference-list',
  trash: 'reference-trash',
  magnifyingglass: 'reference-search',
  'xmark.circle.fill': 'reference-clear',
  languages: 'reference-languages',
  'doc.on.doc': 'reference-copy',
  star: 'reference-star',
  'star.fill': 'reference-star-filled',
  pin: 'reference-pin',
  'pin.fill': 'reference-pin-filled',
};

export function HistoryIcon({ name }: { name: HistorySymbol }) {
  return <span className="history-symbol" data-symbol={name} aria-hidden="true"
    style={{ '--symbol-url': `url("${(name === 'note' ? noteIcon : symbols[`../assets/history/${reference[name] ?? name}.png`])}")` } as CSSProperties} />;
}
