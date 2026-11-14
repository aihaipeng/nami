import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { HistoryIcon } from './HistoryIcon';
import type { ViewProps } from '../App';
import { cn } from '../lib/utils';
import { languages } from '../shared/languages';
import { filterHistory, historyPreview, historySamples, historyScrollTarget, type HistoryRecord } from './history';
import sampleIcon from '../../src-tauri/icons/icon.png';

interface Props extends Pick<ViewProps, 't' | 'snapshot' | 'command' | 'native'> {
  selectedId: string | null;
  onSelect: (record: HistoryRecord, retranslate?: boolean) => void;
  onKeyDown: (event: KeyboardEvent) => void;
}

export function HistoryPanel({ t, snapshot, command, native, selectedId, onSelect, onKeyDown }: Props) {
  const [records, setRecords] = useState(historySamples);
  const [query, setQuery] = useState('');
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const state = snapshot.window;
  const filtered = filterHistory(records, query, favoritesOnly);
  const name = (code: string) => languages.find(([id]) => id === code)?.[1] ?? code;

  useEffect(() => { list.current?.scrollTo(0, 0); pinnedList.current?.scrollTo(0, 0); }, [query, favoritesOnly]);

  function update(id: string, field: 'isFavorite' | 'isPinned') {
    setRecords((current) => current.map((record) => record.id === id ? { ...record, [field]: !record[field] } : record));
  }

  function row(record: HistoryRecord) {
    const canDelete = !record.isFavorite && !record.isPinned;
    const date = new Date(record.date);
    const today = date.toDateString() === new Date().toDateString();
    const two = (value: number) => String(value).padStart(2, '0');
    const time = `${today ? '' : `${two(date.getMonth() + 1)}-${two(date.getDate())} `}${two(date.getHours())}:${two(date.getMinutes())}`;
    const label = `${name(record.sourceLanguage)} - ${record.targetLanguages.map(name).join(', ')}`;
    return <article key={record.id} data-record-id={record.id} className={cn('history-row', selectedId === record.id && 'selected', record.isPinned && 'pinned')}
      style={{ '--history-actions-width': `${canDelete ? 124 : 99}px` } as CSSProperties}>
      <button className="history-row-select" aria-label={`Sample translation: ${record.text}`} aria-pressed={selectedId === record.id} onClick={(event) => {
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
          <img className="history-origin" src={sampleIcon} alt="Nami sample" title="Synthetic test record · Nami" />
          <span className="history-languages" title={label}>{label}</span>
          <time dateTime={record.date}>{time}</time>
        </span>
        <span className="history-preview">{historyPreview(record.text)}</span>
      </button>
      <div className="history-row-actions" role="group" aria-label={t('actions')}>
        <button className="icon-button" title={t('retranslate')} aria-label={t('retranslate')} onClick={() => onSelect(record, true)}><HistoryIcon name="arrow.clockwise" /></button>
        <button className="icon-button" title={t('copySource')} aria-label={t('copySource')} disabled={!native} onClick={() => void command('translation_sample_copy', { sampleId: record.id, part: 'source' })}><HistoryIcon name="doc.on.doc" /></button>
        <button className="icon-button" title={record.isFavorite ? t('unfavorite') : t('favorite')} aria-label={record.isFavorite ? t('unfavorite') : t('favorite')} aria-pressed={record.isFavorite} onClick={() => update(record.id, 'isFavorite')}><HistoryIcon name={record.isFavorite ? 'star.fill' : 'star'} /></button>
        <button className="icon-button" title={record.isPinned ? t('unpinRecord') : t('pinRecord')} aria-label={record.isPinned ? t('unpinRecord') : t('pinRecord')} aria-pressed={record.isPinned} onClick={() => update(record.id, 'isPinned')}><HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /></button>
        {canDelete && <button className="icon-button history-delete" title={t('deleteHistory')} aria-label={t('deleteHistory')} onClick={() => setRecords((current) => current.filter(({ id }) => id !== record.id))}><HistoryIcon name="trash" /></button>}
      </div>
    </article>;
  }

  return <aside id="translation-history" className="history-panel" aria-label={t('history')} onKeyDown={onKeyDown}>
    <div className="drag-strip" data-tauri-drag-region />
    <header className="history-toolbar" onMouseDown={(event) => {
      if ((event.target as HTMLElement).closest('button')) event.preventDefault();
    }}>
      <label className="history-search"><HistoryIcon name="magnifyingglass" /><input ref={search} type="search" aria-label={t('searchHistory')} placeholder={t('searchHistory')} value={query} onChange={(event) => setQuery(event.target.value)} />
        {query && <button type="button" aria-label={t('clearHistorySearch')} onClick={() => { setQuery(''); search.current?.focus(); }}><HistoryIcon name="xmark.circle.fill" /></button>}
      </label>
      <button className="icon-button" title={t('allHistory')} aria-label={t('allHistory')} aria-pressed={!favoritesOnly} onClick={() => setFavoritesOnly(false)}><HistoryIcon name="list.bullet" /></button>
      <button className="icon-button" title={t('favoriteHistory')} aria-label={t('favoriteHistory')} aria-pressed={favoritesOnly} onClick={() => setFavoritesOnly(true)}><HistoryIcon name={favoritesOnly ? 'star.fill' : 'star'} /></button>
      <button className="icon-button history-pin" title={state.historyPinned ? t('unpin') : t('pinHistory')} aria-label={state.historyPinned ? t('unpin') : t('pinHistory')} aria-pressed={state.historyPinned} onClick={() => void command('window_pin', { region: 'history', pinned: !state.historyPinned })}><HistoryIcon name={state.historyPinned ? 'pin.fill' : 'pin'} /></button>
    </header>
    <div ref={pinnedList} className={cn('history-pinned', !filtered.some((record) => record.isPinned) && 'empty')}>{filtered.filter((record) => record.isPinned).map(row)}</div>
    <div ref={list} className="history-list">
      {filtered.filter((record) => !record.isPinned).map(row)}
      {filtered.length === 0 && <div className="history-empty" role="status">{query ? t('historyNoMatches') : favoritesOnly ? t('historyFavoritesEmpty') : t('historyEmpty')}</div>}
    </div>
  </aside>;
}
