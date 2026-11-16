import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { HistoryIcon } from './HistoryIcon';
import { RecordToolbar } from '../shared/RecordToolbar';
import { useRecordBrowser } from '../shared/useRecordBrowser';
import type { ViewProps } from '../App';
import { cn } from '../lib/utils';
import { languages } from '../shared/languages';
import { filterHistory, historyPreview, historyScrollTarget, type HistoryRecord } from './history';
import { RecordHeading } from '../shared/RecordHeading';
import { RecordActions } from '../shared/RecordActions';
import { useHistory } from './useHistory';
import { ActivityBeam } from '../shared/ActivityBeam';
import { HistoryList } from '../shared/HistoryList';
import { RecordMenu } from '../shared/RecordMenu';
import { HistoryFooter } from '../shared/HistoryFooter';
import { RecordFeedback } from '../shared/RecordFeedback';
import { invoke } from '@tauri-apps/api/core';

interface Props extends Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> {
  selectedId: string | null;
  onSelect: (record: HistoryRecord, retranslate?: boolean) => void;
  onKeyDown: (event: KeyboardEvent) => void;
  active?: boolean;
  activity?: ViewProps['activity'];
}

export function HistoryPanel({ t, snapshot, command, native, selectedId, onSelect, onKeyDown, active = true, activity }: Props) {
  const history = useHistory(native);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [session, setSession] = useState(0);
  const [copyFailed, setCopyFailed] = useState(false);
  const copyOperation = useRef(0);
  useEffect(() => { ++copyOperation.current; setCopyFailed(false); }, [active, snapshot.window.recordsSession]);
  useEffect(() => () => { ++copyOperation.current; }, []);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const state = snapshot.window;
  const lastQuery = useRef({ query: '', favoritesOnly: false });
  const { query, setQuery, favoritesOnly, toggleFavorites, remember } = useRecordBrowser(state.recordsSession, active, list, pinnedList, () => !native || history.displayReady(query, favoritesOnly), () => history.snapshot.records);
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
  const filtered = useMemo(() => native ? records.filter((record) => !favoritesOnly || record.isFavorite).sort((a,b) => Number(b.isPinned)-Number(a.isPinned) || Date.parse(b.date)-Date.parse(a.date)) : filterHistory(records, query, favoritesOnly), [records, query, favoritesOnly, native]);
  const name = (code: string) => languages.find(([id]) => id === code)?.[1] ?? code;

  useEffect(() => {
    if (!native || (lastQuery.current.query === query && lastQuery.current.favoritesOnly === favoritesOnly)) return;
    const favoriteChanged = lastQuery.current.favoritesOnly !== favoritesOnly;
    const submit = () => {
      lastQuery.current = { query, favoritesOnly };
      void history.setQuery(query, favoritesOnly, favoriteChanged);
    };
    if (favoriteChanged) { submit(); return; }
    const timer = window.setTimeout(submit, 150);
    return () => window.clearTimeout(timer);
  }, [query, favoritesOnly, native, history.setQuery]);

  useEffect(() => { setMenu(null); }, [query, favoritesOnly, state.historyVisible, active]);
  useEffect(() => { if (!state.historyVisible || !active) setSession((value) => value + 1); }, [state.historyVisible, active]);
  useEffect(() => { setMenu(null); setSession((value) => value + 1); }, [state.recordsSession]);
  useEffect(() => { if (active && state.recordsFocus !== undefined) search.current?.focus({ preventScroll: true }); }, [active, state.recordsFocus]);
  useEffect(() => {
    if (!active || !selection) return;
    const row = [...(pinnedList.current?.querySelectorAll<HTMLElement>('.history-row') ?? []), ...(list.current?.querySelectorAll<HTMLElement>('.history-row') ?? [])].find((row) => row.dataset.recordId === selection);
    const scroller = list.current;
    if (!row || !scroller) return;
    const viewport = scroller.getBoundingClientRect();
    const top = historyScrollTarget(scroller.scrollTop, { top: viewport.top + (row.closest('.history-pinned') || pinnedList.current?.dataset.overflow === 'true' ? 0 : Math.min(pinnedList.current?.offsetHeight ?? 0, (viewport.bottom - viewport.top))) + 4, bottom: viewport.bottom - 4 }, row.getBoundingClientRect());
    if (top !== null) scroller.scrollTo({ top, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
  }, [active, selection]);
  useEffect(() => {
    const close = () => setMenu(null);
    window.addEventListener('scroll', close, true);
    return () => window.removeEventListener('scroll', close, true);
  }, []);

  function loadNearEnd(element: HTMLDivElement) {
    if (element.clientHeight > 0 && element.scrollHeight - element.scrollTop - element.clientHeight < 180) void history.loadMore(query, favoritesOnly);
  }

  async function copyRecord(id: string) {
    if (!native || !snapshot.capabilities.clipboardWrite) return;
    const operation = ++copyOperation.current;
    setCopyFailed(false); activity?.(1);
    try { await invoke('translation_sample_copy', { sampleId: id, part: 'source' }); }
    catch { if (operation === copyOperation.current) setCopyFailed(true); }
    finally { activity?.(-1); }
  }

  function whenReady(id: string, action: () => void) {
    const version = actionVersion.current;
    if (!active) return;
    const execute = (ready: boolean) => { if (ready && version === actionVersion.current && actionContext.current.active) action(); };
    const ready = native ? history.whenReady(query, favoritesOnly, id) : true;
    if (typeof ready === 'boolean') execute(ready);
    else void ready.then(execute);
  }

  function row(record: HistoryRecord) {
    const canDelete = !record.isFavorite && !record.isPinned;
    const label = `${name(record.sourceLanguage)} - ${record.targetLanguages.map(name).join(', ')}`;
    return <article onContextMenu={(event) => { event.preventDefault(); setMenu({ id: record.id, x: event.clientX, y: event.clientY }); }} key={record.id} data-record-id={record.id} className={cn('history-row', selection === record.id && 'selected', record.isPinned && 'pinned')}
      style={{ '--history-actions-width': `${canDelete ? 99 : 74}px` } as CSSProperties}>
      <button className="history-row-select" onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault();
          const buttons = [...(event.currentTarget.closest('.history-panel')?.querySelectorAll<HTMLButtonElement>('.history-row-select') ?? [])];
          const index = buttons.indexOf(event.currentTarget);
          if (event.key === 'ArrowDown') buttons[index + 1]?.focus();
          else if (index > 0) buttons[index - 1]?.focus();
          else search.current?.focus();
        }
      }} aria-label={`Translation: ${record.text}`} aria-pressed={selection === record.id} onClick={(event) => {
        const row = event.currentTarget.closest<HTMLElement>('.history-row');
        const scroller = list.current;
        if (row && scroller) {
          const viewport = scroller.getBoundingClientRect();
          const top = historyScrollTarget(scroller.scrollTop, { top: viewport.top + (row.closest('.history-pinned') || pinnedList.current?.dataset.overflow === 'true' ? 0 : Math.min(pinnedList.current?.offsetHeight ?? 0, (viewport.bottom - viewport.top))) + 4, bottom: viewport.bottom - 4 }, row.getBoundingClientRect());
          if (top !== null) scroller.scrollTo({ top, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
        }
        whenReady(record.id, () => onSelect(record));
      }}>
        <RecordHeading date={record.date} label={label} sourceApp={record.sourceApp} />
        <span className="history-preview">{historyPreview(record.text)}</span>
      </button>
      <RecordActions t={t} native={native} favorite={record.isFavorite} pinned={record.isPinned} canCopy={snapshot.capabilities.clipboardWrite} copyLabel={t('copySource')} deleteLabel={t('deleteHistory')}
        onCopy={() => whenReady(record.id, () => { void copyRecord(record.id); })} onUpdate={(action) => whenReady(record.id, () => { void history.update(record.id, action); })} />
    </article>;
  }

  return <aside id="translation-history" className="history-panel" aria-label={t('history')} onKeyDown={onKeyDown}>
    {active && history.showLoading && <ActivityBeam label={t('working')} />}
    <div className="drag-strip" data-tauri-drag-region />
    <RecordToolbar t={t} snapshot={snapshot} command={command} view="translation" query={query} onQuery={setQuery} search={search} onArrowDown={() => {
      (pinnedList.current?.querySelector<HTMLButtonElement>('.history-row-select') ?? list.current?.querySelector<HTMLButtonElement>('.history-row-select'))?.focus();
    }} />
    {history.error && <p role="alert" className="history-error">{t('nativeError')} <button onClick={() => history.refresh(query, favoritesOnly)}>{t('retry')}</button></p>}
    <HistoryList active={active} onScroll={remember} list={list} pinnedList={pinnedList} pinned={filtered.filter((record) => record.isPinned).map(row)} onNearEnd={loadNearEnd}>
      {filtered.filter((record) => !record.isPinned).map(row)}
      {!history.loading && !history.error && filtered.length === 0 && <div className="history-empty" role="status">{query ? t('historyNoMatches') : favoritesOnly ? t('historyFavoritesEmpty') : t('historyEmpty')}</div>}
    </HistoryList>
    <HistoryFooter key={`${state.recordsSession ?? 0}:${session}`} favoritesOnly={favoritesOnly} onFavorites={toggleFavorites} total={displayed.totalRecords} primaryModifier={snapshot.capabilities.primaryModifier} />
    {menu && menuRecord && <RecordMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} onReturnFocus={() => { document.querySelector<HTMLButtonElement>(`#translation-history [data-record-id="${menuRecord.id}"] .history-row-select`)?.focus(); }} items={[
      { action: 'retranslate', label: 'Retranslate', icon: <HistoryIcon name="languages" /> },
      { action: 'copy', label: 'Copy source text', icon: <HistoryIcon name="doc.on.doc" />, disabled: !native || !snapshot.capabilities.clipboardWrite },
      { action: 'favorite', label: menuRecord.isFavorite ? 'Remove favorite' : 'Favorite', icon: <HistoryIcon name={menuRecord.isFavorite ? 'star.fill' : 'star'} />, disabled: !native },
      { action: 'pin', label: menuRecord.isPinned ? 'Unpin record' : 'Pin record', icon: <HistoryIcon name={menuRecord.isPinned ? 'pin.fill' : 'pin'} />, disabled: !native },
      { action: 'delete', label: 'Delete', icon: <HistoryIcon name="trash" />, destructive: true, disabled: !native || menuRecord.isFavorite || menuRecord.isPinned },
    ]} onAction={(action) => {
      setMenu(null);
      whenReady(menuRecord.id, () => {
        if (action === 'retranslate') onSelect(menuRecord, true);
        else if (action === 'copy') void copyRecord(menuRecord.id);
        else void history.update(menuRecord.id, action as 'favorite' | 'pin' | 'delete');
      });
    }} />}
    <RecordFeedback message={active && copyFailed ? t('copyFailed') : null} />
  </aside>;
}
