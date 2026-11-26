import { useEffect, useId, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject, type UIEvent } from 'react';
import {
  averageRowHeight,
  buildOffsets,
  computeScrollTargetForIndex,
  computeVirtualWindow,
  ROW_GAP_PX,
  VIRTUALIZE_THRESHOLD,
} from './virtualList';

export interface HistoryListHandle {
  scrollToId: (id: string, behavior?: ScrollBehavior) => boolean;
}

export interface VirtualItemsConfig<T> {
  items: readonly T[];
  allItems?: readonly T[];
  itemKey: (item: T) => string;
  renderItem: (item: T) => ReactNode;
  resetKey?: unknown;
  threshold?: number;
}

// Clip ordinary rows below the transparent pinned region; keep one full-height scrollbar.
export function HistoryList<T = never>({
  list,
  pinnedList,
  pinned,
  pinnedCount = pinned.length,
  children,
  virtualItems,
  handleRef,
  onNearEnd,
  onScroll,
  onScrollOwnerChange,
  active = true,
  scrollOnly = false,
}: {
  list: RefObject<HTMLDivElement | null>;
  pinnedList: RefObject<HTMLDivElement | null>;
  pinned: ReactNode[];
  pinnedCount?: number;
  children?: ReactNode;
  virtualItems?: VirtualItemsConfig<T>;
  handleRef?: RefObject<HistoryListHandle | null>;
  onNearEnd: (element: HTMLDivElement) => void;
  onScroll?: () => void;
  onScrollOwnerChange?: () => void;
  active?: boolean;
  scrollOnly?: boolean;
}) {
  const listId = useId();
  const [scrollbarVisible, setScrollbarVisible] = useState(false);
  const idle = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const dragging = useRef(false);
  const focused = useRef(false);
  const enabled = useRef(active);
  enabled.current = active;
  const dragCleanup = useRef<(() => void) | undefined>(undefined);
  function showScrollbar() {
    if (!enabled.current) return;
    window.clearTimeout(idle.current);
    setScrollbarVisible(true);
    idle.current = window.setTimeout(() => {
      if (!dragging.current && !focused.current) setScrollbarVisible(false);
    }, 1000);
  }
  useEffect(() => () => { window.clearTimeout(idle.current); dragCleanup.current?.(); }, []);
  useLayoutEffect(() => {
    window.clearTimeout(idle.current);
    dragging.current = false; focused.current = false;
    dragCleanup.current?.();
    setScrollbarVisible(false);
  }, [active]);
  useLayoutEffect(() => () => { list.current = null; }, [list]);

  const viewport = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const itemsRef = useRef<HTMLDivElement>(null);
  const [scroll, setScroll] = useState({ top: 0, height: 0, content: 0, track: 0, overflow: false });
  const heightsRef = useRef(new Map<string, number>());
  const [, setHeightRevision] = useState(0);
  const resetKey = virtualItems?.resetKey;
  const previousResetKey = useRef(resetKey);
  if (previousResetKey.current !== resetKey) {
    previousResetKey.current = resetKey;
    heightsRef.current.clear();
  }

  const visibleList = virtualItems?.items;
  const itemKey = virtualItems?.itemKey;
  const ids = useMemo(
    () => (visibleList && itemKey ? visibleList.map(itemKey) : []),
    [visibleList, itemKey],
  );
  const idToIndex = useMemo(() => {
    const map = new Map<string, number>();
    for (let i = 0; i < ids.length; i += 1) map.set(ids[i], i);
    return map;
  }, [ids]);

  const fallbackHeight = averageRowHeight(heightsRef.current);
  const itemsOffsetTop = scroll.overflow ? (pinnedList.current?.offsetHeight ?? 0) : 0;
  const relativeScrollTop = Math.max(0, scroll.top - itemsOffsetTop);
  const virtual = virtualItems
    ? computeVirtualWindow({
      ids,
      heights: heightsRef.current,
      fallbackHeight,
      scrollTop: relativeScrollTop,
      viewportHeight: scroll.height,
      threshold: virtualItems.threshold ?? VIRTUALIZE_THRESHOLD,
    })
    : { virtualized: false, start: 0, end: 0, paddingTop: 0, paddingBottom: 0, totalHeight: 0 };

  function measure() {
    const element = scroll.overflow ? frame.current : content.current;
    list.current = element;
    if (!element) return;
    const height = viewport.current?.clientHeight ?? 0;
    const pinnedHeight = pinnedList.current?.offsetHeight ?? 0;
    let overflow = scroll.overflow;
    if (height > 0) {
      if (pinnedHeight > 0) {
        const style = content.current ? getComputedStyle(content.current) : null;
        const padding = (parseFloat(style?.paddingTop ?? '') || 0) + (parseFloat(style?.paddingBottom ?? '') || 0);
        overflow = pinnedHeight + padding >= height;
      } else overflow = false;
    }
    const maxTop = Math.max(0, element.scrollHeight - element.clientHeight);
    const next = {
      top: Math.max(0, Math.min(maxTop, element.scrollTop)),
      height: element.clientHeight,
      content: element.scrollHeight,
      track: height || element.clientHeight,
      overflow,
    };
    setScroll((old) => Object.keys(next).every((key) => old[key as keyof typeof old] === next[key as keyof typeof next]) ? old : next);
  }
  const measureRef = useRef(measure);
  measureRef.current = measure;

  function measureRenderedRows() {
    if (!virtualItems || !itemsRef.current || ids.length === 0) return;
    const container = itemsRef.current;
    const scroller = list.current;
    const rows = container.querySelectorAll<HTMLElement>(':scope > .history-row:not([hidden])');
    if (rows.length === 0) return;
    const offsets = virtual.virtualized ? buildOffsets(ids, heightsRef.current, fallbackHeight) : null;
    let changed = false;
    let scrollCorrection = 0;
    for (const row of rows) {
      const id = row.dataset.recordId;
      if (!id) continue;
      const index = idToIndex.get(id);
      if (index === undefined) continue;
      const rawHeight = row.offsetHeight;
      if (rawHeight <= 0) continue;
      const measured = rawHeight + (index < ids.length - 1 ? ROW_GAP_PX : 0);
      const previous = heightsRef.current.get(id);
      const effectivePrevious = previous ?? fallbackHeight;
      if (previous === undefined || Math.abs(measured - previous) >= 0.5) {
        if (offsets && !dragging.current && scroller && scroller.scrollTop > 0 && (itemsOffsetTop + offsets[index]) < scroller.scrollTop) {
          scrollCorrection += measured - effectivePrevious;
        }
        heightsRef.current.set(id, measured);
        changed = true;
      }
    }
    if (heightsRef.current.size > Math.max(2000, ids.length * 2)) {
      for (const key of heightsRef.current.keys()) {
        if (!idToIndex.has(key)) heightsRef.current.delete(key);
      }
    }
    if (scrollCorrection !== 0 && scroller) {
      scroller.scrollTop = Math.max(0, scroller.scrollTop + scrollCorrection);
    }
    if (changed) {
      setHeightRevision((value) => value + 1);
      measureRef.current();
    }
  }
  const measureRowsRef = useRef(measureRenderedRows);
  measureRowsRef.current = measureRenderedRows;

  useImperativeHandle(handleRef, () => ({
    scrollToId(id: string, behavior: ScrollBehavior = 'instant') {
      if (!virtualItems || !virtual.virtualized) return false;
      const index = idToIndex.get(id);
      const scroller = list.current;
      if (index === undefined || !scroller) return false;
      const offsets = buildOffsets(ids, heightsRef.current, fallbackHeight);
      const target = computeScrollTargetForIndex(offsets, index, scroller.scrollTop, scroller.clientHeight, itemsOffsetTop);
      if (target !== null) {
        scroller.scrollTo({ top: target, behavior });
        measureRef.current();
      }
      return true;
    },
  }), [virtualItems, virtual.virtualized, idToIndex, ids, fallbackHeight, itemsOffsetTop, list]);

  useLayoutEffect(() => {
    const owner = scroll.overflow ? frame.current : content.current;
    if (owner && list.current && owner !== list.current) {
      owner.scrollTop = scroll.top;
      list.current = owner;
      onScrollOwnerChange?.();
    }
    if (!scroll.overflow && frame.current) frame.current.scrollTop = 0;
    if (scroll.overflow && content.current) content.current.scrollTop = 0;
    measureRef.current();
    measureRowsRef.current();
  }, [children, pinned, active, scroll.overflow, visibleList, virtual.start, virtual.end, virtual.virtualized]);

  useLayoutEffect(() => {
    const observer = new ResizeObserver(() => {
      measureRef.current();
      measureRowsRef.current();
    });
    for (const element of [viewport.current, frame.current, content.current, pinnedList.current, itemsRef.current]) {
      if (element) observer.observe(element);
    }
    return () => observer.disconnect();
  }, [active, scroll.overflow]);

  useLayoutEffect(() => {
    if (!virtualItems || !itemsRef.current) return;
    const rows = itemsRef.current.querySelectorAll<HTMLElement>(':scope > .history-row:not([hidden])');
    if (rows.length === 0) return;
    const observer = new ResizeObserver(() => measureRowsRef.current());
    for (const row of rows) observer.observe(row);
    return () => observer.disconnect();
  }, [virtualItems !== undefined, virtual.virtualized, virtual.start, virtual.end, visibleList]);

  const maximum = Math.max(0, scroll.content - scroll.height);
  const thumb = Math.min(scroll.track, Math.max(24, scroll.track * scroll.height / Math.max(1, scroll.content)));
  const travel = scroll.track - thumb;
  function move(top: number) { list.current?.scrollTo({ top: Math.max(0, Math.min(maximum, top)) }); }
  function handleScroll(event: UIEvent<HTMLDivElement>) {
    if (event.currentTarget !== list.current) return;
    measure(); if (active) { onScroll?.(); onNearEnd(event.currentTarget); }
  }

  const virtualThreshold = virtualItems?.threshold ?? VIRTUALIZE_THRESHOLD;
  // When a real viewport is active and the retained pool exceeds the virtualization threshold,
  // avoid materializing hundreds of hidden rows when a filter or search shrinks visible results.
  const retainAllItems = virtualItems?.allItems !== undefined
    && (scroll.height <= 0 || virtualItems.allItems.length <= virtualThreshold);
  const renderedItems = virtualItems
    ? (virtual.virtualized
      ? virtualItems.items.slice(virtual.start, virtual.end)
      : (retainAllItems ? virtualItems.allItems! : virtualItems.items)
    ).map(virtualItems.renderItem)
    : null;

  return <div ref={viewport} className="history-viewport" onWheel={(event) => {
    showScrollbar();
    if (!scroll.overflow && pinnedList.current?.contains(event.target as Node) && list.current) {
      list.current.scrollTop += event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? list.current.clientHeight : 1);
    }
  }} onTouchMove={showScrollbar} onKeyDownCapture={(event) => {
    if (!event.altKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing && ['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) showScrollbar();
  }}>
    <div ref={frame} id={scroll.overflow ? listId : undefined} className={`history-list-frame${scroll.overflow ? ' history-list' : ''}`} data-overflow={scroll.overflow} onScroll={handleScroll}>
      <div ref={pinnedList} className={`history-pinned${pinnedCount ? '' : ' empty'}`} data-overflow={scroll.overflow} role={pinnedCount ? 'region' : undefined} aria-label={pinnedCount ? 'Pinned records' : undefined}>{pinned}</div>
      <div ref={content} id={scroll.overflow ? undefined : listId} className={`history-unpinned${scroll.overflow ? '' : ' history-list'}`} onScroll={handleScroll}>
        <div
          ref={itemsRef}
          className="history-items"
          data-virtualized={virtual.virtualized || undefined}
          data-loaded-count={virtualItems ? ids.length : undefined}
          style={virtual.virtualized ? { paddingTop: virtual.paddingTop, paddingBottom: virtual.paddingBottom } : undefined}
        >
          {renderedItems}
          {children}
        </div>
      </div>
    </div>
    {maximum > 0 && <div className={`history-scrollbar${scrollbarVisible ? ' visible' : ''}`} onPointerMove={() => { if (!scrollOnly || scrollbarVisible) showScrollbar(); }} onFocus={() => { focused.current = true; showScrollbar(); }} onBlur={() => { focused.current = false; if (!scrollOnly || scrollbarVisible) showScrollbar(); }} role="scrollbar" aria-label="Scroll records" aria-controls={listId} aria-orientation="vertical" aria-valuemin={0} aria-valuemax={maximum} aria-valuenow={Math.round(scroll.top)} tabIndex={0}
      onKeyDown={(event) => {
        const delta = event.key === 'ArrowDown' ? 40 : event.key === 'ArrowUp' ? -40 : event.key === 'PageDown' ? scroll.height : event.key === 'PageUp' ? -scroll.height : 0;
        if (delta || event.key === 'Home' || event.key === 'End') { showScrollbar(); event.preventDefault(); event.stopPropagation(); move(event.key === 'Home' ? 0 : event.key === 'End' ? maximum : scroll.top + delta); }
      }} onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        dragging.current = true; showScrollbar();
        const track = event.currentTarget;
        track.setPointerCapture(event.pointerId);
        const startY = event.clientY;
        const onThumb = (event.target as HTMLElement).classList.contains('history-scrollbar-thumb');
        const startTop = onThumb ? scroll.top : Math.max(0, Math.min(maximum, (event.clientY - track.getBoundingClientRect().top - thumb / 2) / Math.max(1, travel) * maximum));
        move(startTop);
        const drag = (next: PointerEvent) => { if (next.pointerId === event.pointerId) { showScrollbar(); move(startTop + (next.clientY - startY) / Math.max(1, travel) * maximum); } };
        const cleanup = () => { track.removeEventListener('pointermove', drag); track.removeEventListener('lostpointercapture', end); dragCleanup.current = undefined; };
        const end = () => { dragging.current = false; showScrollbar(); cleanup(); };
        track.addEventListener('pointermove', drag);
        track.addEventListener('lostpointercapture', end);
        dragCleanup.current = cleanup;
      }}><span className="history-scrollbar-thumb" style={{ height: thumb, transform: `translateY(${maximum ? scroll.top / maximum * travel : 0}px)` }} /></div>}
  </div>;
}
