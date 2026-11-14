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
export interface ClipboardCursor { pinned: boolean; createdAt: number; sequence: number }
export interface ClipboardPage { records: ClipboardRecord[]; cursor: ClipboardCursor | null; hasMore: boolean; totalRecords: number }
interface ClipboardWirePage extends Omit<ClipboardPage, 'records'> {
  records: (Omit<ClipboardRecord, 'sourceApp' | 'imagePath'> & { sourceAppId: number | null })[];
  sources: Record<string, NonNullable<ClipboardRecord['sourceApp']>>;
}
function hydrate(page: ClipboardWirePage): ClipboardPage {
  return { ...page, records: page.records.map(({ sourceAppId, ...record }) => ({ ...record, sourceApp: sourceAppId === null ? null : page.sources[sourceAppId] })) };
}
const emptyPage: ClipboardPage = { records: [], cursor: null, hasMore: false, totalRecords: 0 };

export function useClipboard(native: boolean, query: string, favoritesOnly: boolean) {
  const [page, setPage] = useState(emptyPage);
  const [loading, setLoading] = useState(native);
  const [showActivity, setShowActivity] = useState(native);
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
  const pages = useRef(new Map<boolean, ClipboardPage>());

  const fetchPage = useCallback(async (append = false, animate = true) => {
    if (!native || !active.current || !ready.current || (append && (morePending.current || resetPending.current || loadedFilters.current.query !== filters.current.query || loadedFilters.current.favoritesOnly !== filters.current.favoritesOnly || !latest.current.hasMore))) return;
    const current = append ? request.current : ++request.current;
    const cursor = append ? latest.current.cursor : null;
    const queryFilters = { ...filters.current };
    if (append) morePending.current = true;
    else { resetPending.current = true; morePending.current = false; }
    setLoading(true); setShowActivity(animate); setError(false);
    try {
      const result = await invoke<{ page: ClipboardWirePage; alternate: ClipboardWirePage | null }>('clipboard_query', { ...queryFilters, cursor });
      const next = hydrate(result.page);
      if (!active.current || current !== request.current) return;
      const merged = append ? { ...next, records: [...latest.current.records, ...next.records.filter((record) => !latest.current.records.some(({ id }) => id === record.id))] } : next;
      if (!append) {
        loadedFilters.current = queryFilters;
        pages.current.set(queryFilters.favoritesOnly, next);
        if (result.alternate) pages.current.set(!queryFilters.favoritesOnly, hydrate(result.alternate));
      }
      setCaptureError(false);
      latest.current = merged;
      setPage(merged);
    } catch {
      if (active.current && current === request.current) setError(true);
    } finally {
      if (active.current && current === request.current) {
        if (append) morePending.current = false;
        else resetPending.current = false;
        setLoading(false); setShowActivity(false);
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
        () => listen<number>('clipboard_changed', () => { if (!disposed) { setCaptureError(false); pages.current.clear(); void fetchPage(false, false); } }),
        () => listen<string>('clipboard_capture_error', () => { if (!disposed) setCaptureError(true); }),
      ]) {
        const remove = await subscribe();
        if (disposed) { remove(); return; }
        removers.push(remove);
      }
      ready.current = true;
      await fetchPage();
    })().catch(() => { if (!disposed) { setError(true); setLoading(false); setShowActivity(false); } });
    return () => { disposed = true; active.current = false; ready.current = false; pages.current.clear(); ++request.current; removers.forEach((remove) => remove()); };
  }, [native, fetchPage, connection]);

  useEffect(() => {
    const favoritesChanged = filters.current.favoritesOnly !== favoritesOnly;
    if (filters.current.query === query && !favoritesChanged) return;
    if (filters.current.query !== query) pages.current.clear();
    filters.current = { query, favoritesOnly };
    resetPending.current = true;
    ++request.current;
    if (!ready.current) return;
    if (favoritesChanged) {
      const cached = pages.current.get(favoritesOnly);
      if (cached) {
        latest.current = cached;
        loadedFilters.current = { query, favoritesOnly };
        resetPending.current = false;
        morePending.current = false;
        setPage(cached); setLoading(false); setShowActivity(false); setError(false);
        return;
      }
      // Show known matches immediately; the database still supplies the complete first page.
      const preview = { ...latest.current, records: latest.current.records.filter((record) => !favoritesOnly || record.isFavorite), cursor: null, hasMore: false };
      latest.current = preview;
      setPage(preview);
      void fetchPage(false, false);
      return;
    }
    setLoading(true); setShowActivity(true);
    const timer = window.setTimeout(() => { void fetchPage(); }, 150);
    return () => window.clearTimeout(timer);
  }, [query, favoritesOnly, fetchPage]);

  const update = useCallback(async (recordId: string, action: 'favorite' | 'pin' | 'delete') => {
    if (!native || !active.current) return;
    const session = lifecycle.current;
    setError(false);
    try {
      await invoke('clipboard_record_update', { recordId, action });
    } catch { if (active.current && session === lifecycle.current) setError(true); }
  }, [native]);

  const displayedPage = filters.current.query === query && loadedFilters.current.favoritesOnly !== favoritesOnly ? pages.current.get(favoritesOnly) ?? page : page;
  const retainedRecords = [...new Map(displayedPage.records.concat([...pages.current.values()].flatMap((cached) => cached.records)).map((record) => [record.id, record])).values()];
  return { page: displayedPage, retainedRecords, loading, showActivity, error: error || captureError, update, loadMore: () => fetchPage(true), refresh: () => { if (ready.current) void fetchPage(); else setConnection((value) => value + 1); } };
}
