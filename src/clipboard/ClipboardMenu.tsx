import { useEffect, useState } from 'react';
import type { Translate } from '../shared/i18n';
import { clipboardOpenTarget, openTargetLabels } from './openTarget';
import { ClipboardPaste, ExternalLink, FolderOpen, Mail } from 'lucide-react';
import { RecordMenu } from '../shared/RecordMenu';
import { HistoryIcon } from '../translation/HistoryIcon';
import type { ClipboardRecord } from './useClipboard';

export function ClipboardMenu({ record, x, y, canPaste, canTranslate, canOcr, canOpenTarget, t, busy, onClose, onReturnFocus, onAction }: {
  record: ClipboardRecord; x: number; y: number; canPaste: boolean; canTranslate: boolean; canOcr: boolean; canOpenTarget: boolean; t: Translate; busy: boolean;
  onReturnFocus: () => void; onClose: () => void; onAction: (action: 'paste' | 'translate' | 'ocr' | 'openTarget' | 'favorite' | 'pin' | 'note' | 'delete') => void;
}) {
  const [target, setTarget] = useState<{ id: string; kind: keyof typeof openTargetLabels } | null>(null);
  useEffect(() => {
    if (!canOpenTarget || record.kind === 'image') return;
    let active = true;
    void clipboardOpenTarget(record.id, false).then((kind) => {
      if (active && kind) setTarget({ id: record.id, kind });
    }).catch(() => {});
    return () => { active = false; };
  }, [canOpenTarget, record.id, record.kind]);
  const kind = canOpenTarget && target?.id === record.id ? target.kind : null;
  const TargetIcon = kind === 'email' ? Mail : kind === 'link' ? ExternalLink : FolderOpen;
  const items = [
    { action: 'paste', label: 'Paste', icon: <ClipboardPaste aria-hidden="true" />, disabled: !canPaste },
    ...(kind ? [{ action: 'openTarget' as const, label: t(openTargetLabels[kind]), icon: <TargetIcon aria-hidden="true" /> }] : []),
    ...(canTranslate ? [{ action: 'translate' as const, label: 'Translate', icon: <HistoryIcon name="languages" />, disabled: busy }] : []),
    ...(canOcr ? [{ action: 'ocr' as const, label: 'Extract text', icon: <HistoryIcon name="ocr" />, disabled: busy }] : []),
    { action: 'favorite', label: record.isFavorite ? 'Remove favorite' : 'Favorite', icon: <HistoryIcon name="star" />, active: record.isFavorite },
    { action: 'pin', label: record.isPinned ? 'Unpin record' : 'Pin record', icon: <HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /> },
    { action: 'note', label: 'Note', icon: <HistoryIcon name="note" /> },
    { action: 'delete', label: 'Delete', destructive: true, icon: <HistoryIcon name="trash" />, disabled: busy },
  ] as const;
  return <RecordMenu x={x} y={y} items={items} onClose={onClose} onReturnFocus={onReturnFocus} onAction={(action) => onAction(action as typeof items[number]['action'])} />;
}
