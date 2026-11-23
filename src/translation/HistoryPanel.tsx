import { focusRecordList, useRecordPointer, useRecordsOpenFocus } from '../shared/recordFocus';
import { useEffect, useEffectEvent, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { HistoryIcon } from './HistoryIcon';
import { RecordToolbar } from '../shared/RecordToolbar';
import { useRecordBrowser } from '../shared/useRecordBrowser';
import type { ViewProps } from '../App';
import { cn } from '../lib/utils';
import { languageName } from '../shared/languages';
import { filterHistory, historyPreview, historyScrollTarget, type HistoryRecord } from './history';
import { RecordHeading } from '../shared/RecordHeading';
import { RecordActions } from '../shared/RecordActions';
import { useHistory } from './useHistory';
import { ActivityBeam } from '../shared/ActivityBeam';
import { HistoryList } from '../shared/HistoryList';
import { RecordMenu, recordMenuItems } from '../shared/RecordMenu';
import { HistoryFooter } from '../shared/HistoryFooter';
import { RecordFeedback } from '../shared/RecordFeedback';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { translationShortcutEnabled } from '../shared/contracts';
import { editsText, matchesBinding, sameBinding } from '../shared/shortcuts';
import { ClipboardEditor } from '../clipboard/ClipboardEditor';
import { clipboardGlobalBindings, clipboardKeyAction, clipboardPanelShortcuts, clipboardShortcutAvailable, sharedRecordActions } from '../shared/clipboardShortcuts';

interface Props extends Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> {
  selectedId: string | null;
  onSelect: (record: HistoryRecord, retranslate?: boolean, preview?: boolean) => void;
  onKeyDown: (event: KeyboardEvent) => void;
  active?: boolean;
  activity?: ViewProps['activity'];
}

export function HistoryPanel({ t, snapshot, command, native, selectedId, onSelect, onKeyDown, active = true }: Props) {
  const history = useHistory(native);
  const [editing, setEditing] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [session, setSession] = useState(0);
  const [keyboardSelection, setKeyboardSelection] = useState<{ id: string; session: number | undefined } | null>(null);
  const [refocus, setRefocus] = useState<string | null>(null);
  const [copyFailed, setCopyFailed] = useState(false);
  const copyOperation = useRef(0);
  const clickedRecord = useRef<string | null>(null);
  useEffect(() => { if (!active || !snapshot.window.translationVisible) clickedRecord.current = null; }, [active, snapshot.window.translationVisible, snapshot.window.recordsSession]);
  useEffect(() => { ++copyOperation.current; setCopyFailed(false); }, [active, snapshot.window.recordsSession]);
  useEffect(() => () => { ++copyOperation.current; }, []);
  const panel = useRef<HTMLElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const state = snapshot.window;
  const lastQuery = useRef({ query: '', favoritesOnly: false });
  const [followQuery, setFollowQuery] = useState<{ query: string; favoritesOnly: boolean; selectedId: string | null; selectionRevision: number } | null>(null);
  const { query, setQuery, favoritesOnly, setFavoritesOnly, toggleFavorites, remember, restoreScroll } = useRecordBrowser(state.recordsSession, active, list, () => !native || history.displayReady(query, favoritesOnly), () => history.snapshot.records);
  const displayed = history.displaySnapshot(query, favoritesOnly);
  const records = displayed.records;
  const selection = snapshot.window.translationVisible ? native ? history.snapshot.selectedId : selectedId : null;
  const menuRecord = menu ? records.find((record) => record.id === menu.id) : null;
  const actionVersion = useRef(0);
  const actionContext = useRef({ active, query, favoritesOnly, session: state.recordsSession });
  const previousContext = actionContext.current;
  if (previousContext.active !== active || previousContext.query !== query || previousContext.favoritesOnly !== favoritesOnly || previousContext.session !== state.recordsSession) ++actionVersion.current;
  actionContext.current = { active, query, favoritesOnly, session: state.recordsSession };
  useEffect(() => () => { ++actionVersion.current; }, []);
  const allFiltered = useMemo(() => native ? records.filter((record) => !favoritesOnly || record.isFavorite) : filterHistory(records, query, favoritesOnly), [records, query, favoritesOnly, native]);
  const deleting = history.deletingIds;
  const filtered = useMemo(() => deleting.size ? allFiltered.filter((record) => !deleting.has(record.id)) : allFiltered, [allFiltered, deleting]);
  const pendingDeleteCount = allFiltered.length - filtered.length;
  const visibleTotal = Math.max(0, displayed.totalRecords - pendingDeleteCount);
  const name = languageName;

  useEffect(() => {
    if (!native || (lastQuery.current.query === query && lastQuery.current.favoritesOnly === favoritesOnly)) return;
    const favoriteChanged = lastQuery.current.favoritesOnly !== favoritesOnly;
    let cancelled = false;
    const submit = () => {
      lastQuery.current = { query, favoritesOnly };
      void history.setQuery(query, favoritesOnly, favoriteChanged || !query).then(() => {
        if (!cancelled) setFollowQuery({ query, favoritesOnly, selectedId: selection, selectionRevision: history.selectionRevision });
      });
    };
    const timer = favoriteChanged || !query ? undefined : window.setTimeout(submit, 150);
    if (timer === undefined) submit();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [query, favoritesOnly, native, history.setQuery]);

  useEffect(() => {
    if (!followQuery) return;
    if (!active || !state.translationVisible || followQuery.query !== query || followQuery.favoritesOnly !== favoritesOnly) { setFollowQuery(null); return; }
    if (history.loading || history.error || !history.displayReady(query, favoritesOnly)) return;
    setFollowQuery(null);
    if (!followQuery.selectedId || history.selectionRevision !== followQuery.selectionRevision) return;
    if (selection && (selection !== followQuery.selectedId || filtered.some(({ id }) => id === selection))) return;
    const first = filtered.find((record) => !record.isPinned) ?? filtered[0];
    if (first) onSelect(first);
    else void command('history_sample_select', { sampleId: null, retranslate: false, clearDetail: true });
  }, [followQuery, active, state.translationVisible, query, favoritesOnly, history.loading, history.error, filtered, selection, history.selectionRevision]);

  useEffect(() => { setMenu(null); clickedRecord.current = null; }, [query, favoritesOnly, state.historyVisible, active]);
  useEffect(() => { if (!selection) clickedRecord.current = null; }, [selection, history.selectionRevision]);
  useEffect(() => { if (!state.historyVisible || !active) { setEditing(null); setSession((value) => value + 1); } }, [state.historyVisible, active]);
  useEffect(() => { setEditing(null); setMenu(null); setSession((value) => value + 1); }, [state.recordsSession]);
  const focusedId = keyboardSelection?.session === state.recordsSession ? keyboardSelection?.id : undefined;
  const keyboardId = filtered.find(({ id }) => id === focusedId)?.id ?? (filtered.find(({ id }) => id === selection) ?? filtered.find((record) => !record.isPinned) ?? filtered[0])?.id;
  const targetOnMouseMove = useRecordPointer();
  const setTarget = (id: string | null) => setKeyboardSelection((current) =>
    id === null ? null : current?.id === id && current.session === state.recordsSession ? current : { id, session: state.recordsSession });
  const focusReady = !native || history.displayReady(query, favoritesOnly);
  useRecordsOpenFocus(search, active, state.historyVisible, state.recordsFocus, snapshot.clipboardControls.recordsOpenFocus === 'list',
    focusReady, !history.loading, setTarget);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    // A fast clipboard translation can finish while this mounted panel is becoming visible.
    const remove = listen<string>('history_refocus', ({ payload }) => { if (!disposed) setRefocus(payload); });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()); };
  }, [native]);
  useLayoutEffect(() => {
    if (!refocus) return;
    if (!active || !state.historyVisible || !state.translationVisible || editing || menu || history.error) {
      setRefocus(null); return;
    }
    if (!focusReady) return;
    if (!filtered.some(({ id }) => id === refocus)) {
      if (query) setQuery('');
      if (favoritesOnly) setFavoritesOnly(false);
      if (!query && !favoritesOnly) setRefocus(null);
      return;
    }
    setRefocus(null);
    setTarget(refocus);
    focusRow(refocus, true);
  });
  useEffect(() => {
    if (!active || !selection) return;
    const row = [...(pinnedList.current?.querySelectorAll<HTMLElement>('.history-row') ?? []), ...(list.current?.querySelectorAll<HTMLElement>('.history-row') ?? [])].find((row) => row.dataset.recordId === selection);
    const scroller = list.current;
    if (!row || !scroller || !scroller.contains(row)) return;
    const viewport = scroller.getBoundingClientRect();
    const top = historyScrollTarget(scroller.scrollTop, { top: viewport.top + 4, bottom: viewport.bottom - 4 }, row.getBoundingClientRect());
    if (top !== null) scroller.scrollTo({ top, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
  }, [active, selection]);
  useEffect(() => {
    const close = (event: Event) => {
      if (!(event.target instanceof Element && event.target.closest('.record-context-menu-keys'))) setMenu(null);
    };
    window.addEventListener('scroll', close, true);
    return () => window.removeEventListener('scroll', close, true);
  }, []);

  function loadNearEnd(element: HTMLDivElement) {
    if (element.clientHeight > 0 && element.scrollHeight - element.scrollTop - element.clientHeight < 180) void history.loadMore(query, favoritesOnly);
  }

  async function copyRecord(id: string, part: 'source' | 'primary' | 'secondary' = 'source') {
    if (!native || !snapshot.capabilities.clipboardWrite || !active) return false;
    const version = actionVersion.current;
    const ready = history.whenReady(query, favoritesOnly, id);
    if (!(typeof ready === 'boolean' ? ready : await ready) || version !== actionVersion.current || !actionContext.current.active) return false;
    const operation = ++copyOperation.current;
    setCopyFailed(false);
    try {
      await invoke('translation_sample_copy', { sampleId: id, part });
      return operation === copyOperation.current && version === actionVersion.current;
    } catch { if (operation === copyOperation.current && version === actionVersion.current) setCopyFailed(true); }
    return false;
  }

  function whenReady(id: string, action: () => void) {
    const version = actionVersion.current;
    if (!active) return;
    const execute = (ready: boolean) => { if (ready && version === actionVersion.current && actionContext.current.active) action(); };
    const ready = native ? history.whenReady(query, favoritesOnly, id) : true;
    if (typeof ready === 'boolean') execute(ready);
    else void ready.then(execute);
  }

  async function updateRecord(id: string, action: 'favorite' | 'pin' | 'delete') {
    const version = actionVersion.current;
    const follow = { query, favoritesOnly, selectedId: selection, selectionRevision: history.selectionRevision };
    if (action === 'delete') {
      const focusedRow = document.activeElement?.closest<HTMLElement>('article[data-record-id]');
      if (focusedRow?.dataset.recordId === id) {
        const ordered = [...filtered.filter((record) => record.isPinned), ...filtered.filter((record) => !record.isPinned)];
        const index = ordered.findIndex((record) => record.id === id);
        const neighbor = (ordered[index + 1] ?? ordered[index - 1])?.id;
        if (neighbor) focusRow(neighbor);
        else search.current?.focus({ preventScroll: true });
      }
    }
    if (await history.update(id, action) && version === actionVersion.current && actionContext.current.active) setFollowQuery(follow);
  }

  function previewRecord(record: HistoryRecord, explicit = false) {
    if (!explicit && state.recordsBrowsing === false) return;
    if (!state.translationVisible || !explicit && selection === record.id) return;
    whenReady(record.id, () => onSelect(record, false, !explicit));
  }

  async function toggleDetails(record?: HistoryRecord) {
    if (state.translationVisible && (!record || selection === record.id)) {
      await command('records_details_toggle');
      return;
    }
    const target = record ?? filtered.find((item) => !item.isPinned) ?? filtered[0];
    if (target) whenReady(target.id, () => onSelect(target));
    else await command('records_details_toggle');
  }

  function previewKeyDown(key: globalThis.KeyboardEvent, rowId?: string) {
    if (key.defaultPrevented || key.isComposing || key.keyCode === 229 || key.key === 'Dead'
      || !active || !state.historyVisible || editing || menu || panel.current?.querySelector('.history-shortcuts-panel')) return;
    key.preventDefault();
    if (key.repeat) return;
    if (state.translationVisible) { void toggleDetails(); return; }
    const record = filtered.find(({ id }) => id === (rowId ?? keyboardId));
    if (focusReady && record) void toggleDetails(record);
  }

  function retranslateKeyDown(key: globalThis.KeyboardEvent, rowId: string | undefined, preventDefault: () => void) {
    const preferences = snapshot.translationPreferences;
    if (!translationShortcutEnabled(preferences, 'translate') || !matchesBinding(preferences.shortcuts.translate, key)) return false;
    const record = focusReady && filtered.find(({ id }) => id === rowId);
    if (!record) return false;
    preventDefault();
    if (!key.repeat) whenReady(record.id, () => onSelect(record, true));
    return true;
  }

  function copyTranslationKeyDown(key: globalThis.KeyboardEvent, id: string | undefined, preventDefault: () => void) {
    if (!focusReady || !id || !filtered.some((record) => record.id === id)
      || !native || !snapshot.capabilities.clipboardWrite || editsText(key)) return false;
    const preferences = snapshot.translationPreferences;
    const action = (['copyPrimary', 'copySecondary'] as const).find((candidate) =>
      translationShortcutEnabled(preferences, candidate) && matchesBinding(preferences.shortcuts[candidate], key));
    if (!action) return false;
    preventDefault();
    if (!key.repeat) void copyRecord(id, action === 'copyPrimary' ? 'primary' : 'secondary');
    return true;
  }

  const recordShortcutFromBody = useEffectEvent((event: globalThis.KeyboardEvent) => {
    // WebKit can leave focus on the body after a click on the list background.
    if (event.target !== document.body || event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.key === 'Dead'
      || editing || menu || panel.current?.querySelector('.history-shortcuts-panel')) return;
    if (copyTranslationKeyDown(event, keyboardId, () => event.preventDefault())) return;
    const action = clipboardKeyAction(clipboardPanelShortcuts(snapshot, sharedRecordActions), event);
    if (action === 'preview') previewKeyDown(event);
    else if (!action) retranslateKeyDown(event, focusedId, () => event.preventDefault());
  });
  useEffect(() => {
    if (!active || !state.historyVisible) return;
    const keydown = (event: globalThis.KeyboardEvent) => recordShortcutFromBody(event);
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [active, state.historyVisible]);

  function focusRow(id: string, scroll = false) {
    const buttons = [pinnedList.current, list.current].flatMap((element) => [...(element?.querySelectorAll<HTMLButtonElement>('.history-row:not([hidden]) .history-row-select') ?? [])]);
    const button = buttons.find((button) => button.closest<HTMLElement>('article')?.dataset.recordId === id);
    (button ?? search.current)?.focus({ preventScroll: !scroll });
  }

  function recordKeyDown(event: KeyboardEvent<HTMLElement>) {
    onKeyDown(event);
    const key = event.nativeEvent;
    if (!active || editing || menu || event.currentTarget.querySelector('.history-shortcuts-panel') || event.defaultPrevented || key.isComposing || key.keyCode === 229 || key.key === 'Dead') return;
    const target = event.target as HTMLElement;
    const fromSearch = target === search.current;
    const row = target.closest<HTMLElement>('article[data-record-id]');
    if ((row || fromSearch || target.closest('.history-toolbar, .history-footer'))
      && copyTranslationKeyDown(key, row?.dataset.recordId ?? keyboardId, () => event.preventDefault())) return;
    const action = clipboardKeyAction(clipboardPanelShortcuts(snapshot, sharedRecordActions), key);
    if (row && !action && retranslateKeyDown(key, row.dataset.recordId, () => event.preventDefault())) return;
    if (action === 'preview') {
      previewKeyDown(key, row?.dataset.recordId);
      return;
    }
    if (!row && !fromSearch && !target.closest('.history-toolbar, .history-footer')) return;
    const plain = !key.altKey && !key.ctrlKey && !key.metaKey && !key.shiftKey;
    if (plain && !action && (key.key === 'ArrowLeft' || key.key === 'ArrowRight') && (!fromSearch || !query)) {
      if (target.closest('.history-footer')?.querySelector('[role=dialog]')) return;
      event.preventDefault();
      if (!key.repeat) { toggleFavorites(); search.current?.focus({ preventScroll: true }); }
      return;
    }
    if (plain && !action && (key.key === 'ArrowUp' || key.key === 'ArrowDown')) {
      if (!fromSearch && !row) return;
      event.preventDefault();
      if (fromSearch) {
        if (key.key === 'ArrowDown') focusRecordList(search.current?.closest('.history-panel'));
        return;
      }
      const buttons = [pinnedList.current, list.current].flatMap((element) => [...(element?.querySelectorAll<HTMLButtonElement>('.history-row:not([hidden]) .history-row-select') ?? [])]);
      const index = buttons.findIndex((button) => button.closest('article') === row);
      const next = buttons[index + (key.key === 'ArrowDown' ? 1 : -1)];
      if (next) {
        next.focus();
        const record = records.find(({ id }) => id === next.closest<HTMLElement>('article')?.dataset.recordId);
        if (record) previewRecord(record, true);
      } else if (row && key.key === 'ArrowUp') search.current?.focus();
      return;
    }
    if (!action || action === 'switchView') return;
    if (fromSearch && key.key === 'Backspace' && query && action !== 'delete') return;
    const record = row ? filtered.find(({ id }) => id === row.dataset.recordId) : filtered.find(({ id }) => id === keyboardId);
    if (!record || !clipboardShortcutAvailable(snapshot, native, action)) return;
    event.preventDefault();
    if (key.repeat) return;
    const actionId = action === 'copyRecord' ? 'copy' : action;
    const article = row ?? [pinnedList.current, list.current].flatMap((element) => [...(element?.querySelectorAll<HTMLElement>('article[data-record-id]') ?? [])]).find((element) => element.dataset.recordId === record.id);
    const button = article?.querySelector<HTMLButtonElement>(`[data-action=${actionId}] button:not(:disabled)`);
    button?.click();
  }

  function row(record: HistoryRecord) {
    const isDeleting = deleting.has(record.id);
    const canDelete = !record.isFavorite && !record.isPinned;
    const label = `${name(record.sourceLanguage)} - ${record.targetLanguages.map(name).join(', ')}`;
    const description = [
      record.title ? `Note: ${record.title}` : null,
      record.isPinned ? 'Pinned' : null,
      record.isFavorite ? 'Favorite' : null,
      label,
    ].filter(Boolean).join(', ');
    return <article hidden={isDeleting} aria-hidden={isDeleting || undefined} inert={isDeleting || undefined} data-record-target={active && focusReady && !isDeleting && (menu?.id ?? focusedId) === record.id || undefined} onFocus={(event) => {
      if (!event.currentTarget.contains(event.relatedTarget)) { setTarget(record.id); previewRecord(record); }
    }} onMouseMove={(event) => {
      if (!active || editing || menu || !focusReady || isDeleting) return;
      targetOnMouseMove(event, () => { setTarget(record.id); previewRecord(record); });
    }} onContextMenu={(event) => { event.preventDefault(); setMenu({ id: record.id, x: event.clientX, y: event.clientY }); }} key={record.id} data-record-id={record.id} data-keyboard-current={keyboardId === record.id || undefined} className={cn('history-row', selection === record.id && 'selected', menu?.id === record.id && 'context-target', record.isPinned && 'pinned')}
      style={{ '--history-actions-width': `${canDelete ? 124 : 99}px` } as CSSProperties}>
      <button className="history-row-select" aria-label={`Translation: ${record.text}`} aria-description={description} aria-current={keyboardId === record.id || undefined} aria-pressed={selection === record.id} onClick={(event) => {
        setTarget(record.id);
        if (clickedRecord.current === record.id) {
          clickedRecord.current = null;
          void command('records_details_toggle');
          return;
        }
        const row = event.currentTarget.closest<HTMLElement>('.history-row');
        const scroller = list.current;
        if (row && scroller?.contains(row)) {
          const viewport = scroller.getBoundingClientRect();
          const top = historyScrollTarget(scroller.scrollTop, { top: viewport.top + 4, bottom: viewport.bottom - 4 }, row.getBoundingClientRect());
          if (top !== null) scroller.scrollTo({ top, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
        }
        whenReady(record.id, () => {
          clickedRecord.current = record.id;
          onSelect(record);
        });
      }}>
        <RecordHeading date={record.date} label={label} sourceApp={record.sourceApp} />
        <span className={cn('history-preview', !record.title && 'history-text-preview')}>{record.title || historyPreview(record.text)}</span>
      </button>
      <RecordActions recordId={record.id} t={t} native={native} favorite={record.isFavorite} pinned={record.isPinned} canCopy={snapshot.capabilities.clipboardWrite} copyLabel={t('copySource')} deleteLabel={t('deleteHistory')}
        onCopy={() => copyRecord(record.id)} onNote={() => whenReady(record.id, () => setEditing(record.id))} onUpdate={(action) => whenReady(record.id, () => { void updateRecord(record.id, action); })} />
    </article>;
  }

  function menuItems(record: HistoryRecord) {
    const canCopy = native && snapshot.capabilities.clipboardWrite;
    const copyIcon = <HistoryIcon name="doc.on.doc" />;
    const preferences = snapshot.translationPreferences;
    const globals = clipboardGlobalBindings(snapshot);
    const translationBinding = (action: 'copyPrimary' | 'copySecondary' | 'translate') => {
      const binding = preferences.shortcuts[action];
      return binding && translationShortcutEnabled(preferences, action) && !globals.some((key) => key && sameBinding(key, binding)) ? binding : null;
    };
    const primary = translationBinding('copyPrimary');
    const secondary = translationBinding('copySecondary');
    const shared = clipboardPanelShortcuts(snapshot, sharedRecordActions);
    return recordMenuItems([
      [
        { action: 'source', label: t('copySource'), icon: copyIcon, disabled: !canCopy },
        { action: 'primary', label: t('copyPrimaryText'), icon: copyIcon, disabled: !canCopy || !record.translations[0]?.trim() },
        ...(record.translations[1]?.trim() ? [{ action: 'secondary', label: t('copySecondaryText'), icon: copyIcon, disabled: !canCopy }] : []),
      ],
      [{ action: 'translate', label: t('translation'), icon: <HistoryIcon name="languages" /> }],
      [
        { action: 'favorite', label: record.isFavorite ? 'Unfavorite' : 'Favorite', icon: <HistoryIcon name="star" />, active: record.isFavorite, disabled: !native },
        { action: 'pin', label: record.isPinned ? 'Unpin' : 'Pin', icon: <HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} />, disabled: !native },
        { action: 'note', label: 'Note', icon: <HistoryIcon name="note" />, disabled: !native },
      ],
      [{ action: 'delete', label: 'Delete', icon: <HistoryIcon name="trash" />, destructive: true, disabled: !native || record.isFavorite || record.isPinned }],
    ], snapshot.capabilities.platform === 'macos', t, (action) => {
      if (snapshot.storageError) return null;
      if (action === 'primary') return primary;
      if (action === 'secondary') return secondary;
      if (action === 'translate') return translationBinding('translate');
      const key = shared.find((shortcut) => shortcut.action === (action === 'source' ? 'copyRecord' : action) && shortcut.enabled && !shortcut.globalConflict)?.binding;
      // Copy-result chords take precedence over shared record chords in the list too.
      return key && !(canCopy && [primary, secondary].some((binding) => binding && sameBinding(key, binding))) ? key : null;
    });
  }

  return <aside ref={panel} data-focus-ready={focusReady || undefined} id="translation-history" className="history-panel" style={{ '--history-preview-lines': snapshot.clipboardControls.previewLines?.translation } as CSSProperties} aria-label={t('history')} inert={editing !== null} onKeyDown={recordKeyDown}>
    <ActivityBeam label={t('working')} active={active && history.showLoading} />
    <div className="drag-strip" data-tauri-drag-region />
    <RecordToolbar t={t} snapshot={snapshot} command={command} view="translation" query={query} onQuery={setQuery} search={search} onDetailsToggle={() => { void toggleDetails(); }} />
    {history.error && <p role="alert" className="history-error">{t('nativeError')} <button onClick={() => history.refresh(query, favoritesOnly)}>{t('retry')}</button></p>}
    <HistoryList onScrollOwnerChange={restoreScroll} active={active} onScroll={remember} list={list} pinnedList={pinnedList} pinnedCount={filtered.filter((record) => record.isPinned).length} pinned={allFiltered.filter((record) => record.isPinned).map(row)} onNearEnd={loadNearEnd}>
      {allFiltered.filter((record) => !record.isPinned).map(row)}
      {!history.loading && !history.error && filtered.length === 0 && <div className="history-empty" role="status">{query ? t('historyNoMatches') : favoritesOnly ? t('historyFavoritesEmpty') : t('historyEmpty')}</div>}
    </HistoryList>
    <HistoryFooter key={`${state.recordsSession ?? 0}:${session}`} t={t} snapshot={snapshot} command={command} native={native} active={active && state.historyVisible && !menu && !editing} search={search} favoritesOnly={favoritesOnly} onFavorites={toggleFavorites} total={visibleTotal} />
    {menu && menuRecord && <RecordMenu key={menuRecord.id} x={menu.x} y={menu.y} onClose={() => setMenu(null)} onReturnFocus={() => focusRow(menuRecord.id, true)} items={menuItems(menuRecord)} onAction={(action) => {
      setMenu(null);
      if (action === 'source' || action === 'primary' || action === 'secondary') { void copyRecord(menuRecord.id, action); return; }
      whenReady(menuRecord.id, () => {
        if (action === 'translate') onSelect(menuRecord, true);
        else if (action === 'note') setEditing(menuRecord.id);
        else if (action === 'favorite' || action === 'pin' || action === 'delete') void updateRecord(menuRecord.id, action);
      });
    }} />}
    {editing && <ClipboardEditor recordId={editing} onClose={() => setEditing(null)} saveTitle={(title) => history.update(editing, 'note', title)} onReturnFocus={() => { requestAnimationFrame(() => focusRow(editing)); }} />}
    <RecordFeedback message={active && copyFailed ? t('copyFailed') : null} />
  </aside>;
}
