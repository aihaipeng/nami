import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { motion } from 'motion/react';

// The scrollbar spans the complete viewport while pinned rows stay above the scrolling list.
export function HistoryList({ list, pinnedList, pinned, pinnedCount = pinned.length, children, onNearEnd, onScroll, active = true, scrollOnly = false }: {
  list: RefObject<HTMLDivElement | null>; pinnedList: RefObject<HTMLDivElement | null>;
  pinned: ReactNode[]; pinnedCount?: number; children: ReactNode; onNearEnd: (element: HTMLDivElement) => void; onScroll?: () => void; active?: boolean; scrollOnly?: boolean;
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
  const viewport = useRef<HTMLDivElement>(null);
  const [scroll, setScroll] = useState({ top: 0, height: 0, content: 0, pinned: 0 });
  function measure() {
    const element = list.current;
    if (!element) return;
    const next = { top: Math.max(0, Math.min(element.scrollHeight - element.clientHeight, element.scrollTop)), height: element.clientHeight, content: element.scrollHeight, pinned: pinnedList.current?.offsetHeight ?? 0 };
    setScroll((old) => Object.keys(next).every((key) => old[key as keyof typeof old] === next[key as keyof typeof next]) ? old : next);
  }
  useLayoutEffect(() => {
    measure();
    const observer = new ResizeObserver(measure);
    for (const element of [list.current, pinnedList.current, list.current?.querySelector('.history-items')]) if (element) observer.observe(element);
    return () => observer.disconnect();
  }, [children, pinned, active]);
  const maximum = Math.max(0, scroll.content - scroll.height);
  const thumb = Math.min(scroll.height, Math.max(24, scroll.height * scroll.height / Math.max(1, scroll.content)));
  const travel = scroll.height - thumb;
  function move(top: number) { list.current?.scrollTo({ top: Math.max(0, Math.min(maximum, top)) }); }
  return <div ref={viewport} className="history-viewport" onWheel={showScrollbar} onTouchMove={showScrollbar} onKeyDownCapture={(event) => {
    if (!event.altKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing && ['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) showScrollbar();
  }}>
    <motion.div layoutScroll ref={list} id={listId} className="history-list" onScroll={(event) => { measure(); if (active) { onScroll?.(); onNearEnd(event.currentTarget); } }}>
      <div ref={pinnedList} className={`history-pinned${pinnedCount ? '' : ' empty'}`} data-overflow={scroll.height > 0 && scroll.pinned >= scroll.height}>{pinned}</div>
      <div className="history-items">{children}</div>
    </motion.div>
    {maximum > 0 && <div className={`history-scrollbar${scrollbarVisible ? ' visible' : ''}`} onPointerMove={() => { if (!scrollOnly || scrollbarVisible) showScrollbar(); }} onFocus={() => { focused.current = true; if (!scrollOnly || scrollbarVisible) showScrollbar(); }} onBlur={() => { focused.current = false; if (!scrollOnly || scrollbarVisible) showScrollbar(); }} role="scrollbar" aria-label="Scroll records" aria-controls={listId} aria-orientation="vertical" aria-valuemin={0} aria-valuemax={maximum} aria-valuenow={Math.round(scroll.top)} tabIndex={0}
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
