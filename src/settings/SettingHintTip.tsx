import { useId, useRef, useState } from 'react';
import { CircleAlert } from 'lucide-react';
import { FloatingPanel } from '../shared/FloatingPanel';

export function SettingHintTip({ label, text, conflict, className = '', width = 420 }: {
  label: string; text: string; conflict?: boolean; className?: string; width?: number;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  const [hoverOpen, setHoverOpen] = useState(false);
  const [clickOpen, setClickOpen] = useState(false);
  const open = hoverOpen || clickOpen;

  function close() {
    setHoverOpen(false);
    setClickOpen(false);
  }

  return <>
    <button ref={trigger} type="button" className={className ? `settings-shortcut-tip ${className}` : 'settings-shortcut-tip'}
      data-conflict={conflict || undefined} aria-label={label} aria-describedby={open ? id : undefined} aria-expanded={open}
      onMouseEnter={() => setHoverOpen(true)}
      onMouseLeave={close}
      onClick={(event) => {
        event.currentTarget.focus({ preventScroll: true });
        if (clickOpen) close();
        else { setClickOpen(true); setHoverOpen(false); }
      }}
      onBlur={close}
      onKeyDown={(event) => {
        if (open && event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); }
      }}><CircleAlert aria-hidden="true" /></button>
    {open && <FloatingPanel anchor={trigger} onClose={close} id={id} role="tooltip" width={width} side="bottom" className="settings-popover clipboard-tip-panel" dismissOnSettingsScroll>
      {text.includes('\n')
        ? <div className="clipboard-tip-lines">{text.split('\n').map((line, index) => <div key={index}>{line}</div>)}</div>
        : text}
    </FloatingPanel>}
  </>;
}
