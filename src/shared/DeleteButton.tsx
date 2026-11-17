import { useEffect, useRef, useState, type ComponentProps } from 'react';

export function DeleteButton({ context, className = '', onClick, onBlur, ...props }: ComponentProps<'button'> & { context: string }) {
  const button = useRef<HTMLButtonElement>(null);
  const [armed, setArmed] = useState(false);
  useEffect(() => setArmed(false), [context, props.disabled]);
  useEffect(() => {
    if (!armed) return;
    const cancel = () => setArmed(false);
    const outside = (event: PointerEvent) => { if (!button.current?.contains(event.target as Node)) cancel(); };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape' && !event.isComposing) cancel(); };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape, true);
    window.addEventListener('blur', cancel);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('keydown', escape, true);
      window.removeEventListener('blur', cancel);
    };
  }, [armed]);
  return <button {...props} ref={button} className={`${className}${armed ? ' delete-armed' : ''}`} aria-description={armed ? 'Click again to delete' : undefined}
    onBlur={(event) => { setArmed(false); onBlur?.(event); }} onClick={(event) => {
      if (!armed) { setArmed(true); return; }
      setArmed(false);
      onClick?.(event);
    }} />;
}
