// Adapted from EcoPaste ClipboardQuickActions; modified for Nami. See licenses/EcoPaste.txt.
import type { ViewProps } from '../App';
import type { ComponentProps } from 'react';
import { AnimatePresence, motion, useIsPresent, useReducedMotion } from 'motion/react';
import { HistoryIcon } from '../translation/HistoryIcon';

export function RecordActions({ t, favorite, pinned, native, canCopy, copyLabel, deleteLabel, onCopy, onUpdate, onTranslate, onOcr, onNote, busy = false }: Pick<ViewProps, 't' | 'native'> & {
  favorite: boolean; pinned: boolean; canCopy: boolean;
  copyLabel: string; deleteLabel: string;
  onCopy: () => void; onUpdate: (action: 'favorite' | 'pin' | 'delete') => void;
  onTranslate?: () => void; onOcr?: () => void; onNote?: () => void; busy?: boolean;
}) {
  const reduceMotion = useReducedMotion();
  const actions = [
    { id: 'copy', label: copyLabel, icon: 'doc.on.doc' as const, disabled: !canCopy, onClick: onCopy },
    ...(onTranslate ? [{ id: 'translate', label: 'Translate', icon: 'languages' as const, disabled: busy, onClick: onTranslate }] : []),
    ...(onOcr ? [{ id: 'ocr', label: 'Extract text', icon: 'ocr' as const, disabled: busy, onClick: onOcr }] : []),
    ...(onNote ? [{ id: 'note', label: 'Note', icon: 'note' as const, onClick: onNote }] : []),
    { id: 'favorite', label: favorite ? t('unfavorite') : t('favorite'), icon: favorite ? 'star.fill' as const : 'star' as const, pressed: favorite, onClick: () => onUpdate('favorite') },
    { id: 'pin', label: pinned ? t('unpinRecord') : t('pinRecord'), icon: pinned ? 'pin.fill' as const : 'pin' as const, pressed: pinned, onClick: () => onUpdate('pin') },
    ...(!favorite && !pinned ? [{ id: 'delete', label: deleteLabel, icon: 'trash' as const, onClick: () => onUpdate('delete') }] : []),
  ];
  return <div className="history-row-actions" role="group" aria-label={t('actions')}>
    <AnimatePresence initial={false} mode="popLayout">
      {actions.map((action) => <motion.span key={action.id} className="history-action" data-action={action.id} layout layoutDependency={`${favorite}:${pinned}:${!!onTranslate}:${!!onOcr}:${!!onNote}`}
        initial={{ opacity: 0, scale: reduceMotion ? 1 : .9, width: 0, x: reduceMotion ? 0 : 4 }}
        animate={{ opacity: 1, scale: 1, width: 24, x: 0 }}
        exit={{ opacity: 0, scale: reduceMotion ? 1 : .9, width: 0, x: reduceMotion ? 0 : 4 }}
        transition={{ duration: reduceMotion ? 0 : .16, ease: 'easeOut' }}>
        <ActionButton className={`icon-button${action.id === 'delete' ? ' history-delete' : ''}`} title={action.label} aria-label={action.label}
          aria-pressed={'pressed' in action ? action.pressed : undefined} disabled={!native || ('disabled' in action && action.disabled)} onClick={action.onClick}><HistoryIcon name={action.icon} /></ActionButton>
      </motion.span>)}
    </AnimatePresence>
  </div>;
}

function ActionButton(props: ComponentProps<'button'>) {
  const present = useIsPresent();
  return <button {...props} disabled={props.disabled || !present} aria-hidden={!present} inert={!present} tabIndex={present ? undefined : -1} onClick={present ? props.onClick : undefined} />;
}
