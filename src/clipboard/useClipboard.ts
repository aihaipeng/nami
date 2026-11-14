import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface ClipboardRecord {
  id: string;
  kind: 'text' | 'image' | 'files';
  text: string;
  date: string;
  isFavorite: boolean;
  isPinned: boolean;
  sourceApp?: { name: string; iconDataUrl?: string | null } | null;
  imagePath?: string | null;
}
export interface ClipboardCursor { pinned: boolean; date: string; sequence: number }
export interface ClipboardPage { records: ClipboardRecord[]; cursor: ClipboardCursor | null; hasMore: boolean; totalRecords: number }
const emptyPage: ClipboardPage = { records: [], cursor: null, hasMore: false, totalRecords: 0 };

export function useClipboard(native: boolean, query: string, favoritesOnly: boolean) {
  const [page, setPage] = useState(emptyPage);
  const [loading, setLoading] = useState(native);
  const [error, setError] = useState(false);
  const [captureError, setCaptureError] = useState(false);
  const [connection, setConnection] = useState(0);
  const active = useRef(false);
  const ready = useRef(false);
  const lifecycle = useRef(0);
  const request = useRef(0);
  const latest = useRef(page);
  const filters = useRef({ query, favoritesOnly });
  const loadedFilters = useRef({ query, favoritesOnly });
  const morePending = useRef(false);
  const resetPending = useRef(false);

  const fetchPage = useCallback(async (append = false) => {
    if (!native || !active.current || !ready.current || (append && (morePending.current || resetPending.current || loadedFilters.current.query !== filters.current.query || loadedFilters.current.favoritesOnly !== filters.current.favoritesOnly || !latest.current.hasMore))) return;
    const current = append ? request.current : ++request.current;
    const cursor = append ? latest.current.cursor : null;
    const queryFilters = { ...filters.current };
    if (append) morePending.current = true;
    else { resetPending.current = true; morePending.current = false; }
    setLoading(true); setError(false);
    try {
      const next = await invoke<ClipboardPage>('clipboard_query', { ...queryFilters, cursor });
      if (!active.current || current !== request.current) return;
      const merged = append ? { ...next, records: [...latest.current.records, ...next.records.filter((record) => !latest.current.records.some(({ id }) => id === record.id))] } : next;
      if (!append) loadedFilters.current = queryFilters;
      setCaptureError(false);
      latest.current = merged;
      setPage(merged);
    } catch {
      if (active.current && current === request.current) setError(true);
    } finally {
      if (active.current && current === request.current) {
        if (append) morePending.current = false;
        else resetPending.current = false;
        setLoading(false);
      }
    }
  }, [native]);

  useEffect(() => {
    active.current = true;
    ++lifecycle.current;
    morePending.current = false;
    let disposed = false;
    const removers: (() => void)[] = [];
    if (native) void (async () => {
      for (const subscribe of [
        () => listen<number>('clipboard_changed', () => { if (!disposed) { setCaptureError(false); void fetchPage(); } }),
        () => listen<string>('clipboard_capture_error', () => { if (!disposed) setCaptureError(true); }),
      ]) {
        const remove = await subscribe();
        if (disposed) { remove(); return; }
        removers.push(remove);
      }
      ready.current = true;
      await fetchPage();
    })().catch(() => { if (!disposed) { setError(true); setLoading(false); } });
    return () => { disposed = true; active.current = false; ready.current = false; ++request.current; removers.forEach((remove) => remove()); };
  }, [native, fetchPage, connection]);

  useEffect(() => {
    const favoritesChanged = filters.current.favoritesOnly !== favoritesOnly;
    if (filters.current.query === query && !favoritesChanged) return;
    filters.current = { query, favoritesOnly };
    resetPending.current = true;
    ++request.current;
    if (!ready.current) return;
    if (favoritesChanged) { void fetchPage(); return; }
    setLoading(true);
    const timer = window.setTimeout(() => { void fetchPage(); }, 150);
    return () => window.clearTimeout(timer);
  }, [query, favoritesOnly, fetchPage]);

  const update = useCallback(async (recordId: string, action: 'favorite' | 'pin' | 'delete') => {
    if (!native || !active.current) return;
    const session = lifecycle.current;
    setError(false);
    try {
      await invoke('clipboard_record_update', { recordId, action });
      if (active.current && session === lifecycle.current) await fetchPage();
    } catch { if (active.current && session === lifecycle.current) setError(true); }
  }, [native, fetchPage]);

  return { page, loading, error: error || captureError, update, loadMore: () => fetchPage(true), refresh: () => { if (ready.current) void fetchPage(); else setConnection((value) => value + 1); } };
}
