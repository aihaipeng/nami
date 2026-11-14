import { useEffect, useRef } from 'react';
import type { ViewProps } from '../App';

export function HistoryDivider({ t, width, command, onStart, onEscape }: Pick<ViewProps, 't' | 'command'> & {
  width: number;
  onStart?: () => void;
  onEscape?: () => void;
}) {
  const drag = useRef<{ x: number; width: number } | null>(null);
  const frame = useRef(0);
  useEffect(() => () => cancelAnimationFrame(frame.current), []);
  useEffect(() => { if (drag.current) drag.current.width = width; }, [width]);

  function dragWidth(x: number) {
    const current = drag.current!;
    current.width = Math.min(720, Math.max(280, current.width + x - current.x));
    current.x = x;
    return current.width;
  }

  return <div className="history-divider" role="separator" aria-label={t('resizeHistory')} aria-orientation="vertical" aria-valuemin={280} aria-valuemax={720} aria-valuenow={width} tabIndex={0}
    onKeyDown={(event) => {
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') {
        event.preventDefault();
        void command('history_width_set', { width: event.key === 'Home' ? 280 : event.key === 'End' ? 720 : width + (event.key === 'ArrowRight' ? 24 : -24), initialize: false });
      } else if (event.key === 'Escape' && onEscape) { event.preventDefault(); onEscape(); }
    }}
    onPointerDown={(event) => {
      if (event.button !== 0) return;
      event.preventDefault(); onStart?.();
      drag.current = { x: event.screenX, width };
      event.currentTarget.setPointerCapture(event.pointerId);
    }}
    onPointerMove={(event) => {
      if (!drag.current) return;
      const next = dragWidth(event.screenX);
      cancelAnimationFrame(frame.current);
      frame.current = requestAnimationFrame(() => { void command('history_width_set', { width: next, initialize: false }); });
    }}
    onPointerUp={(event) => {
      if (!drag.current) return;
      const next = dragWidth(event.screenX);
      drag.current = null; cancelAnimationFrame(frame.current);
      event.currentTarget.releasePointerCapture(event.pointerId);
      void command('history_width_set', { width: next, initialize: false });
    }}
    onLostPointerCapture={() => { drag.current = null; cancelAnimationFrame(frame.current); }} />;
}
