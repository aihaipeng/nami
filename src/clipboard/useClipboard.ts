import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface ClipboardRecord {
  title?: string;
  id: string;
  kind: 'text' | 'image' | 'files';
  text: string;
  date: string;
  isFavorite: boolean;
  isPinned: boolean;
  sourceApp?: { name: string; iconDataUrl?: string | null } | null;
  imagePath?: string | null;
}
export type ClipboardKind = ClipboardRecord['kind'];
const cacheKey = (kind: ClipboardKind | null, favorites: boolean) => `${kind ?? 'all'}:${favorites}`;
const matchesCache = (record: ClipboardRecord, key: string) => key === cacheKey(record.kind, false) || (record.isFavorite && key === cacheKey(record.kind, true)) || key === cacheKey(null, false) || (record.isFavorite && key === cacheKey(null, true));
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
function placeCopied(records: ClipboardRecord[], mark: { id: string; date: string }) {
  const index = records.findIndex((record) => record.id === mark.id);
  if (index < 0) return records;
  const current = records[index];
  const record = current.date === mark.date ? current : { ...current, date: mark.date };
  let insert = 0;
  if (!record.isPinned) while (insert < records.length && records[insert].isPinned) insert += 1;
  if (insert === index && record === current) return records;
  const next = records.slice();
  next.splice(index, 1);
  next.splice(insert > index ? insert - 1 : insert, 0, record);
  return next;
}

// Keep local copy order only until the server acknowledges the changed copy date.
// A later pin can lead the group while retaining its older display date.
function retainCopied(records: ClipboardRecord[], mark: { id: string; date: string; previousDate: string | undefined }) {
  const index = records.findIndex((record) => record.id === mark.id);
  if (index < 0) return { records, drop: false };
  let head = 0;
  while (head < records.length && records[head].isPinned !== records[index].isPinned) head += 1;
  if (head === index || records[index].date !== mark.previousDate || Date.parse(records[head].date) >= Date.parse(mark.date)) return { records, drop: true };
  return { records: placeCopied(records, mark), drop: false };
}

function insertByLastCopy(records: ClipboardRecord[], record: ClipboardRecord) {
  if (record.isPinned) { records.unshift(record); return; }
  const time = Date.parse(record.date);
  let index = 0;
  while (index < records.length && records[index].isPinned) index += 1;
  while (index < records.length && Date.parse(records[index].date) >= time) index += 1;
  records.splice(index, 0, record);
}

export function useClipboard(native: boolean, query: string, favoritesOnly: boolean, deferRefresh?: () => boolean, kind: ClipboardKind | null = null) {
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
  const filters = useRef({ query, favoritesOnly, kind });
  const loadedFilters = useRef({ query, favoritesOnly, kind });
  const morePending = useRef(false);
  const resetPending = useRef(false);
  const pages = useRef(new Map<string, ClipboardPage>());
  const invalidPages = useRef(new Map<string, number>());
  const defaultPage = useRef<{ page: ClipboardPage; revision: number } | null>(null);
  const defaultRevision = useRef(0);
  const deferred = useRef(false);
  const localWrites = useRef(0);
  const writeQueue = useRef(Promise.resolve());
  const copiedMark = useRef<{ id: string; date: string; previousDate: string | undefined } | null>(null);
  const copyEpoch = useRef(0);
  const shouldDefer = useRef(deferRefresh);
  shouldDefer.current = deferRefresh;

  const fetchPage = useCallback(async (append = false, animate = true) => {
    if (!native || !active.current || !ready.current || (append && (morePending.current || resetPending.current || loadedFilters.current.query !== filters.current.query || loadedFilters.current.favoritesOnly !== filters.current.favoritesOnly || loadedFilters.current.kind !== filters.current.kind || !latest.current.hasMore))) return;
    const current = append ? request.current : ++request.current;
    const epochAtStart = copyEpoch.current;
    const cursor = append ? latest.current.cursor : null;
    const queryFilters = { ...filters.current };
    const cacheRevision = defaultRevision.current;
    if (append) morePending.current = true;
    else { resetPending.current = true; morePending.current = false; }
    setLoading(true); setShowActivity(animate); setError(false);
    try {
      const result = await invoke<{ page: ClipboardWirePage; alternate: ClipboardWirePage | null }>('clipboard_query', { query: queryFilters.query, favoritesOnly: queryFilters.favoritesOnly, ...(queryFilters.kind ? { kind: queryFilters.kind } : {}), cursor });
      let next = hydrate(result.page);
      const reloadCount = append ? 0 : invalidPages.current.get(cacheKey(queryFilters.kind, queryFilters.favoritesOnly)) ?? 0;
      while (next.hasMore && next.records.length < reloadCount) {
        if (!active.current || current !== request.current) return;
        const continuation = await invoke<{ page: ClipboardWirePage }>('clipboard_query', { query: queryFilters.query, favoritesOnly: queryFilters.favoritesOnly, ...(queryFilters.kind ? { kind: queryFilters.kind } : {}), cursor: next.cursor });
        const page = hydrate(continuation.page);
        next = { ...page, records: [...next.records, ...page.records] };
      }
      if (!active.current || current !== request.current) return;
      const merged = append ? { ...next, records: [...latest.current.records, ...next.records.filter((record) => !latest.current.records.some(({ id }) => id === record.id))] } : next;
      if (epochAtStart === copyEpoch.current && copiedMark.current) {
        const applied = retainCopied(merged.records, copiedMark.current);
        merged.records = applied.records;
        if (applied.drop) copiedMark.current = null;
      }
      if (!append) {
        loadedFilters.current = queryFilters;
        const alternate = result.alternate ? hydrate(result.alternate) : null;
        if (alternate) {
          if (epochAtStart === copyEpoch.current && copiedMark.current) {
            const applied = retainCopied(alternate.records, copiedMark.current);
            alternate.records = applied.records;
            if (applied.drop) copiedMark.current = null;
          }
          const alternateKey = cacheKey(queryFilters.kind, !queryFilters.favoritesOnly);
          const retained = pages.current.get(alternateKey);
          if ((!retained || retained.records.length <= alternate.records.length) && alternate.records.length >= (invalidPages.current.get(alternateKey) ?? 0)) {
            pages.current.set(alternateKey, alternate); invalidPages.current.delete(alternateKey);
          }
        }
        const home = queryFilters.favoritesOnly ? alternate : next;
        if (!queryFilters.query && !queryFilters.kind && home && home.records.length <= 100 && cacheRevision === defaultRevision.current) defaultPage.current = { page: home, revision: cacheRevision };
      }
      pages.current.set(cacheKey(queryFilters.kind, queryFilters.favoritesOnly), merged);
      invalidPages.current.delete(cacheKey(queryFilters.kind, queryFilters.favoritesOnly));
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
        () => listen<number>('clipboard_changed', () => { if (!disposed) { ++defaultRevision.current; setCaptureError(false); if (localWrites.current || shouldDefer.current?.()) deferred.current = true; else { pages.current.clear(); invalidPages.current.clear(); deferred.current = false; void fetchPage(false, false); } } }),
        () => listen<string>('clipboard_capture_error', () => { if (!disposed) setCaptureError(true); }),
      ]) {
        const remove = await subscribe();
        if (disposed) { remove(); return; }
        removers.push(remove);
      }
      ready.current = true;
      await fetchPage();
    })().catch(() => { if (!disposed) { setError(true); setLoading(false); setShowActivity(false); } });
    return () => { disposed = true; active.current = false; ready.current = false; pages.current.clear(); invalidPages.current.clear(); defaultPage.current = null; ++defaultRevision.current; ++request.current; removers.forEach((remove) => remove()); };
  }, [native, fetchPage, connection]);

  useEffect(() => {
    const favoritesChanged = filters.current.favoritesOnly !== favoritesOnly;
    const kindChanged = filters.current.kind !== kind;
    const queryChanged = filters.current.query !== query;
    if (filters.current.query === query && !favoritesChanged && !kindChanged) return;
    if (filters.current.query !== query) { pages.current.clear(); invalidPages.current.clear(); }
    filters.current = { query, favoritesOnly, kind };
    resetPending.current = true;
    ++request.current;
    if (!ready.current) return;
    const cached = (!queryChanged && !invalidPages.current.has(cacheKey(kind, favoritesOnly)) ? pages.current.get(cacheKey(kind, favoritesOnly)) : undefined)
      ?? (!query && !kind && !favoritesOnly && defaultPage.current?.revision === defaultRevision.current ? defaultPage.current.page : undefined);
    if (cached) {
      latest.current = cached;
      loadedFilters.current = { query, favoritesOnly, kind };
      pages.current.set(cacheKey(kind, favoritesOnly), cached);
      resetPending.current = false;
      morePending.current = false;
      setPage(cached); setLoading(false); setShowActivity(false); setError(false);
      return;
    }
    if (!queryChanged && (favoritesChanged || kindChanged)) {
      // Show known matches immediately; the database still supplies the complete first page.
      const preview = { ...latest.current, records: latest.current.records.filter((record) => (!favoritesOnly || record.isFavorite) && (!kind || record.kind === kind)), cursor: null, hasMore: false };
      latest.current = preview;
      setPage(preview);
      void fetchPage(false, false);
      return;
    }
    if (!query) { void fetchPage(false, false); return; }
    setLoading(true); setShowActivity(false);
    const timer = window.setTimeout(() => { void fetchPage(); }, 150);
    return () => window.clearTimeout(timer);
  }, [query, favoritesOnly, kind, fetchPage]);

  const update = useCallback(async (recordId: string, action: 'favorite' | 'pin' | 'delete') => {
    if (!native || !active.current) return;
    const session = lifecycle.current;
    ++defaultRevision.current;
    const pending = writeQueue.current;
    let release!: () => void;
    writeQueue.current = new Promise<void>((resolve) => { release = resolve; });
    ++localWrites.current;
    await pending;
    if (!active.current || session !== lifecycle.current) { --localWrites.current; release(); return; }
    const previous = latest.current.records.find((record) => record.id === recordId);
    setError(false);
    try {
      await invoke('clipboard_record_update', { recordId, action });
      if (!active.current || session !== lifecycle.current) return;
      if (action === 'pin') copiedMark.current = null;
      if (action === 'delete' && defaultPage.current) defaultPage.current.page = { ...defaultPage.current.page, records: defaultPage.current.page.records.filter((record) => record.id !== recordId) };
      if (!shouldDefer.current?.()) {
        pages.current.clear(); invalidPages.current.clear(); deferred.current = false; void fetchPage(false, false);
      } else if (previous) {
        const updated = action === 'delete' ? null : { ...previous, ...(action === 'favorite' ? { isFavorite: !previous.isFavorite } : { isPinned: !previous.isPinned }) };
        const pinOrder = new Map(latest.current.records.filter((record) => record.isPinned).map((record, index) => [record.id, index]));
        for (const [key, cached] of pages.current) {
          const before = matchesCache(previous, key);
          const after = updated !== null && matchesCache(updated, key);
          const totalRecords = cached.totalRecords + Number(after) - Number(before);
          if (invalidPages.current.has(key)) {
            invalidPages.current.set(key, invalidPages.current.get(key)! + Number(after) - Number(before));
            pages.current.set(key, { ...cached, totalRecords }); continue;
          }
          const records = cached.records.filter((record) => record.id !== recordId);
          if (after && updated) {
            if (action === 'pin') insertByLastCopy(records, updated);
            else {
              const oldIndex = cached.records.findIndex((record) => record.id === recordId);
              if (oldIndex < 0 && updated.isPinned) {
                if (!pinOrder.has(recordId) || records.some((record) => record.isPinned && !pinOrder.has(record.id))) {
                  invalidPages.current.set(key, cached.records.length + Number(after) - Number(before));
                  pages.current.set(key, { ...cached, totalRecords }); continue;
                }
                const index = records.findIndex((record) => !record.isPinned || pinOrder.get(record.id)! > pinOrder.get(recordId)!);
                records.splice(index < 0 ? records.length : index, 0, updated);
              } else records.splice(oldIndex < 0 ? records.length : oldIndex, 0, updated);
            }
          }
          pages.current.set(key, { ...cached, records, totalRecords });
        }
        const next = pages.current.get(cacheKey(filters.current.kind, filters.current.favoritesOnly));
        if (next) { latest.current = next; setPage(next); }
      }
    } catch { if (active.current && session === lifecycle.current) setError(true); }
    finally { --localWrites.current; release(); }
  }, [native, fetchPage]);

  const noteCopied = useCallback((recordId: string) => {
    copyEpoch.current += 1;
    const mark = { id: recordId, date: new Date().toISOString(), previousDate: latest.current.records.find(({ id }) => id === recordId)?.date };
    copiedMark.current = mark;
    const reorder = (cached: ClipboardPage): ClipboardPage => ({ ...cached, records: placeCopied(cached.records, mark) });
    for (const [key, cached] of pages.current) pages.current.set(key, reorder(cached));
    if (defaultPage.current) defaultPage.current = { ...defaultPage.current, page: reorder(defaultPage.current.page) };
    latest.current = reorder(latest.current);
    setPage(latest.current);
  }, []);

  function applyEdit(recordId: string, detail: { title: string; text: string }) {
    ++defaultRevision.current;
    const previous = latest.current.records.find((record) => record.id === recordId);
    if (!active.current || !previous) return;
    const matches = detail.text.toLowerCase().includes(filters.current.query.trim().toLowerCase());
    const updated = { ...previous, title: detail.title, text: [...detail.text].slice(0, 256).join('') };
    if (defaultPage.current) defaultPage.current.page = { ...defaultPage.current.page, records: defaultPage.current.page.records.map((record) => record.id === recordId ? updated : record) };
    const patch = (cached: ClipboardPage, key: string): ClipboardPage => {
      const before = matchesCache(previous, key);
      const after = before && matches;
      const records = cached.records.flatMap((record) => record.id !== recordId ? [record] : after ? [updated] : []);
      const totalRecords = cached.totalRecords + Number(after) - Number(before);
      return { ...cached, records, totalRecords, hasMore: cached.hasMore && records.length < totalRecords };
    };
    for (const [key, cached] of pages.current) pages.current.set(key, patch(cached, key));
    latest.current = pages.current.get(cacheKey(filters.current.kind, filters.current.favoritesOnly)) ?? patch(latest.current, cacheKey(filters.current.kind, filters.current.favoritesOnly));
    setPage(latest.current);
  }

  const restoringDefault = !query && !kind && !favoritesOnly && (loadedFilters.current.query !== query || loadedFilters.current.favoritesOnly !== favoritesOnly || loadedFilters.current.kind !== kind);
  const home = defaultPage.current?.revision === defaultRevision.current ? defaultPage.current.page : null;
  const displayedPage = restoringDefault ? home ?? emptyPage : filters.current.query === query && (loadedFilters.current.favoritesOnly !== favoritesOnly || loadedFilters.current.kind !== kind) ? (!invalidPages.current.has(cacheKey(kind, favoritesOnly)) ? pages.current.get(cacheKey(kind, favoritesOnly)) : undefined) ?? page : page;
  const displayedIds = new Set(displayedPage.records.map(({ id }) => id));
  const retained = [...(defaultPage.current?.page.records ?? []), ...[...pages.current.values()].flatMap((cached) => cached.records)];
  const retainedRecords = [...displayedPage.records, ...new Map(retained.filter(({ id }) => !displayedIds.has(id)).map((record) => [record.id, record])).values()];
  const resetFilters = useCallback((next: { query: string; favoritesOnly: boolean; kind: ClipboardKind | null }) => {
    filters.current = next;
    pages.current.clear(); invalidPages.current.clear();
    defaultPage.current = null;
    ++defaultRevision.current;
    deferred.current = false;
    resetPending.current = true;
    morePending.current = false;
    if (ready.current) void fetchPage(false, false);
  }, [fetchPage]);
  return { applyEdit, noteCopied, page: displayedPage, displayReady: Boolean(restoringDefault && home) || loadedFilters.current.query === query && ((loadedFilters.current.favoritesOnly === favoritesOnly && loadedFilters.current.kind === kind) || !invalidPages.current.has(cacheKey(kind, favoritesOnly)) && pages.current.has(cacheKey(kind, favoritesOnly))), retainedRecords, loading: restoringDefault && home ? false : loading, showActivity: restoringDefault ? false : showActivity, error: error || captureError, update, resetFilters, refreshDeferred: () => { if (deferred.current && !shouldDefer.current?.()) { deferred.current = false; pages.current.clear(); invalidPages.current.clear(); void fetchPage(false, false); } }, loadMore: () => fetchPage(true), refresh: () => { ++defaultRevision.current; if (ready.current) void fetchPage(); else setConnection((value) => value + 1); } };
}
