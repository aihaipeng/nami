import { useId, useLayoutEffect, useRef } from 'react';
import type { Translate } from '../shared/i18n';

export function markedDeleteDescription(t: Translate, record: { isFavorite: boolean; isPinned: boolean }) {
  if (record.isPinned && record.isFavorite) return t('clipboardDeletePinnedFavorite');
  if (record.isPinned) return t('clipboardDeletePinned');
  return t('clipboardDeleteFavorite');
}

export function ClipboardDeleteConfirm({ t, description, onCancel, onConfirm }: {
  t: Translate;
  description: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  useLayoutEffect(() => {
    const element = dialog.current!;
    const trigger = document.activeElement;
    element.showModal();
    element.querySelector<HTMLElement>('[data-initial-focus]')?.focus({ preventScroll: true });
    return () => {
      element.close();
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus({ preventScroll: true });
    };
  }, []);
  return <dialog ref={dialog} className="clipboard-delete-confirm" role="alertdialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId}
    onCancel={(event) => { event.preventDefault(); onCancel(); }}
    onKeyDown={(event) => {
      event.stopPropagation();
      if (event.nativeEvent.isComposing || event.keyCode === 229) return;
      if (event.key !== 'Escape' && event.key !== 'Enter') return;
      event.preventDefault();
      if (event.repeat) return;
      if (event.key === 'Escape') onCancel();
      else onConfirm();
    }}>
    <strong id={titleId}>{t('clipboardDeleteMarkedTitle')}</strong>
    <p id={descriptionId}>{description}</p>
    <div className="clipboard-editor-actions">
      <button type="button" data-initial-focus aria-keyshortcuts="Escape" onClick={onCancel}>{t('clipboardCancel')}<span className="keycaps" aria-hidden="true"><kbd>Esc</kbd></span></button>
      <button type="button" className="clipboard-delete-confirm-action" aria-keyshortcuts="Enter" onClick={onConfirm}>{t('clipboardDeleteConfirm')}<span className="keycaps" aria-hidden="true"><kbd>↵</kbd></span></button>
    </div>
  </dialog>;
}
