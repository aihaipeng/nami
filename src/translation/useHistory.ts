import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { historySamples, type HistorySelection, type HistorySnapshot } from './history';

type HistoryPage = Pick<HistorySnapshot, 'records' | 'totalRecords' | 'hasMore'>;

export function useHistory(native: boolean) {
  const [snapshot, setSnapshot] = useState<HistorySnapshot>({ revision: -1, records: native ? [] : historySamples, selectedId: null, totalRecords: native ? 0 : historySamples.length, hasMore: false, generation: 0 });
  const [error, setError] = useState(false);
  const [connecting, setConnecting] = useState(native);
  const [querying, setQuerying] = useState(false);
  const [quietQuery, setQuietQuery] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [connectionKey, setConnectionKey] = useState(0);
  const ready = useRef(false);
  const retryFilters = useRef<{ query: string; favoritesOnly: boolean } | null>(null);
  const active = useRef(false);
  const lifecycle = useRef(0);
  const revision = useRef(-1);
  const selection = useRef({ revision: -1, selectedId: null as string | null });
  const latest = useRef(snapshot);
  const queryRequest = useRef(0);
  const moreRequest = useRef(false);
  const queryPending = useRef(false);
  const queryTask = useRef<{ query: string; favoritesOnly: boolean; done: Promise<boolean> } | null>(null);
  const cacheQuery = useRef('');
  const pages = useRef(new Map<boolean, HistoryPage>());
  const projection = useRef({ query: '', favoritesOnly: false, generation: 0 });

  const accept = useCallback((next: HistorySnapshot) => {
    if (!active.current || next.revision <= revision.current) return;
    // Selection has no rows, so a delayed snapshot must still supply its page data.
    const dataRevision = next.revision;
    if (selection.current.revision > next.revision) next = { ...next, ...selection.current };
    if (revision.current < 0) projection.current.generation = next.generation;
    if (next.generation === projection.current.generation && cacheQuery.current === projection.current.query) {
      // An unchanged filtered page can still hide a newly added record in the alternate view.
      pages.current.clear();
      pages.current.set(projection.current.favoritesOnly, next);
    } else if (next.generation === latest.current.generation || !queryPending.current) {
      pages.current.clear();
    }
    revision.current = dataRevision;
    latest.current = next;
    setSnapshot(next);
    setConnecting(false); setError(false);
  }, []);

  const setQuery = useCallback(async (query: string, favoritesOnly: boolean, quiet = false) => {
    if (!native || !active.current) return;
    if (!ready.current) { retryFilters.current = { query, favoritesOnly }; return; }
    const request = ++queryRequest.current;
    const connection = lifecycle.current;
    if (cacheQuery.current !== query) { pages.current.clear(); cacheQuery.current = query; }
    queryPending.current = true;
    let settled!: (ready: boolean) => void;
    queryTask.current = { query, favoritesOnly, done: new Promise<boolean>((resolve) => { settled = resolve; }) };
    let accepted = false;
    setError(false); setQuietQuery(quiet); setQuerying(true);
    try {
      const next = await invoke<HistorySnapshot>('history_query', { query, favoritesOnly });
      if (active.current && lifecycle.current === connection && request === queryRequest.current) {
        accept(next);
        if (next.generation === latest.current.generation) {
          projection.current = { query, favoritesOnly, generation: next.generation };
          pages.current.set(favoritesOnly, latest.current);
          accepted = true;
        }
      }
    } catch {
      if (active.current && lifecycle.current === connection && request === queryRequest.current) setError(true);
    } finally {
      if (active.current && lifecycle.current === connection && request === queryRequest.current) { queryPending.current = false; setQuerying(false); }
      settled(accepted && active.current && lifecycle.current === connection && request === queryRequest.current);
    }
  }, [native, accept]);

  useEffect(() => {
    active.current = true;
    const connection = ++lifecycle.current;
    if (!native) return () => { active.current = false; };
    let disposed = false;
    const unlisten: (() => void)[] = [];
    async function connect() {
      const remove = await listen<HistorySnapshot>('history_changed', ({ payload }) => { if (!disposed) accept(payload); });
      if (disposed) { remove(); return; }
      unlisten.push(remove);
      const removeSelection = await listen<HistorySelection>('history_sample_selected', ({ payload }) => {
        if (disposed || payload.revision <= Math.max(revision.current, selection.current.revision)) return;
        selection.current = { revision: payload.revision, selectedId: payload.sampleId };
        latest.current = { ...latest.current, ...selection.current };
        setSnapshot(latest.current);
      }).catch((error: unknown) => { unlisten.splice(0).forEach((remove) => remove()); throw error; });
      if (disposed) { removeSelection(); return; }
      unlisten.push(removeSelection); ready.current = true;
      const retry = retryFilters.current; retryFilters.current = null;
      if (retry) await setQuery(retry.query, retry.favoritesOnly);
      else {
        const initial = await invoke<HistorySnapshot>('history_snapshot');
        if (!disposed) accept(initial);
      }
      if (!disposed) setConnecting(false);
    }
    void connect().catch(() => { if (!disposed) { setError(true); setConnecting(false); } });
    return () => {
      disposed = true;
      if (lifecycle.current === connection) { active.current = false; ready.current = false; pages.current.clear(); }
      unlisten.splice(0).forEach((remove) => remove());
    };
  }, [native, accept, setQuery, connectionKey]);

  const loadMore = useCallback(async (text?: string, favoritesOnly?: boolean) => {
    if (text !== undefined && (projection.current.query !== text || projection.current.favoritesOnly !== favoritesOnly)) return;
    if (!native || !active.current || !latest.current.hasMore || moreRequest.current || queryPending.current) return;
    const query = queryRequest.current;
    const generation = latest.current.generation;
    const connection = lifecycle.current;
    moreRequest.current = true; setLoadingMore(true); setError(false);
    try {
      const next = await invoke<HistorySnapshot>('history_load_more');
      if (active.current && lifecycle.current === connection && query === queryRequest.current && generation === next.generation) accept(next);
    } catch {
      if (active.current && lifecycle.current === connection && query === queryRequest.current) setError(true);
    } finally {
      moreRequest.current = false;
      if (active.current && lifecycle.current === connection) setLoadingMore(false);
    }
  }, [native, accept]);

  const update = useCallback(async (recordId: string, action: 'favorite' | 'pin' | 'delete') => {
    if (!native) return;
    const connection = lifecycle.current;
    setError(false);
    try {
      const next = await invoke<HistorySnapshot>('history_record_update', { recordId, action });
      if (active.current && lifecycle.current === connection) accept(next);
    } catch { if (active.current && lifecycle.current === connection) setError(true); }
  }, [native, accept]);

  function refresh(query: string, favoritesOnly: boolean) {
    if (ready.current) { void setQuery(query, favoritesOnly); return; }
    retryFilters.current = { query, favoritesOnly };
    setError(false); setConnecting(native); setConnectionKey((value) => value + 1);
  }

  function displaySnapshot(query: string, favoritesOnly: boolean): HistorySnapshot {
    const cached = cacheQuery.current === query ? pages.current.get(favoritesOnly) : undefined;
    return cached ? { ...snapshot, records: cached.records, totalRecords: cached.totalRecords, hasMore: cached.hasMore } : snapshot;
  }
  function whenReady(query: string, favoritesOnly: boolean, recordId: string): boolean | Promise<boolean> {
    const current = () => active.current && ready.current && !queryPending.current && projection.current.query === query && projection.current.favoritesOnly === favoritesOnly && latest.current.records.some((record) => record.id === recordId);
    if (!queryPending.current) return current();
    const task = queryTask.current;
    return task?.query === query && task.favoritesOnly === favoritesOnly ? task.done.then((accepted) => accepted && current()) : false;
  }
  return { snapshot, displaySnapshot, displayReady: (query: string, favoritesOnly: boolean) => !connecting && cacheQuery.current === query && pages.current.has(favoritesOnly), whenReady, error, loading: connecting || querying || loadingMore, showLoading: connecting || (querying && !quietQuery) || loadingMore, refresh, setQuery, loadMore, update };
}
