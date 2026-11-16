import { useEffect, useRef, useState } from 'react';
import { HistoryIcon } from '../translation/HistoryIcon';

export function HistoryFooter({ total, clipboard = false, favoritesOnly = false, onFavorites, primaryModifier }: { total: number; clipboard?: boolean; favoritesOnly?: boolean; onFavorites?: () => void; primaryModifier: 'meta' | 'control' }) {
  const [open, setOpen] = useState(false);
  const footer = useRef<HTMLElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!footer.current?.contains(event.target as Node)) setOpen(false); };
    const dismiss = () => setOpen(false);
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('blur', dismiss);
    window.addEventListener('resize', dismiss);
    window.addEventListener('nami:window-moved', dismiss);
    return () => { document.removeEventListener('pointerdown', outside, true); window.removeEventListener('blur', dismiss); window.removeEventListener('resize', dismiss); window.removeEventListener('nami:window-moved', dismiss); };
  }, [open]);
  return <footer ref={footer} className="history-footer" onKeyDown={(event) => {
    if (open && event.key === 'Escape' && !event.nativeEvent.isComposing) { event.preventDefault(); event.stopPropagation(); setOpen(false); button.current?.focus(); }
  }}>
    <span className="history-record-count">{total} {total === 1 ? 'record' : 'records'}</span>
    {onFavorites && <button type="button" className="icon-button history-favorites" aria-label={clipboard ? 'Favorite clipboard records' : 'Favorite history'} title="Favorites" aria-pressed={favoritesOnly} onClick={onFavorites}><HistoryIcon name={favoritesOnly ? 'star.fill' : 'star'} /></button>}
    <button ref={button} type="button" className="icon-button history-shortcuts" aria-label="Keyboard shortcuts" title="Keyboard shortcuts" aria-expanded={open} onClick={() => setOpen(!open)}><HistoryIcon name="keyboard" /></button>
    {open && <div className="history-shortcuts-panel floating-panel" role="dialog" aria-label="Keyboard shortcuts">
      <strong>Keyboard shortcuts</strong>
      <div><span>Navigate {clipboard ? 'records' : 'history'}</span><span>↑ / ↓</span></div>
      <div><span>{clipboard ? 'Paste selection' : 'Open selection'}</span><span>Return</span></div>
      <div><span>Hide window</span><span>Esc / {primaryModifier === 'meta' ? '⌘' : 'Ctrl'} W</span></div>
      <small>Select a record to navigate</small>
    </div>}
  </footer>;
}
