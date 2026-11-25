import type { RefObject } from 'react';
import type { ViewProps } from '../App';
import { HistoryIcon } from '../translation/HistoryIcon';
import { HistoryWordmark } from './HistoryWordmark';

export function RecordToolbar({ t, snapshot, command, query, onQuery, search, onDetailsToggle }: Pick<ViewProps, 't' | 'snapshot' | 'command'> & {
  query: string; onQuery: (value: string) => void;
  search: RefObject<HTMLInputElement | null>; onDetailsToggle?: () => void;
}) {
  const state = snapshot.window;
  const searchLabel = t('searchClipboard');
  return <header className="history-toolbar" onMouseDown={(event) => {
    if ((event.target as HTMLElement).closest('button')) event.preventDefault();
  }}>
    <HistoryWordmark />
    <label className="history-search"><HistoryIcon name="magnifyingglass" /><input ref={search} type="search" aria-label={searchLabel} placeholder={searchLabel} value={query} onChange={(event) => onQuery(event.target.value)} />
      <button type="button" hidden={!query} aria-label={t('clearClipboardSearch')} onClick={() => { onQuery(''); search.current?.focus(); }}><HistoryIcon name="xmark.circle.fill" /></button>
    </label>
    <button type="button" className="icon-button history-pin" title={state.clipboardPinned ? 'Unpin records' : 'Pin records'} aria-label={state.clipboardPinned ? 'Unpin records' : 'Pin records'} aria-pressed={state.clipboardPinned} onClick={() => void command('window_pin', { region: 'clipboard', pinned: !state.clipboardPinned })}><HistoryIcon name={state.clipboardPinned ? 'pin.fill' : 'pin'} /></button>
    <button type="button" className="icon-button records-details" title={state.clipboardDetailVisible ? 'Collapse details' : 'Expand details'} aria-label={state.clipboardDetailVisible ? 'Collapse details' : 'Expand details'} aria-expanded={state.clipboardDetailVisible} onClick={onDetailsToggle ?? (() => { void command('records_details_toggle'); })}><HistoryIcon name={state.clipboardDetailVisible ? 'chevron.left' : 'chevron.right'} /></button>
  </header>;
}
