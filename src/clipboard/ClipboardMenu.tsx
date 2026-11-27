import type { Translate } from '../shared/i18n';
import { ClipboardPaste, FolderOpen } from 'lucide-react';
import { RecordMenu, recordMenuItems, type RecordMenuItem } from '../shared/RecordMenu';
import type { AppSnapshot } from '../shared/contracts';
import { clipboardPanelShortcuts } from '../shared/clipboardShortcuts';
import { keycapLabels } from '../shared/shortcuts';
import { HistoryIcon } from '../translation/HistoryIcon';
import type { ClipboardRecord } from './useClipboard';

type Action = 'paste' | 'pastePlain' | 'pastePaths' | 'translate' | 'ocr' | 'openTarget' | 'favorite' | 'pin' | 'note' | 'delete';
type Item = RecordMenuItem & { action: Action };

export function ClipboardMenu({ record, snapshot, x, y, canPaste, canTranslate, canOcr, canOpenTarget, t, busy, onClose, onReturnFocus, onAction }: {
  record: ClipboardRecord; snapshot: AppSnapshot; x: number; y: number; canPaste: boolean; canTranslate: boolean; canOcr: boolean; canOpenTarget: boolean; t: Translate; busy: boolean;
  onReturnFocus: () => void; onClose: () => void; onAction: (action: Action) => void;
}) {
  const mac = snapshot.capabilities.platform === 'macos';
  const pasteIcon = <ClipboardPaste aria-hidden="true" />;
  const paste: Item = {
    action: 'paste', label: 'Paste', icon: pasteIcon, disabled: !canPaste,
    keyHint: canPaste && !snapshot.storageError ? keycapLabels({ code: 'Enter', modifiers: [] }, mac, t)[0] : undefined,
  };
  const marks: Item[] = [
    { action: 'favorite', label: record.isFavorite ? 'Unfavorite' : 'Favorite', icon: <HistoryIcon name="star" />, active: record.isFavorite },
    { action: 'pin', label: record.isPinned ? 'Unpin' : 'Pin', icon: <HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /> },
    { action: 'note', label: 'Note', icon: <HistoryIcon name="note" /> },
  ];
  const remove: Item[] = [{ action: 'delete', label: 'Delete', destructive: true, icon: <HistoryIcon name="trash" />, disabled: busy }];
  const groups: Item[][] = record.kind === 'image' ? [
    [paste, ...(record.hasFilePaths ? [{ action: 'pastePaths' as const, label: t('clipboardPastePaths'), icon: pasteIcon, disabled: !canPaste }] : [])],
    ...(record.hasFilePaths ? [[{ action: 'openTarget' as const, label: t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager'), icon: <FolderOpen aria-hidden="true" />, disabled: !canOpenTarget }]] : []),
    marks,
    [
      ...(canTranslate ? [{ action: 'translate' as const, label: 'Translate', description: t('clipboardTranslatePrivacy'), icon: <HistoryIcon name="languages" />, disabled: busy }] : []),
      ...(canOcr ? [{ action: 'ocr' as const, label: t('clipboardShortcutOcr'), description: t('clipboardOcrPrivacy'), icon: <HistoryIcon name="ocr" />, disabled: busy }] : []),
    ],
    remove,
  ] : record.kind === 'files' ? [
    [paste, { action: 'pastePaths', label: t('clipboardPastePaths'), icon: pasteIcon, disabled: !canPaste }],
    [{ action: 'openTarget', label: t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager'), icon: <FolderOpen aria-hidden="true" />, disabled: !canOpenTarget }],
    marks, remove,
  ] : [
    [paste, { action: 'pastePlain', label: t('clipboardPastePlain'), icon: pasteIcon, disabled: !canPaste }],
    marks, remove,
  ];
  const configured = clipboardPanelShortcuts(snapshot);
  const items = recordMenuItems(groups, mac, t, (itemAction) =>
    snapshot.storageError ? null : configured.find(({ action, enabled, globalConflict }) => action === itemAction && enabled && !globalConflict)?.binding);
  return <RecordMenu x={x} y={y} items={items} onClose={onClose} onReturnFocus={onReturnFocus} onAction={(action) => onAction(action as Action)} />;
}
