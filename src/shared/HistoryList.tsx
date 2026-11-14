import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';

// The scrollbar spans the complete viewport while pinned rows stay above the scrolling list.
export function HistoryList({ list, pinnedList, pinned, pinnedCount = pinned.length, children, onNearEnd }: {
  list: RefObject<HTMLDivElement | null>; pinnedList: RefObject<HTMLDivElement | null>;
  pinned: ReactNode[]; pinnedCount?: number; children: ReactNode; onNearEnd: (element: HTMLDivElement) => void;
}) {
  const listId = useId();
  const [scrollbarVisible, setScrollbarVisible] = useState(false);
  const idle = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const dragging = useRef(false);
  const focused = useRef(false);
  function showScrollbar() {
    window.clearTimeout(idle.current);
    setScrollbarVisible(true);
    idle.current = window.setTimeout(() => {
      if (!dragging.current && !focused.current) setScrollbarVisible(false);
    }, 1000);
  }
  useEffect(() => () => window.clearTimeout(idle.current), []);
  const viewport = useRef<HTMLDivElement>(null);
  const [scroll, setScroll] = useState({ top: 0, height: 0, content: 0, track: 0 });
  function measure() {
    const element = list.current;
    if (!element) return;
    const next = { top: Math.max(0, Math.min(element.scrollHeight - element.clientHeight, element.scrollTop)), height: element.clientHeight, content: element.scrollHeight, track: viewport.current?.clientHeight ?? 0 };
    setScroll((old) => Object.keys(next).every((key) => old[key as keyof typeof old] === next[key as keyof typeof next]) ? old : next);
  }
  useLayoutEffect(() => {
    measure();
    const observer = new ResizeObserver(measure);
    for (const element of [viewport.current, list.current, pinnedList.current, ...Array.from(list.current?.children ?? [])]) if (element) observer.observe(element);
    return () => observer.disconnect();
  }, [children, pinned]);
  const maximum = Math.max(0, scroll.content - scroll.height);
  const thumb = Math.min(scroll.track, Math.max(24, scroll.track * scroll.height / Math.max(1, scroll.content)));
  const travel = scroll.track - thumb;
  function move(top: number) { list.current?.scrollTo({ top: Math.max(0, Math.min(maximum, top)) }); }
  return <div ref={viewport} className="history-viewport">
    <div ref={pinnedList} className={`history-pinned${pinnedCount ? '' : ' empty'}`} onScroll={(event) => onNearEnd(event.currentTarget)}>{pinned}</div>
    <div ref={list} id={listId} className="history-list" onScroll={(event) => { showScrollbar(); measure(); onNearEnd(event.currentTarget); }}>{children}</div>
    {maximum > 0 && <div className={`history-scrollbar${scrollbarVisible ? ' visible' : ''}`} onPointerEnter={showScrollbar} onFocus={() => { focused.current = true; showScrollbar(); }} onBlur={() => { focused.current = false; showScrollbar(); }} role="scrollbar" aria-label="Scroll records" aria-controls={listId} aria-orientation="vertical" aria-valuemin={0} aria-valuemax={maximum} aria-valuenow={Math.round(scroll.top)} tabIndex={0}
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
        const end = () => { dragging.current = false; showScrollbar(); track.removeEventListener('pointermove', drag); track.removeEventListener('lostpointercapture', end); };
        track.addEventListener('pointermove', drag);
        track.addEventListener('lostpointercapture', end);
      }}><span className="history-scrollbar-thumb" style={{ height: thumb, transform: `translateY(${maximum ? scroll.top / maximum * travel : 0}px)` }} /></div>}
  </div>;
}
