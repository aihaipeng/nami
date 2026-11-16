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
      <button type="button" hidden={!query} aria-label={t(view === 'clipboard' ? 'clearClipboardSearch' : 'clearHistorySearch')} onClick={() => { onQuery(''); search.current?.focus(); }}><HistoryIcon name="xmark.circle.fill" /></button>
    </label>
    <button type="button" className="icon-button" title="Clipboard" aria-label="Clipboard" aria-pressed={view === 'clipboard'} onClick={() => { if (view !== 'clipboard') void command('records_view_set', { view: 'clipboard' }); }}><HistoryIcon name="list.bullet" /></button>
    <button type="button" className="icon-button" title="Translations" aria-label="Translations" aria-pressed={view === 'translation'} onClick={() => { if (view !== 'translation') void command('records_view_set', { view: 'translation' }); }}><HistoryIcon name="languages" /></button>
    <button type="button" className="icon-button history-pin" title={state.historyPinned ? 'Unpin records' : 'Pin records'} aria-label={state.historyPinned ? 'Unpin records' : 'Pin records'} aria-pressed={state.historyPinned} onClick={() => void command('window_pin', { region: 'history', pinned: !state.historyPinned })}><HistoryIcon name={state.historyPinned ? 'pin.fill' : 'pin'} /></button>
    <button type="button" className="icon-button records-details" title={view === 'clipboard' ? 'Details unavailable' : state.translationVisible ? 'Collapse details' : 'Expand details'} aria-label={view === 'clipboard' ? 'Details unavailable' : state.translationVisible ? 'Collapse details' : 'Expand details'} disabled={view === 'clipboard'} aria-expanded={view === 'translation' && state.translationVisible} onClick={() => void command('records_details_toggle')}><HistoryIcon name={state.translationVisible && view === 'translation' ? 'chevron.left' : 'chevron.right'} /></button>
  </header>;
}
