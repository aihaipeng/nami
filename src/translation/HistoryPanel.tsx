import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { HistoryIcon } from './HistoryIcon';
import { RecordToolbar } from '../shared/RecordToolbar';
import { useRecordBrowser } from '../shared/useRecordBrowser';
import type { ViewProps } from '../App';
import { cn } from '../lib/utils';
import { languages } from '../shared/languages';
import { filterHistory, historyPreview, historyScrollTarget, type HistoryRecord } from './history';
import fallbackIcon from '../assets/history/reference-nami.png';
import { useHistory } from './useHistory';
import { ActivityBeam } from '../shared/ActivityBeam';
import { HistoryList } from '../shared/HistoryList';
import { RecordMenu } from '../shared/RecordMenu';
import { HistoryFooter } from '../shared/HistoryFooter';

interface Props extends Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> {
  selectedId: string | null;
  onSelect: (record: HistoryRecord, retranslate?: boolean) => void;
  onKeyDown: (event: KeyboardEvent) => void;
  active?: boolean;
}

export function HistoryPanel({ t, snapshot, command, native, selectedId, onSelect, onKeyDown, active = true }: Props) {
  const history = useHistory(native);
  const records = history.snapshot.records;
  const selection = snapshot.window.translationVisible ? native ? history.snapshot.selectedId : selectedId : null;
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [session, setSession] = useState(0);
  const menuRecord = menu ? records.find((record) => record.id === menu.id) : null;
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const state = snapshot.window;
  const { query, setQuery, favoritesOnly, toggleFavorites, remember } = useRecordBrowser(state.recordsSession, active, list, pinnedList);
  const filtered = useMemo(() => native ? records.filter((record) => !favoritesOnly || record.isFavorite).sort((a,b) => Number(b.isPinned)-Number(a.isPinned) || Date.parse(b.date)-Date.parse(a.date)) : filterHistory(records, query, favoritesOnly), [records, query, favoritesOnly, native]);
  const lastQuery = useRef({ query: '', favoritesOnly: false });
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
  useEffect(() => {
    const close = () => setMenu(null);
    window.addEventListener('scroll', close, true);
    return () => window.removeEventListener('scroll', close, true);
  }, []);

  function loadNearEnd(element: HTMLDivElement) {
    if (element.clientHeight > 0 && element.scrollHeight - element.scrollTop - element.clientHeight < 240) void history.loadMore();
  }

  function row(record: HistoryRecord) {
    const canDelete = !record.isFavorite && !record.isPinned;
    const date = new Date(record.date);
    const today = date.toDateString() === new Date().toDateString();
    const two = (value: number) => String(value).padStart(2, '0');
    const time = `${today ? '' : `${two(date.getMonth() + 1)}-${two(date.getDate())} `}${two(date.getHours())}:${two(date.getMinutes())}`;
    const label = `${name(record.sourceLanguage)} - ${record.targetLanguages.map(name).join(', ')}`;
    return <article onContextMenu={(event) => { event.preventDefault(); setMenu({ id: record.id, x: event.clientX, y: event.clientY }); }} key={record.id} data-record-id={record.id} className={cn('history-row', selection === record.id && 'selected', record.isPinned && 'pinned')}
      style={{ '--history-actions-width': `${canDelete ? 124 : 99}px` } as CSSProperties}>
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
        const scroller = row?.closest<HTMLElement>('.history-list, .history-pinned');
        if (row && scroller) {
          const viewport = scroller.getBoundingClientRect();
          const top = historyScrollTarget(scroller.scrollTop, { top: viewport.top + 4, bottom: viewport.bottom - 4 }, row.getBoundingClientRect());
          if (top !== null) scroller.scrollTo({ top, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
        }
        onSelect(record);
      }}>
        <span className="history-row-heading">
          {record.sourceApp?.iconDataUrl
            ? <img className="history-origin" src={record.sourceApp.iconDataUrl} alt={record.sourceApp.name ?? 'Nami'} title={record.sourceApp.name ?? 'Nami'} />
            : <span className="history-origin history-symbol" role="img" aria-label={record.sourceApp?.name ?? 'Nami'} title={record.sourceApp?.name ?? 'Nami'} style={{ '--symbol-url': `url("${fallbackIcon}")`, borderRadius: 0, background: '#fff' } as CSSProperties} />}
          <span className="history-languages" title={label}>{label}</span>
          <time dateTime={record.date}>{time}</time>
        </span>
        <span className="history-preview">{historyPreview(record.text)}</span>
      </button>
      <div className="history-row-actions" role="group" aria-label={t('actions')}>
        <button className="icon-button" title={t('retranslate')} aria-label={t('retranslate')} onClick={() => onSelect(record, true)}><HistoryIcon name="languages" /></button>
        <button className="icon-button" title={t('copySource')} aria-label={t('copySource')} disabled={!native || !snapshot.capabilities.clipboardWrite} onClick={() => void command('translation_sample_copy', { sampleId: record.id, part: 'source' })}><HistoryIcon name="doc.on.doc" /></button>
        <button className="icon-button" title={record.isFavorite ? t('unfavorite') : t('favorite')} aria-label={record.isFavorite ? t('unfavorite') : t('favorite')} aria-pressed={record.isFavorite} disabled={!native} onClick={() => void history.update(record.id, 'favorite')}><HistoryIcon name={record.isFavorite ? 'star.fill' : 'star'} /></button>
        <button className="icon-button" title={record.isPinned ? t('unpinRecord') : t('pinRecord')} aria-label={record.isPinned ? t('unpinRecord') : t('pinRecord')} aria-pressed={record.isPinned} disabled={!native} onClick={() => void history.update(record.id, 'pin')}><HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /></button>
        {canDelete && <button className="icon-button history-delete" title={t('deleteHistory')} aria-label={t('deleteHistory')} disabled={!native} onClick={() => void history.update(record.id, 'delete')}><HistoryIcon name="trash" /></button>}
      </div>
    </article>;
  }

  return <aside id="translation-history" className="history-panel" aria-label={t('history')} onKeyDown={onKeyDown}>
    {history.showLoading && <ActivityBeam label={t('working')} />}
    <div className="drag-strip" data-tauri-drag-region />
    <RecordToolbar t={t} snapshot={snapshot} command={command} view="translation" query={query} onQuery={setQuery} search={search} onArrowDown={() => {
      (pinnedList.current?.querySelector<HTMLButtonElement>('.history-row-select') ?? list.current?.querySelector<HTMLButtonElement>('.history-row-select'))?.focus();
    }} />
    {history.error && <p role="alert" className="history-error">{t('nativeError')}</p>}
    <HistoryList onScroll={remember} list={list} pinnedList={pinnedList} pinned={filtered.filter((record) => record.isPinned).map(row)} onNearEnd={loadNearEnd}>
      {filtered.filter((record) => !record.isPinned).map(row)}
      {!history.loading && !history.error && filtered.length === 0 && <div className="history-empty" role="status">{query ? t('historyNoMatches') : favoritesOnly ? t('historyFavoritesEmpty') : t('historyEmpty')}</div>}
    </HistoryList>
    <HistoryFooter key={`${state.recordsSession ?? 0}:${session}`} favoritesOnly={favoritesOnly} onFavorites={toggleFavorites} total={history.snapshot.totalRecords} primaryModifier={snapshot.capabilities.primaryModifier} />
    {menu && menuRecord && <RecordMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} onReturnFocus={() => { document.querySelector<HTMLButtonElement>(`[data-record-id="${menuRecord.id}"] .history-row-select`)?.focus(); }} items={[
      { action: 'retranslate', label: 'Retranslate', icon: <HistoryIcon name="languages" /> },
      { action: 'copy', label: 'Copy source text', icon: <HistoryIcon name="doc.on.doc" />, disabled: !native || !snapshot.capabilities.clipboardWrite },
      { action: 'favorite', label: menuRecord.isFavorite ? 'Remove favorite' : 'Favorite', icon: <HistoryIcon name={menuRecord.isFavorite ? 'star.fill' : 'star'} />, disabled: !native },
      { action: 'pin', label: menuRecord.isPinned ? 'Unpin record' : 'Pin record', icon: <HistoryIcon name={menuRecord.isPinned ? 'pin.fill' : 'pin'} />, disabled: !native },
      { action: 'delete', label: 'Delete', icon: <HistoryIcon name="trash" />, destructive: true, disabled: !native || menuRecord.isFavorite || menuRecord.isPinned },
    ]} onAction={(action) => {
      setMenu(null);
      if (action === 'retranslate') onSelect(menuRecord, true);
      else if (action === 'copy') void command('translation_sample_copy', { sampleId: menuRecord.id, part: 'source' });
      else void history.update(menuRecord.id, action as 'favorite' | 'pin' | 'delete');
    }} />}
  </aside>;
}
