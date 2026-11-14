import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { historySamples, type HistorySnapshot } from './history';

export function useHistory(native: boolean) {
  const [snapshot, setSnapshot] = useState<HistorySnapshot>({ revision: -1, records: native ? [] : historySamples, selectedId: null, totalRecords: native ? 0 : historySamples.length, hasMore: false, generation: 0 });
  const [error, setError] = useState(false);
  const [connecting, setConnecting] = useState(native);
  const [querying, setQuerying] = useState(false);
  const [quietQuery, setQuietQuery] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const active = useRef(false);
  const lifecycle = useRef(0);
  const revision = useRef(-1);
  const latest = useRef(snapshot);
  const queryRequest = useRef(0);
  const moreRequest = useRef(false);
  const queryPending = useRef(false);

  const accept = useCallback((next: HistorySnapshot) => {
    if (!active.current || next.revision <= revision.current) return;
    revision.current = next.revision;
    latest.current = next;
    setSnapshot(next);
    setConnecting(false);
  }, []);

  useEffect(() => {
    active.current = true;
    const connection = ++lifecycle.current;
    if (!native) return () => { active.current = false; };
    let disposed = false;
    let unlisten: (() => void) | undefined;
    async function connect() {
      const remove = await listen<HistorySnapshot>('history_changed', ({ payload }) => {
        if (!disposed) accept(payload);
      });
      if (disposed) { remove(); return; }
      unlisten = remove;
      const initial = await invoke<HistorySnapshot>('history_snapshot');
      if (!disposed) accept(initial);
    }
    void connect().catch(() => { if (!disposed) { setError(true); setConnecting(false); } });
    return () => {
      disposed = true;
      if (lifecycle.current === connection) active.current = false;
      unlisten?.();
    };
  }, [native, accept]);

  const setQuery = useCallback(async (query: string, favoritesOnly: boolean, quiet = false) => {
    if (!native || !active.current) return;
    const request = ++queryRequest.current;
    const connection = lifecycle.current;
    queryPending.current = true;
    setError(false); setQuietQuery(quiet); setQuerying(true);
    try {
      const next = await invoke<HistorySnapshot>('history_query', { query, favoritesOnly });
      if (active.current && lifecycle.current === connection && request === queryRequest.current) accept(next);
    } catch {
      if (active.current && lifecycle.current === connection && request === queryRequest.current) setError(true);
    } finally {
      if (active.current && lifecycle.current === connection && request === queryRequest.current) { queryPending.current = false; setQuerying(false); }
    }
  }, [native, accept]);

  const loadMore = useCallback(async () => {
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

  return { snapshot, error, loading: connecting || querying || loadingMore, showLoading: connecting || (querying && !quietQuery) || loadingMore, setQuery, loadMore, update };
}
