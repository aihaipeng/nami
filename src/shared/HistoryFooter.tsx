import { useEffect, useRef, useState, type RefObject } from 'react';
import type { ViewProps } from '../App';
import { clipboardPanelShortcuts, clipboardShortcutAvailable } from './clipboardShortcuts';
import { translationShortcutEnabled } from './contracts';
import { keycapLabels, matchesBinding, recordWindowActions, sameBinding, shortcutTitles } from './shortcuts';
import type { ClipboardKind } from '../clipboard/useClipboard';
import { HistoryIcon } from '../translation/HistoryIcon';

export function HistoryFooter({ t, snapshot, command, native, active, search, total, clipboard = false, favoritesOnly = false, onFavorites, kind = null, onKindChange, activation, clipboardKeys }: Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> & {
  active: boolean; search: RefObject<HTMLInputElement | null>; total: number; clipboard?: boolean; favoritesOnly?: boolean; onFavorites?: () => void;
  kind?: ClipboardKind | null; onKindChange?: (kind: ClipboardKind | null) => void; activation?: 'copy' | 'paste'; clipboardKeys?: { label: string; keys: string }[];
}) {
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
  const { capabilities, translationPreferences: preferences, window: state } = snapshot;
  const localShortcuts = clipboard ? clipboardPanelShortcuts(snapshot).filter(({ action, enabled, globalConflict }) =>
    enabled && !globalConflict && clipboardShortcutAvailable(snapshot, native, action)) : [];
  const shortcuts = recordWindowActions.flatMap((action) => {
    const binding = preferences.shortcuts[action];
    // Older clipboard bindings could overlap the translation window's pin shortcut.
    return binding && translationShortcutEnabled(preferences, action) && !localShortcuts.some((local) => local.binding && sameBinding(local.binding, binding))
      ? [{ action, binding }] : [];
  });
  useEffect(() => {
    if (!active) return;
    const keydown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.key === 'Dead') return;
      const shortcut = shortcuts.find(({ binding }) => matchesBinding(binding, event));
      if (!shortcut) return;
      event.preventDefault();
      if (event.repeat) return;
      if (shortcut.action === 'pin') void command('window_pin', { region: 'history', pinned: !state.historyPinned });
      else if (shortcut.action === 'focusSearch') { setOpen(false); search.current?.focus({ preventScroll: true }); }
      else { setOpen((shown) => !shown); button.current?.focus({ preventScroll: true }); }
    };
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [active, command, search, shortcuts, state.historyPinned]);

  return <footer ref={footer} className="history-footer" onKeyDown={(event) => {
    if (open && event.key === 'Escape' && !event.nativeEvent.isComposing) { event.preventDefault(); event.stopPropagation(); setOpen(false); button.current?.focus(); }
  }}>
    <span className="history-record-count">{total} {total === 1 ? 'record' : 'records'}</span>
    <div className="history-filters">
      {onFavorites && <button type="button" className={`icon-button history-favorites${favoritesOnly ? ' record-favorite' : ''}`} aria-label={clipboard ? 'Favorite clipboard records' : 'Favorite history'} title="Favorites" aria-pressed={favoritesOnly} onClick={onFavorites}><HistoryIcon name="star" /></button>}
      {onKindChange && <span className="history-kinds">{(['text', 'image', 'files'] as const).map((value) => <button key={value} type="button" className="icon-button history-kind" aria-label={`${value === 'text' ? 'Text' : value === 'image' ? 'Image' : 'File'} clipboard records`} title={value === 'text' ? 'Text' : value === 'image' ? 'Images' : 'Files'} aria-pressed={kind === value} onClick={() => onKindChange(kind === value ? null : value)}><HistoryIcon name={value} /></button>)}</span>}
    </div>
    <button ref={button} type="button" className="icon-button history-shortcuts" aria-label="Keyboard shortcuts" title="Keyboard shortcuts" aria-expanded={open} onClick={() => setOpen(!open)}><HistoryIcon name="keyboard" /></button>
    {open && <div className="history-shortcuts-panel floating-panel" role="dialog" aria-label="Keyboard shortcuts">
      <strong>Keyboard shortcuts</strong>
      <div><span>Navigate {clipboard ? 'records' : 'history'}</span><span>↑ / ↓</span></div>
      <div><span>{clipboard ? activation === 'copy' ? 'Copy selection' : 'Paste selection' : 'Open selection'}</span><span>Return</span></div>
      {clipboardKeys?.map(({ label, keys }) => <div key={label}><span>{label}</span><span>{keys}</span></div>)}
      {shortcuts.map(({ action, binding }) => <div key={action}><span>{t(shortcutTitles[action])}</span><span>{keycapLabels(binding, capabilities.platform === 'macos').join(' ')}</span></div>)}
      <div><span>Hide window</span><span>Esc / {capabilities.primaryModifier === 'meta' ? '⌘' : 'Ctrl'} W</span></div>
      {!clipboard && <small>Select a record to navigate</small>}
    </div>}
  </footer>;
}
