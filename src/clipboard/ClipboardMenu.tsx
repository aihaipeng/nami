import type { Translate } from '../shared/i18n';
import { ClipboardPaste, FolderOpen, Save } from 'lucide-react';
import { RecordMenu, recordMenuItems, type RecordMenuItem } from '../shared/RecordMenu';
import type { AppSnapshot } from '../shared/contracts';
import { clipboardPanelShortcuts } from '../shared/clipboardShortcuts';
import { HistoryIcon } from '../translation/HistoryIcon';
import type { ClipboardRecord } from './useClipboard';

type Action = 'paste' | 'pastePlain' | 'pastePaths' | 'copy' | 'save' | 'translate' | 'ocr' | 'openTarget' | 'favorite' | 'pin' | 'note' | 'delete';
type Item = RecordMenuItem & { action: Action };

export function ClipboardMenu({ record, snapshot, x, y, canPaste, canCopy, canSave, canTranslate, canOcr, canOpenTarget, t, busy, onClose, onReturnFocus, onAction }: {
  record: ClipboardRecord; snapshot: AppSnapshot; x: number; y: number; canPaste: boolean; canCopy: boolean; canSave: boolean; canTranslate: boolean; canOcr: boolean; canOpenTarget: boolean; t: Translate; busy: boolean;
  onReturnFocus: () => void; onClose: () => void; onAction: (action: Action) => void;
}) {
  const mac = snapshot.capabilities.platform === 'macos';
  const pasteIcon = <ClipboardPaste aria-hidden="true" />;
  const paste: Item = { action: 'paste', label: 'Paste', icon: pasteIcon, disabled: !canPaste };
  const copy: Item = { action: 'copy', label: t('copy'), icon: <HistoryIcon name="doc.on.doc" />, disabled: !canCopy };
  const marks: Item[] = [
    { action: 'favorite', label: record.isFavorite ? 'Unfavorite' : 'Favorite', icon: <HistoryIcon name="star" />, active: record.isFavorite },
    { action: 'pin', label: record.isPinned ? 'Unpin' : 'Pin', icon: <HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /> },
    { action: 'note', label: 'Note', icon: <HistoryIcon name="note" /> },
  ];
  const remove: Item[] = [{ action: 'delete', label: 'Delete', destructive: true, icon: <HistoryIcon name="trash" />, disabled: busy }];
  const groups: Item[][] = record.kind === 'image' ? [
    [copy, paste, { action: 'save', label: t('clipboardSaveImage'), icon: <Save aria-hidden="true" />, disabled: !canSave }],
    marks,
    [
      ...(canTranslate ? [{ action: 'translate' as const, label: 'Translate', description: t('clipboardTranslatePrivacy'), icon: <HistoryIcon name="languages" />, disabled: busy }] : []),
      ...(canOcr ? [{ action: 'ocr' as const, label: t('clipboardShortcutOcr'), description: t('clipboardOcrPrivacy'), icon: <HistoryIcon name="ocr" />, disabled: busy }] : []),
    ],
    remove,
  ] : record.kind === 'files' ? [
    [paste, { action: 'pastePaths', label: t('clipboardPastePaths'), icon: pasteIcon, disabled: !canPaste }, copy],
    [{ action: 'openTarget', label: t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager'), icon: <FolderOpen aria-hidden="true" />, disabled: !canOpenTarget }],
    marks, remove,
  ] : [
    [paste, { action: 'pastePlain', label: t('clipboardPastePlain'), icon: pasteIcon, disabled: !canPaste }, copy],
    marks, remove,
  ];
  const configured = clipboardPanelShortcuts(snapshot);
  const items = recordMenuItems(groups, mac, t, (action) => {
    if (action === 'paste') return snapshot.storageError ? null : { code: 'Enter', modifiers: [] };
    const shortcutAction = action === 'copy' ? 'copyRecord' : action === 'save' ? 'saveImage' : action;
    return snapshot.storageError ? null : configured.find(({ action, enabled, globalConflict }) => action === shortcutAction && enabled && !globalConflict)?.binding;
  });
  return <RecordMenu x={x} y={y} items={items} onClose={onClose} onReturnFocus={onReturnFocus} onAction={(action) => onAction(action as Action)} />;
}
