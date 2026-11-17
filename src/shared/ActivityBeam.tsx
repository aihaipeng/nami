import { useLayoutEffect, useState } from 'react';

export function ActivityBeam({ label, active = true }: { label: string; active?: boolean }) {
  const [revealed, setRevealed] = useState(false);
  useLayoutEffect(() => {
    setRevealed(false);
    if (!active) return;
    const timer = window.setTimeout(() => setRevealed(true), 180);
    return () => window.clearTimeout(timer);
  }, [active]);
  return <div className="activity-beam" role={active ? 'progressbar' : undefined} aria-label={label} aria-hidden={!active} data-visible={active && revealed}><span /></div>;
}
