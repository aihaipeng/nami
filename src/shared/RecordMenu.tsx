import { useRef, type ReactNode } from 'react';
import { FloatingPanel } from './FloatingPanel';

export interface RecordMenuItem { action: string; label: string; icon: ReactNode; disabled?: boolean; destructive?: boolean }

export function RecordMenu({ x, y, items, onClose, onReturnFocus, onAction }: {
  x: number; y: number; items: readonly RecordMenuItem[];
  onClose: () => void; onReturnFocus: () => void; onAction: (action: string) => void;
}) {
  const anchor = useRef<HTMLSpanElement>(null);
  return <>
    <span ref={anchor} tabIndex={-1} onFocus={onReturnFocus} className="record-menu-anchor" style={{ left: x, top: y }} />
    <FloatingPanel anchor={anchor} onClose={onClose} label="Actions" role="menu" width={176} side="bottom" className="record-context-menu clipboard-context-menu">
      <div onKeyDown={(event) => {
        if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp' && event.key !== 'Home' && event.key !== 'End') return;
        event.preventDefault();
        const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
        const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
        buttons[next]?.focus();
      }}>
        {items.map((item) => <button key={item.action} type="button" role="menuitem" disabled={'disabled' in item && item.disabled} className={item.destructive ? 'clipboard-menu-delete' : undefined} onClick={() => onAction(item.action)}>{item.icon}<span>{item.label}</span></button>)}
      </div>
    </FloatingPanel>
  </>;
}
