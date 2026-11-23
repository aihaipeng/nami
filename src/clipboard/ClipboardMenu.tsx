import { useEffect, useState } from 'react';
import type { Translate } from '../shared/i18n';
import { clipboardOpenTarget, openTargetLabels } from './openTarget';
import { ClipboardPaste, ExternalLink, FolderOpen, Mail, Save } from 'lucide-react';
import { RecordMenu, recordMenuAllowsBinding, type RecordMenuItem } from '../shared/RecordMenu';
import type { AppSnapshot } from '../shared/contracts';
import { clipboardPanelShortcuts, type ClipboardRecordAction } from '../shared/clipboardShortcuts';
import { keycapLabels, modifierKey } from '../shared/shortcuts';
import { HistoryIcon } from '../translation/HistoryIcon';
import type { ClipboardRecord } from './useClipboard';

type Action = 'paste' | 'copy' | 'save' | 'translate' | 'ocr' | 'openTarget' | 'favorite' | 'pin' | 'note' | 'delete';
type Item = RecordMenuItem & { action: Action };

export function ClipboardMenu({ record, snapshot, x, y, canPaste, canCopy, canSave, canTranslate, canOcr, canOpenTarget, t, busy, onClose, onReturnFocus, onAction }: {
  record: ClipboardRecord; snapshot: AppSnapshot; x: number; y: number; canPaste: boolean; canCopy: boolean; canSave: boolean; canTranslate: boolean; canOcr: boolean; canOpenTarget: boolean; t: Translate; busy: boolean;
  onReturnFocus: () => void; onClose: () => void; onAction: (action: Action) => void;
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
  const image = record.kind === 'image';
  const paste: Item = { action: 'paste', label: 'Paste', icon: <ClipboardPaste aria-hidden="true" />, disabled: !canPaste };
  const textActions: Item[] = [
    ...(canTranslate ? [{ action: 'translate' as const, label: 'Translate', description: t('clipboardTranslatePrivacy'), icon: <HistoryIcon name="languages" />, disabled: busy }] : []),
    ...(canOcr ? [{ action: 'ocr' as const, label: image ? 'OCR' : 'Extract text', description: t('clipboardOcrPrivacy'), icon: <HistoryIcon name="ocr" />, disabled: busy }] : []),
  ];
  const marks: Item[] = [
    { action: 'favorite', label: record.isFavorite ? (image ? 'Unfavorite' : 'Remove favorite') : 'Favorite', icon: <HistoryIcon name="star" />, active: record.isFavorite },
    { action: 'pin', label: record.isPinned ? (image ? 'Unpin' : 'Unpin record') : (image ? 'Pin' : 'Pin record'), icon: <HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /> },
    { action: 'note', label: 'Note', icon: <HistoryIcon name="note" /> },
  ];
  const remove: Item = { action: 'delete', label: 'Delete', destructive: true, icon: <HistoryIcon name="trash" />, disabled: busy };
  let items: Item[];
  if (image) {
    const groups: Item[][] = [
      [
        { action: 'copy', label: t('copy'), icon: <HistoryIcon name="doc.on.doc" />, disabled: !canCopy }, paste,
        { action: 'save', label: t('clipboardSaveImage'), icon: <Save aria-hidden="true" />, disabled: !canSave },
      ],
      marks, textActions, [remove],
    ];
    const configured = clipboardPanelShortcuts(snapshot).filter(({ enabled, binding, globalConflict }) => enabled && binding && !globalConflict && recordMenuAllowsBinding(binding));
    const mac = snapshot.capabilities.platform === 'macos';
    items = groups.filter((group) => group.length).flatMap((group, index) => group.map((item, row) => {
      const action: ClipboardRecordAction = item.action === 'copy' ? 'copyRecord' : item.action === 'paste' ? 'pasteRecord' : item.action === 'save' ? 'saveImage' : item.action;
      const binding = !item.disabled && !snapshot.storageError && configured.find((shortcut) => shortcut.action === action)?.binding;
      const separatorBefore = index > 0 && row === 0;
      if (!binding) return { ...item, separatorBefore, shortcut: null };
      const keys = keycapLabels(binding, mac, t);
      const symbol = keys.at(-1) ?? '';
      const ariaKey = /^[!-~]$/.test(symbol) ? symbol : binding.code;
      const aria = modifierKey(binding.code) ? undefined : [...binding.modifiers.map((modifier) => modifier[0].toUpperCase() + modifier.slice(1)), ariaKey].join('+');
      return { ...item, separatorBefore, shortcut: { binding, label: keys.join(mac ? '' : '+'), aria } };
    }));
  } else {
    items = [paste, ...(kind ? [{ action: 'openTarget' as const, label: t(openTargetLabels[kind]), icon: <TargetIcon aria-hidden="true" /> }] : []), ...textActions, ...marks, remove];
  }
  return <RecordMenu x={x} y={y} items={items} onClose={onClose} onReturnFocus={onReturnFocus} onAction={(action) => onAction(action as Action)} />;
}
