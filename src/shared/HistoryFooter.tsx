import { useEffect, useEffectEvent, useRef, useState, type RefObject } from 'react';
import type { ViewProps } from '../App';
import { clipboardPanelShortcuts, clipboardShortcutAvailable, clipboardShortcutLabels, sharedRecordActions } from './clipboardShortcuts';
import { translationShortcutEnabled } from './contracts';
import { editsText, globalTranslationActions, keycapLabels, matchesBinding, recordWindowActions, sameBinding, shortcutTitles, translationWindowActions } from './shortcuts';
import type { ClipboardKind } from '../clipboard/useClipboard';
import { HistoryIcon } from '../translation/HistoryIcon';

export function HistoryFooter({ t, snapshot, command, native, active, search, total, clipboard = false, favoritesOnly = false, onFavorites, kind = null, onKindChange, activation }: Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> & {
  active: boolean; search: RefObject<HTMLInputElement | null>; total: number; clipboard?: boolean; favoritesOnly?: boolean; onFavorites?: () => void;
  kind?: ClipboardKind | null; onKindChange?: (kind: ClipboardKind | null) => void; activation?: 'copy' | 'paste';
}) {
  const [open, setOpen] = useState(false);
  const [announcement, setAnnouncement] = useState('');
  const footer = useRef<HTMLElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const previousFilter = useRef<{ favoritesOnly: boolean; kind: ClipboardKind | null } | null>(null);
  const filterAnnounce = useRef(false);
  useEffect(() => {
    const previous = previousFilter.current;
    previousFilter.current = { favoritesOnly, kind };
    if (!active) { filterAnnounce.current = false; setAnnouncement(''); return; }
    if (previous && (previous.favoritesOnly !== favoritesOnly || previous.kind !== kind)) filterAnnounce.current = true;
    if (!filterAnnounce.current) return;
    const filterName = favoritesOnly ? 'Favorites' : kind === 'text' ? 'Text' : kind === 'image' ? 'Images' : kind === 'files' ? 'Files' : 'All records';
    setAnnouncement(`${filterName}, ${total} ${total === 1 ? 'record' : 'records'}`);
  }, [active, favoritesOnly, kind, total]);
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
  const localShortcuts = clipboardPanelShortcuts(snapshot).filter(({ action, enabled, globalConflict, binding }) =>
    binding && enabled && !globalConflict && (clipboard || sharedRecordActions.some((shared) => shared === action)) && clipboardShortcutAvailable(snapshot, native, action));
  const shortcuts = recordWindowActions.flatMap((action) => {
    const binding = preferences.shortcuts[action];
    // Older clipboard bindings could overlap the translation window's pin shortcut.
    return binding && translationShortcutEnabled(preferences, action) && !localShortcuts.some((local) => local.binding && sameBinding(local.binding, binding))
      ? [{ action, binding }] : [];
  });
  const contextualShortcuts = clipboard
    ? snapshot.clipboardShortcutEnabled && snapshot.clipboardBinding && capabilities.selection && native && !snapshot.storageError
      ? [{ label: 'clipboardOpenHistory' as const, binding: snapshot.clipboardBinding }] : []
    : [...(capabilities.selection ? globalTranslationActions : []), ...translationWindowActions].flatMap((action) => {
      const binding = preferences.shortcuts[action];
      return binding && translationShortcutEnabled(preferences, action)
        ? [{ label: shortcutTitles[action], binding }] : [];
    });
  const keyLabel = (binding: Parameters<typeof keycapLabels>[0]) => keycapLabels(binding, capabilities.platform === 'macos', t).join(' ');
  // Registered once per activation. Holding Command re-renders the panel during the same key dispatch, and a
  // listener removed mid-dispatch never runs for that key in WebKit.
  const keydown = useEffectEvent((event: KeyboardEvent) => {
    if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.key === 'Dead' || editsText(event)) return;
    const target = event.target;
    if (target instanceof HTMLElement && target !== document.body && !footer.current?.parentElement?.contains(target)) return;
    const shortcut = shortcuts.find(({ action, binding }) => action !== 'openSettings' && matchesBinding(binding, event));
    if (!shortcut) return;
    event.preventDefault();
    if (event.repeat) return;
    if (shortcut.action === 'pin') void command('window_pin', { region: 'history', pinned: !state.historyPinned });
    else if (shortcut.action === 'focusSearch') {
      setOpen(false);
      search.current?.focus({ preventScroll: true });
      search.current?.select();
    }
    else { setOpen((shown) => !shown); button.current?.focus({ preventScroll: true }); }
  });
  useEffect(() => {
    if (!active) return;
    window.addEventListener('keydown', keydown, true);
    return () => window.removeEventListener('keydown', keydown, true);
  }, [active]);

  return <footer ref={footer} className="history-footer" onKeyDown={(event) => {
    if (open && event.key === 'Escape' && !event.nativeEvent.isComposing) { event.preventDefault(); event.stopPropagation(); setOpen(false); button.current?.focus(); }
  }}>
    <span className="history-record-count">{total} {total === 1 ? 'record' : 'records'}</span>
    <span className="sr-only" aria-live="polite" aria-atomic="true">{announcement}</span>
    <div className="history-filters" role="group" aria-label="Filter records">
      {onFavorites && <button type="button" className={`icon-button history-favorites${favoritesOnly ? ' record-favorite' : ''}`} aria-label={clipboard ? 'Favorite clipboard records' : 'Favorite history'} title="Favorites" aria-pressed={favoritesOnly} onClick={onFavorites}><HistoryIcon name="star" /></button>}
      {onKindChange && <span className="history-kinds">{(['text', 'image', 'files'] as const).map((value) => <button key={value} type="button" className="icon-button history-kind" aria-label={`${value === 'text' ? 'Text' : value === 'image' ? 'Image' : 'File'} clipboard records`} title={value === 'text' ? 'Text' : value === 'image' ? 'Images' : 'Files'} aria-pressed={kind === value} onClick={() => onKindChange(kind === value ? null : value)}><HistoryIcon name={value} /></button>)}</span>}
    </div>
    <button ref={button} type="button" className="icon-button history-shortcuts" aria-label={t('hotKey')} title={t('hotKey')} aria-expanded={open} onClick={() => setOpen(!open)}><HistoryIcon name="keyboard" /></button>
    {open && <div className="history-shortcuts-panel floating-panel" role="dialog" aria-label={t('hotKey')}>
      <strong>{t('hotKey')}</strong>
      <div><span>{clipboard ? activation === 'copy' ? 'Copy selection' : 'Paste selection' : 'Open selection'}</span><span>{keyLabel({ code: 'Enter', modifiers: [] })}</span></div>
      <div><span>{t('navigateRecords')}</span><span>↑ / ↓</span></div>
      <div><span>{t('switchCategory')}</span><span>← / →</span></div>
      {clipboard && native && !snapshot.storageError && capabilities.platform === 'macos' && capabilities.autoPaste && <div><span>{t('clipboardQuickPaste')}</span><span>⌘1–⌘5</span></div>}
      {localShortcuts.map(({ action, binding }) => binding && <div key={action}><span>{t(clipboardShortcutLabels[action])}</span><span>{keyLabel(binding)}</span></div>)}
      {shortcuts.map(({ action, binding }) => <div key={action}><span>{t(shortcutTitles[action])}</span><span>{keyLabel(binding)}</span></div>)}
      {contextualShortcuts.map(({ label, binding }) => <div key={label}><span>{t(label)}</span><span>{keyLabel(binding)}</span></div>)}
      <div><span>Hide window</span><span>Esc / {capabilities.primaryModifier === 'meta' ? '⌘' : 'Ctrl'} W</span></div>
    </div>}
  </footer>;
}
