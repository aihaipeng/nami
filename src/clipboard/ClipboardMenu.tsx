import { ClipboardPaste } from 'lucide-react';
import { RecordMenu } from '../shared/RecordMenu';
import { HistoryIcon } from '../translation/HistoryIcon';
import type { ClipboardRecord } from './useClipboard';

export function ClipboardMenu({ record, x, y, canPaste, onClose, onReturnFocus, onAction }: {
  record: ClipboardRecord; x: number; y: number; canPaste: boolean;
  onReturnFocus: () => void; onClose: () => void; onAction: (action: 'paste' | 'favorite' | 'pin' | 'note' | 'delete') => void;
}) {
  const items = [
    { action: 'paste', label: 'Paste', icon: <ClipboardPaste aria-hidden="true" />, disabled: !canPaste },
    { action: 'favorite', label: record.isFavorite ? 'Remove favorite' : 'Favorite', icon: <HistoryIcon name={record.isFavorite ? 'star.fill' : 'star'} /> },
    { action: 'pin', label: record.isPinned ? 'Unpin record' : 'Pin record', icon: <HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /> },
    { action: 'note', label: 'Note', icon: <HistoryIcon name="note" /> },
    { action: 'delete', label: 'Delete', destructive: true, icon: <HistoryIcon name="trash" />, disabled: record.isFavorite || record.isPinned },
  ] as const;
  return <RecordMenu x={x} y={y} items={items} onClose={onClose} onReturnFocus={onReturnFocus} onAction={(action) => onAction(action as typeof items[number]['action'])} />;
}
