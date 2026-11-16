import { useLayoutEffect, useRef, useState, type RefObject } from 'react';

export function useRecordBrowser(session: number | undefined, active: boolean, list: RefObject<HTMLDivElement | null>, pinnedList: RefObject<HTMLDivElement | null>, dataReady: () => boolean = () => true, contentVersion: () => unknown = () => undefined) {
  const [query, setQuery] = useState('');
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const positions = useRef(new Map<boolean, { top: number }>());
  const restoring = useRef<{ top: number } | null>(null);
  const previous = useRef({ query, favoritesOnly, active, session });
  const anchor = useRef<{ id: string; offset: number } | null>(null);
  const previousContent = useRef<unknown>(undefined);
  function remember() {
    if (restoring.current && !dataReady()) return;
    restoring.current = null;
    const element = list.current;
    positions.current.set(favoritesOnly, { top: element?.scrollTop ?? 0 });
    const row = element && [...element.querySelectorAll<HTMLElement>('.history-items > .history-row:not([hidden])')].find((row) => row.offsetTop + row.offsetHeight > element.scrollTop + (pinnedList.current?.offsetHeight ?? 0));
    anchor.current = element && element.scrollTop > 0 && row ? { id: row.dataset.recordId!, offset: row.offsetTop - element.scrollTop } : null;
  }
  useLayoutEffect(() => {
    const old = previous.current;
    const reset = old.session !== session;
    const changedQuery = old.query !== query;
    if (reset || changedQuery) { positions.current.clear(); anchor.current = null; restoring.current = null; }
    if (reset) { setQuery(''); setFavoritesOnly(false); }
    if (active && (reset || changedQuery || old.favoritesOnly !== favoritesOnly || !old.active)) {
      const saved = reset || changedQuery ? undefined : positions.current.get(favoritesOnly);
      restoring.current = saved ?? { top: 0 };
      anchor.current = null;
    } else if (active && previousContent.current !== contentVersion() && anchor.current && list.current) {
      const row = [...list.current.querySelectorAll<HTMLElement>('.history-items > .history-row:not([hidden])')].find((row) => row.dataset.recordId === anchor.current?.id);
      if (row) list.current.scrollTop = row.offsetTop - anchor.current.offset;
    }
    if (active && restoring.current && (dataReady() || restoring.current.top === 0)) {
      const saved = restoring.current;
      list.current?.scrollTo({ top: saved.top });
      if (dataReady() && (list.current?.scrollHeight ?? 0) - (list.current?.clientHeight ?? 0) >= saved.top) restoring.current = null;
    }
    previous.current = { query, favoritesOnly, active, session };
    previousContent.current = contentVersion();
  });
  function changeQuery(value: string) { remember(); setQuery(value); }
  function toggleFavorites() { remember(); setFavoritesOnly((value) => !value); }
  return { query, setQuery: changeQuery, favoritesOnly, setFavoritesOnly, toggleFavorites, remember };
}
