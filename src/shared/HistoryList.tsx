import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject, type UIEvent } from 'react';

// Clip ordinary rows below the transparent pinned region; keep one full-height scrollbar.
export function HistoryList({ list, pinnedList, pinned, pinnedCount = pinned.length, children, onNearEnd, onScroll, onScrollOwnerChange, active = true, scrollOnly = false }: {
  list: RefObject<HTMLDivElement | null>; pinnedList: RefObject<HTMLDivElement | null>;
  pinned: ReactNode[]; pinnedCount?: number; children: ReactNode; onNearEnd: (element: HTMLDivElement) => void; onScroll?: () => void; onScrollOwnerChange?: () => void; active?: boolean; scrollOnly?: boolean;
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
  const [scroll, setScroll] = useState({ top: 0, height: 0, content: 0, track: 0, overflow: false });
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
    const next = { top: Math.max(0, Math.min(element.scrollHeight - element.clientHeight, element.scrollTop)), height: element.clientHeight, content: element.scrollHeight, track: height || element.clientHeight, overflow };
    setScroll((old) => Object.keys(next).every((key) => old[key as keyof typeof old] === next[key as keyof typeof next]) ? old : next);
  }
  const measureRef = useRef(measure);
  measureRef.current = measure;
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
  }, [children, pinned, active, scroll.overflow]);
  useLayoutEffect(() => {
    const observer = new ResizeObserver(() => measureRef.current());
    for (const element of [viewport.current, frame.current, content.current, pinnedList.current, content.current?.querySelector('.history-items')]) if (element) observer.observe(element);
    return () => observer.disconnect();
  }, [active, scroll.overflow]);
  const maximum = Math.max(0, scroll.content - scroll.height);
  const thumb = Math.min(scroll.track, Math.max(24, scroll.track * scroll.height / Math.max(1, scroll.content)));
  const travel = scroll.track - thumb;
  function move(top: number) { list.current?.scrollTo({ top: Math.max(0, Math.min(maximum, top)) }); }
  function handleScroll(event: UIEvent<HTMLDivElement>) {
    if (event.currentTarget !== list.current) return;
    measure(); if (active) { onScroll?.(); onNearEnd(event.currentTarget); }
  }
  return <div ref={viewport} className="history-viewport" onWheel={(event) => {
    showScrollbar();
    if (!scroll.overflow && pinnedList.current?.contains(event.target as Node) && list.current) {
      list.current.scrollTop += event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? list.current.clientHeight : 1);
    }
  }} onTouchMove={showScrollbar} onKeyDownCapture={(event) => {
    if (!event.altKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing && ['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) showScrollbar();
  }}>
    <div ref={frame} id={scroll.overflow ? listId : undefined} className={`history-list-frame${scroll.overflow ? ' history-list' : ''}`} data-overflow={scroll.overflow} onScroll={handleScroll}>
      <div ref={pinnedList} className={`history-pinned${pinnedCount ? '' : ' empty'}`} data-overflow={scroll.overflow}>{pinned}</div>
      <div ref={content} id={scroll.overflow ? undefined : listId} className={`history-unpinned${scroll.overflow ? '' : ' history-list'}`} onScroll={handleScroll}>
        <div className="history-items">{children}</div>
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
