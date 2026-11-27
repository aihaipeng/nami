import { useEffect, useEffectEvent, useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import type { ViewProps } from '../App';
import { clipboardPanelShortcuts, clipboardShortcutAvailable, clipboardShortcutLabels, sharedRecordActions } from './clipboardShortcuts';
import { translationShortcutEnabled } from './contracts';
import { editsText, globalTranslationActions, keycapLabels, matchesBinding, recordWindowActions, sameBinding, shortcutTitles, translationWindowActions } from './shortcuts';
import type { ClipboardKind } from '../clipboard/useClipboard';
import { HistoryIcon } from '../translation/HistoryIcon';
import { RecordShortcutHint } from './RecordShortcutHint';

function ShortcutsPopover({ label, children }: { label: string; children: ReactNode }) {
  const scrollId = useId();
  const scrollRef = useRef<HTMLElement>(null);
  const trackRef = useRef<HTMLSpanElement>(null);
  const [scrollbarVisible, setScrollbarVisible] = useState(false);
  const [scroll, setScroll] = useState({ top: 0, height: 0, content: 0, track: 0 });
  const idle = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const dragging = useRef(false);
  const focused = useRef(false);
  const hovered = useRef(false);
  const revealedOnOpen = useRef(false);
  const dragCleanup = useRef<(() => void) | undefined>(undefined);

  function showScrollbar() {
    window.clearTimeout(idle.current);
    setScrollbarVisible(true);
    idle.current = window.setTimeout(() => {
      if (!dragging.current && !focused.current && !hovered.current) setScrollbarVisible(false);
    }, 1000);
  }

  function measure() {
    const element = scrollRef.current;
    if (!element) return;
    const height = element.clientHeight;
    const content = element.scrollHeight;
    const maximum = Math.max(0, content - height);
    const track = trackRef.current?.clientHeight || Math.max(0, height - 16);
    const next = { top: Math.max(0, Math.min(maximum, element.scrollTop)), height, content, track };
    setScroll((old) => (old.top === next.top && old.height === next.height && old.content === next.content && old.track === next.track ? old : next));
    if (maximum > 0 && !revealedOnOpen.current) {
      revealedOnOpen.current = true;
      showScrollbar();
    }
  }
  const measureRef = useRef(measure);
  measureRef.current = measure;

  useEffect(() => () => { window.clearTimeout(idle.current); dragCleanup.current?.(); }, []);
  useLayoutEffect(() => { measureRef.current(); }, [children]);
  useLayoutEffect(() => {
    const observer = new ResizeObserver(() => measureRef.current());
    if (scrollRef.current) observer.observe(scrollRef.current);
    if (trackRef.current) observer.observe(trackRef.current);
    return () => observer.disconnect();
  }, [scroll.content > scroll.height]);

  const maximum = Math.max(0, scroll.content - scroll.height);
  const thumb = Math.min(scroll.track, Math.max(24, scroll.track * scroll.height / Math.max(1, scroll.content)));
  const travel = Math.max(0, scroll.track - thumb);
  function move(top: number) {
    const clamped = Math.max(0, Math.min(maximum, top));
    scrollRef.current?.scrollTo({ top: clamped });
    measure();
  }

  return <div className="history-shortcuts-panel floating-panel" role="dialog" aria-label={label}>
    <section
      ref={scrollRef}
      id={scrollId}
      className="history-shortcuts-scroll"
      data-scrollable={maximum > 0}
      onScroll={() => { measure(); showScrollbar(); }}
      onWheel={showScrollbar}
      onTouchMove={showScrollbar}
    >
      {children}
    </section>
    {maximum > 0 && <span
      ref={trackRef}
      className={`history-scrollbar${scrollbarVisible ? ' visible' : ''}`}
      role="scrollbar"
      aria-label={`Scroll ${label}`}
      aria-controls={scrollId}
      aria-orientation="vertical"
      aria-valuemin={0}
      aria-valuemax={maximum}
      aria-valuenow={Math.round(scroll.top)}
      tabIndex={0}
      onPointerEnter={() => { hovered.current = true; showScrollbar(); }}
      onPointerMove={showScrollbar}
      onPointerLeave={() => { hovered.current = false; showScrollbar(); }}
      onFocus={() => { focused.current = true; showScrollbar(); }}
      onBlur={() => { focused.current = false; showScrollbar(); }}
      onKeyDown={(event) => {
        const delta = event.key === 'ArrowDown' ? 40 : event.key === 'ArrowUp' ? -40 : event.key === 'PageDown' ? scroll.height : event.key === 'PageUp' ? -scroll.height : 0;
        if (delta || event.key === 'Home' || event.key === 'End') {
          showScrollbar();
          event.preventDefault();
          event.stopPropagation();
          move(event.key === 'Home' ? 0 : event.key === 'End' ? maximum : scroll.top + delta);
        }
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        dragging.current = true; showScrollbar();
        const track = event.currentTarget;
        track.setPointerCapture(event.pointerId);
        const startY = event.clientY;
        const onThumb = (event.target as HTMLElement).classList.contains('history-scrollbar-thumb');
        const startTop = onThumb ? scroll.top : Math.max(0, Math.min(maximum, (event.clientY - track.getBoundingClientRect().top - thumb / 2) / Math.max(1, travel) * maximum));
        move(startTop);
        const drag = (next: PointerEvent) => { if (next.pointerId === event.pointerId) { showScrollbar(); move(startTop + (next.clientY - startY) / Math.max(1, travel) * maximum); } };
        const cleanup = () => { track.removeEventListener('pointermove', drag); track.removeEventListener('lostpointercapture', end); dragCleanup.current = undefined; };
        const end = () => { dragging.current = false; showScrollbar(); cleanup(); };
        track.addEventListener('pointermove', drag);
        track.addEventListener('lostpointercapture', end);
        dragCleanup.current = cleanup;
      }}
    >
      <span className="history-scrollbar-thumb" style={{ height: thumb, transform: `translateY(${maximum ? scroll.top / maximum * travel : 0}px)` }} />
    </span>}
  </div>;
}

export function HistoryFooter({ t, snapshot, command, native, active, search, total, clipboard = false, favoritesOnly = false, onFavorites, kind = null, onKindChange, shortcutsHint }: Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> & {
  active: boolean; search: RefObject<HTMLInputElement | null>; total: number; clipboard?: boolean; favoritesOnly?: boolean; onFavorites?: () => void;
  kind?: ClipboardKind | null; onKindChange?: (kind: ClipboardKind | null) => void; shortcutsHint?: string;
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
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.isComposing || event.keyCode === 229) return;
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      if (event.target instanceof Node && footer.current?.contains(event.target)) button.current?.focus();
    };
    const dismiss = () => setOpen(false);
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('keydown', escape, true);
    window.addEventListener('blur', dismiss);
    window.addEventListener('resize', dismiss);
    window.addEventListener('nami:window-moved', dismiss);
    return () => { document.removeEventListener('pointerdown', outside, true); window.removeEventListener('keydown', escape, true); window.removeEventListener('blur', dismiss); window.removeEventListener('resize', dismiss); window.removeEventListener('nami:window-moved', dismiss); };
  }, [open]);
  const { capabilities, translationPreferences: preferences, window: state } = snapshot;
  const panelShortcuts = clipboardPanelShortcuts(snapshot);
  const localShortcuts = panelShortcuts.filter(({ action, enabled, globalConflict, binding }) =>
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
  const keyLabels = (binding: Parameters<typeof keycapLabels>[0]) => keycapLabels(binding, capabilities.platform === 'macos', t);
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
    if (shortcut.action === 'pin') void command('window_pin', { region: 'clipboard', pinned: !state.clipboardPinned });
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

  return <footer ref={footer} className="history-footer" onMouseDown={(event) => {
    if ((event.target as HTMLElement).closest('button')) event.preventDefault();
  }} onKeyDown={(event) => {
    if (open && event.key === 'Escape' && !event.nativeEvent.isComposing) { event.preventDefault(); event.stopPropagation(); setOpen(false); button.current?.focus(); }
  }}>
    <span className="history-record-count">{total} {total === 1 ? 'record' : 'records'}</span>
    <span className="sr-only" aria-live="polite" aria-atomic="true">{announcement}</span>
    <div className="history-filters" role="group" aria-label="Filter records">
      {onKindChange && <button type="button" className="icon-button history-all" aria-label="All clipboard records" title="All" aria-pressed={!favoritesOnly && kind === null} onClick={() => { if (favoritesOnly || kind !== null) onKindChange(null); }}><HistoryIcon name="list.bullet" /></button>}
      {onFavorites && <button type="button" className={`icon-button history-favorites${favoritesOnly ? ' record-favorite' : ''}`} aria-label={clipboard ? 'Favorite clipboard records' : 'Favorite history'} title="Favorites" aria-pressed={favoritesOnly} onClick={onFavorites}><HistoryIcon name="star" /></button>}
      {onKindChange && <span className="history-kinds">{(['text', 'image', 'files'] as const).map((value) => <button key={value} type="button" className="icon-button history-kind" aria-label={`${value === 'text' ? 'Text' : value === 'image' ? 'Image' : 'File'} clipboard records`} title={value === 'text' ? 'Text' : value === 'image' ? 'Images' : 'Files'} aria-pressed={kind === value} onClick={() => onKindChange(kind === value ? null : value)}><HistoryIcon name={value} /></button>)}</span>}
    </div>
    <button ref={button} type="button" className="icon-button history-shortcuts" aria-label={t('hotKey')} title={t('hotKey')} aria-expanded={open} aria-keyshortcuts={shortcutsHint ? `Meta+${shortcutsHint}` : undefined} onClick={() => setOpen(!open)}>{shortcutsHint ? <span className="history-command-hint" aria-hidden="true">{shortcutsHint}</span> : <HistoryIcon name="keyboard" />}</button>
    {open && <ShortcutsPopover label={t('hotKey')}>
      <strong>{t('hotKey')}</strong>
      <div><span>{clipboard ? 'Paste selection' : 'Open selection'}</span><RecordShortcutHint keys={keyLabels({ code: 'Enter', modifiers: [] })} /></div>
      <div><span>{t('navigateRecords')}</span><RecordShortcutHint keys={['↑ / ↓']} /></div>
      {clipboard && <div><span>{t('switchCategory')}</span><RecordShortcutHint keys={['← / →']} /></div>}
      {clipboard && native && !snapshot.storageError && capabilities.platform === 'macos' && capabilities.autoPaste && <div><span>{t('clipboardQuickPaste')}</span><RecordShortcutHint keys={['⌘', '1～5']} /></div>}
      {localShortcuts.map(({ action, binding }) => binding && <div key={action}><span>{t(clipboardShortcutLabels[action])}</span><RecordShortcutHint keys={keyLabels(binding)} /></div>)}
      {shortcuts.map(({ action, binding }) => <div key={action}><span>{t(shortcutTitles[action])}</span><RecordShortcutHint keys={keyLabels(binding)} /></div>)}
      {contextualShortcuts.map(({ label, binding }) => <div key={label}><span>{t(label)}</span><RecordShortcutHint keys={keyLabels(binding)} /></div>)}
      <div><span>Hide window</span><RecordShortcutHint keys={clipboard ? ['Esc'] : ['Esc /', capabilities.primaryModifier === 'meta' ? '⌘' : 'Ctrl', 'W']} /></div>
    </ShortcutsPopover>}
  </footer>;
}
