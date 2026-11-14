import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { HistoryIcon } from './HistoryIcon';
import { HistoryWordmark } from '../shared/HistoryWordmark';
import type { ViewProps } from '../App';
import { cn } from '../lib/utils';
import { languages } from '../shared/languages';
import { filterHistory, historyPreview, historyScrollTarget, type HistoryRecord } from './history';
import fallbackIcon from '../assets/history/reference-nami.png';
import { useHistory } from './useHistory';
import { ActivityBeam } from '../shared/ActivityBeam';
import { HistoryList } from '../shared/HistoryList';
import { HistoryFooter } from '../shared/HistoryFooter';

interface Props extends Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> {
  selectedId: string | null;
  onSelect: (record: HistoryRecord, retranslate?: boolean) => void;
  onKeyDown: (event: KeyboardEvent) => void;
}

export function HistoryPanel({ t, snapshot, command, native, selectedId, onSelect, onKeyDown }: Props) {
  const history = useHistory(native);
  const records = history.snapshot.records;
  const selection = native ? history.snapshot.selectedId : selectedId;
  const [query, setQuery] = useState('');
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const state = snapshot.window;
  const filtered = useMemo(() => native ? records.filter((record) => !favoritesOnly || record.isFavorite).sort((a,b) => Number(b.isPinned)-Number(a.isPinned) || Date.parse(b.date)-Date.parse(a.date)) : filterHistory(records, query, favoritesOnly), [records, query, favoritesOnly, native]);
  const lastQuery = useRef({ query: '', favoritesOnly: false });
  const name = (code: string) => languages.find(([id]) => id === code)?.[1] ?? code;

  useEffect(() => { list.current?.scrollTo(0, 0); pinnedList.current?.scrollTo(0, 0); }, [query, favoritesOnly]);
  useEffect(() => {
    if (!native || (lastQuery.current.query === query && lastQuery.current.favoritesOnly === favoritesOnly)) return;
    const favoriteChanged = lastQuery.current.favoritesOnly !== favoritesOnly;
    const submit = () => {
      lastQuery.current = { query, favoritesOnly };
      void history.setQuery(query, favoritesOnly);
    };
    if (favoriteChanged) { submit(); return; }
    const timer = window.setTimeout(submit, 150);
    return () => window.clearTimeout(timer);
  }, [query, favoritesOnly, native, history.setQuery]);

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
    return <article key={record.id} data-record-id={record.id} className={cn('history-row', selection === record.id && 'selected', record.isPinned && 'pinned')}
      style={{ '--history-actions-width': `${canDelete ? 124 : 99}px` } as CSSProperties}>
      <button className="history-row-select" aria-label={`Translation: ${record.text}`} aria-pressed={selection === record.id} onClick={(event) => {
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
    {history.loading && <ActivityBeam label={t('working')} />}
    <div className="drag-strip" data-tauri-drag-region />
    <header className="history-toolbar" onMouseDown={(event) => {
      if ((event.target as HTMLElement).closest('button')) event.preventDefault();
    }}>
      <HistoryWordmark />
      <label className="history-search"><HistoryIcon name="magnifyingglass" /><input ref={search} type="search" aria-label={t('searchHistory')} placeholder={t('searchHistory')} value={query} onChange={(event) => setQuery(event.target.value)} />
        {query && <button type="button" aria-label={t('clearHistorySearch')} onClick={() => { setQuery(''); search.current?.focus(); }}><HistoryIcon name="xmark.circle.fill" /></button>}
      </label>
      <button className="icon-button" title={t('allHistory')} aria-label={t('allHistory')} aria-pressed={!favoritesOnly} onClick={() => setFavoritesOnly(false)}><HistoryIcon name="list.bullet" /></button>
      <button className="icon-button" title={t('favoriteHistory')} aria-label={t('favoriteHistory')} aria-pressed={favoritesOnly} onClick={() => setFavoritesOnly(true)}><HistoryIcon name={favoritesOnly ? 'star.fill' : 'star'} /></button>
      <button className="icon-button history-pin" title={state.historyPinned ? t('unpin') : t('pinHistory')} aria-label={state.historyPinned ? t('unpin') : t('pinHistory')} aria-pressed={state.historyPinned} onClick={() => void command('window_pin', { region: 'history', pinned: !state.historyPinned })}><HistoryIcon name={state.historyPinned ? 'pin.fill' : 'pin'} /></button>
    </header>
    {history.error && <p role="alert" className="history-error">{t('nativeError')}</p>}
    <HistoryList list={list} pinnedList={pinnedList} pinned={filtered.filter((record) => record.isPinned).map(row)} onNearEnd={loadNearEnd}>
      {filtered.filter((record) => !record.isPinned).map(row)}
      {!history.loading && !history.error && filtered.length === 0 && <div className="history-empty" role="status">{query ? t('historyNoMatches') : favoritesOnly ? t('historyFavoritesEmpty') : t('historyEmpty')}</div>}
    </HistoryList>
    <HistoryFooter total={history.snapshot.totalRecords} primaryModifier={snapshot.capabilities.primaryModifier} />
  </aside>;
}
