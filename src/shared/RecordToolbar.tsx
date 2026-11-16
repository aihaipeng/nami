import type { RefObject } from 'react';
import type { ViewProps } from '../App';
import { HistoryIcon } from '../translation/HistoryIcon';
import { HistoryWordmark } from './HistoryWordmark';

export function RecordToolbar({ t, snapshot, command, view, query, onQuery, search, onArrowDown }: Pick<ViewProps, 't' | 'snapshot' | 'command'> & {
  view: 'clipboard' | 'translation'; query: string; onQuery: (value: string) => void;
  search: RefObject<HTMLInputElement | null>; onArrowDown?: () => void;
}) {
  const state = snapshot.window;
  const searchLabel = t(view === 'clipboard' ? 'searchClipboard' : 'searchHistory');
  return <header className="history-toolbar" onMouseDown={(event) => {
    if ((event.target as HTMLElement).closest('button')) event.preventDefault();
  }}>
    <HistoryWordmark />
    <label className="history-search"><HistoryIcon name="magnifyingglass" /><input ref={search} type="search" aria-label={searchLabel} placeholder={searchLabel} value={query} onChange={(event) => onQuery(event.target.value)} onKeyDown={(event) => {
      if (event.key === 'ArrowDown' && !event.nativeEvent.isComposing && event.keyCode !== 229 && onArrowDown) { event.preventDefault(); onArrowDown(); }
    }} />
      {query && <button type="button" aria-label={t(view === 'clipboard' ? 'clearClipboardSearch' : 'clearHistorySearch')} onClick={() => { onQuery(''); search.current?.focus(); }}><HistoryIcon name="xmark.circle.fill" /></button>}
    </label>
    <button type="button" className="icon-button" title="Clipboard" aria-label="Clipboard" aria-pressed={view === 'clipboard'} onClick={() => { if (view !== 'clipboard') void command('records_view_set', { view: 'clipboard' }); }}><HistoryIcon name="list.bullet" /></button>
    <button type="button" className="icon-button" title="Translations" aria-label="Translations" aria-pressed={view === 'translation'} onClick={() => { if (view !== 'translation') void command('records_view_set', { view: 'translation' }); }}><HistoryIcon name="languages" /></button>
    <button type="button" className="icon-button history-pin" title={state.historyPinned ? 'Unpin records' : 'Pin records'} aria-label={state.historyPinned ? 'Unpin records' : 'Pin records'} aria-pressed={state.historyPinned} onClick={() => void command('window_pin', { region: 'history', pinned: !state.historyPinned })}><HistoryIcon name={state.historyPinned ? 'pin.fill' : 'pin'} /></button>
    <button type="button" className="icon-button records-details" title={view === 'clipboard' ? 'Details unavailable' : state.translationVisible ? 'Collapse details' : 'Expand details'} aria-label={view === 'clipboard' ? 'Details unavailable' : state.translationVisible ? 'Collapse details' : 'Expand details'} disabled={view === 'clipboard'} aria-expanded={view === 'translation' && state.translationVisible} onClick={() => void command('records_details_toggle')}><svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d={state.translationVisible && view === 'translation' ? 'M8.5 3.5 5 7l3.5 3.5' : 'M5.5 3.5 9 7l-3.5 3.5'} strokeLinecap="round" strokeLinejoin="round" /></svg></button>
  </header>;
}
