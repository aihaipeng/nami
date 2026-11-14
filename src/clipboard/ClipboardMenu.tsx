import { useRef } from 'react';
import { ClipboardPaste } from 'lucide-react';
import { FloatingPanel } from '../shared/FloatingPanel';
import { HistoryIcon } from '../translation/HistoryIcon';
import type { ClipboardRecord } from './useClipboard';
import noteIcon from '../assets/history/reference/note.svg?url';

export function ClipboardMenu({ record, x, y, canPaste, onClose, onReturnFocus, onAction }: {
  record: ClipboardRecord; x: number; y: number; canPaste: boolean;
  onReturnFocus: () => void; onClose: () => void; onAction: (action: 'paste' | 'favorite' | 'pin' | 'note' | 'delete') => void;
}) {
  const anchor = useRef<HTMLSpanElement>(null);
  const items = [
    { action: 'paste', label: 'Paste', icon: <ClipboardPaste aria-hidden="true" />, disabled: !canPaste },
    { action: 'favorite', label: record.isFavorite ? 'Remove favorite' : 'Favorite', icon: <HistoryIcon name={record.isFavorite ? 'star.fill' : 'star'} /> },
    { action: 'pin', label: record.isPinned ? 'Unpin record' : 'Pin record', icon: <HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /> },
    { action: 'note', label: 'Note', icon: <span className="clipboard-note-icon" style={{ maskImage: `url("${noteIcon}")`, WebkitMaskImage: `url("${noteIcon}")` }} aria-hidden="true" /> },
    { action: 'delete', label: 'Delete', icon: <HistoryIcon name="trash" />, disabled: record.isFavorite || record.isPinned },
  ] as const;
  return <>
    <span ref={anchor} tabIndex={-1} onFocus={onReturnFocus} className="clipboard-menu-anchor" style={{ left: x, top: y }} />
    <FloatingPanel anchor={anchor} onClose={onClose} label="Actions" role="menu" width={176} side="bottom" className="clipboard-context-menu">
      <div onKeyDown={(event) => {
        if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp' && event.key !== 'Home' && event.key !== 'End') return;
        event.preventDefault();
        const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
        const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
        buttons[next]?.focus();
      }}>
        {items.map((item) => <button key={item.action} type="button" role="menuitem" disabled={'disabled' in item && item.disabled} className={item.action === 'delete' ? 'clipboard-menu-delete' : undefined} onClick={() => onAction(item.action)}>{item.icon}<span>{item.label}</span></button>)}
      </div>
    </FloatingPanel>
  </>;
}
