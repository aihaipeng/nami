import { useLayoutEffect, useRef } from 'react';

export function ActivityBeam({ label }: { label: string }) {
  const track = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = track.current;
    if (!element) return;
    const measure = () => {
      const width = element.getBoundingClientRect().width;
      element.style.setProperty('--beam-duration', `${(width + Math.min(width * .3, 220)) / 240}s`);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return <div ref={track} className="activity-beam" role="progressbar" aria-label={label}><span /></div>;
}
