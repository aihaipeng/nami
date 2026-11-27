// Adapted from EcoPaste ClipboardQuickActions; modified for Nami. See licenses/EcoPaste.txt.
import type { ViewProps } from '../App';
import type { ComponentProps } from 'react';
import { CircleCheck, ExternalLink, FolderOpen, ScanQrCode } from 'lucide-react';
import { AnimatePresence, motion, useIsPresent, useReducedMotion } from 'motion/react';
import { HistoryIcon } from '../translation/HistoryIcon';
import { useActionDone } from './useActionDone';

type ActionOutcome = boolean | void | Promise<boolean | void>;

export function RecordActions({ t, favorite, pinned, native, deleteLabel, onOpenTarget, onQrCode, onUpdate, onTranslate, translateActive = false, onOcr, onNote, busy = false, retainDelete = false, allowMarkedDelete = false, tabIndex, tooltipSide, recordId }: Pick<ViewProps, 't' | 'native'> & {
  favorite: boolean; pinned: boolean; deleteLabel: string;
  onOpenTarget?: { kind: 'file' | 'link' | 'email'; label: string; tooltip?: string; onClick: () => ActionOutcome };
  onQrCode?: { active: boolean; label: string; onClick: () => ActionOutcome };
  onUpdate: (action: 'favorite' | 'pin' | 'delete') => ActionOutcome;
  onTranslate?: () => ActionOutcome; translateActive?: boolean; onOcr?: () => ActionOutcome; onNote?: () => void; busy?: boolean; retainDelete?: boolean; allowMarkedDelete?: boolean; tabIndex?: number; tooltipSide?: 'top' | 'bottom'; recordId?: string;
}) {
  const { doneKey, run } = useActionDone(recordId);
  const reduceMotion = useReducedMotion();
  const protectedRecord = !allowMarkedDelete && (favorite || pinned);
  const animatedDelete = !retainDelete && !allowMarkedDelete;
  const staticActions = [
    ...(onOpenTarget ? [{ id: 'openTarget', label: onOpenTarget.label, tooltip: onOpenTarget.tooltip ?? onOpenTarget.label, customIcon: onOpenTarget.kind === 'file' ? <FolderOpen aria-hidden="true" /> : <ExternalLink aria-hidden="true" />, onClick: () => { void run('openTarget', onOpenTarget.onClick); } }] : []),
    ...(onQrCode ? [{ id: 'qrcode', label: onQrCode.label, tooltip: onQrCode.label, customIcon: <ScanQrCode aria-hidden="true" />, pressed: onQrCode.active, onClick: () => { void run('qrcode', onQrCode.onClick, { animate: !onQrCode.active }); } }] : []),
    ...(onTranslate ? [{ id: 'translate', label: 'Translate', tooltip: t('translation'), description: t('clipboardTranslatePrivacy'), icon: 'languages' as const, pressed: translateActive, disabled: busy, onClick: () => { void run('translate', onTranslate, { animate: !translateActive }); } }] : []),
    ...(onOcr ? [{ id: 'ocr', label: 'Extract text', tooltip: t('extractText'), description: t('clipboardOcrPrivacy'), icon: 'ocr' as const, disabled: busy, onClick: () => { void run('ocr', onOcr); } }] : []),
    ...(onNote ? [{ id: 'note', label: 'Note', tooltip: t('recordNote'), icon: 'note' as const, onClick: onNote }] : []),
    { id: 'favorite', label: favorite ? t('unfavorite') : t('favorite'), tooltip: favorite ? t('unfavorite') : t('favorite'), icon: 'star' as const, pressed: favorite, onClick: () => { void run('favorite', () => onUpdate('favorite'), { animate: !favorite }); } },
    { id: 'pin', label: pinned ? t('unpinRecord') : t('pinRecord'), tooltip: pinned ? t('unpinRecord') : t('pinRecord'), icon: pinned ? 'pin.fill' as const : 'pin' as const, pressed: pinned, onClick: () => { void run('pin', () => onUpdate('pin'), { animate: !pinned }); } },
    ...(!animatedDelete && (!protectedRecord || retainDelete) ? [{ id: 'delete', label: deleteLabel, tooltip: t('clipboardDeleteConfirm'), icon: 'trash' as const, disabled: protectedRecord || busy, onClick: () => { void onUpdate('delete'); } }] : []),
  ];
  const renderButton = (action: typeof staticActions[number]) => {
    const description = 'description' in action ? action.description : undefined;
    const activeToggle = (action.id === 'qrcode' || action.id === 'translate') && 'pressed' in action && action.pressed;
    const done = doneKey === action.id;
    const className = [
      'icon-button',
      action.id === 'delete' && 'history-delete',
      action.id === 'favorite' && favorite && 'record-favorite',
      activeToggle && 'record-active',
      done && 'record-copied',
    ].filter(Boolean).join(' ');
    return <ActionButton className={className} data-tooltip={action.tooltip} data-tooltip-side={tooltipSide} aria-label={action.label} aria-description={description}
      tabIndex={tabIndex} aria-pressed={'pressed' in action ? action.pressed : undefined} disabled={!native || ('disabled' in action && action.disabled)} onClick={action.onClick}>{done ? <CircleCheck className="copy-success" aria-hidden="true" /> : 'customIcon' in action ? action.customIcon : <HistoryIcon name={action.icon} />}</ActionButton>;
  };
  const buttons = staticActions.map((action) => <span key={action.id} className="history-action" data-action={action.id}>{renderButton(action)}</span>);
  return <div className="history-row-actions" role="group" aria-label={t('actions')} onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onPointerDown={(event) => event.stopPropagation()} onMouseDown={(event) => {
    event.stopPropagation();
    if ((event.target as HTMLElement).closest('button')) event.preventDefault();
  }}>
    {buttons}
    {animatedDelete && <AnimatePresence initial={false}>
      {!protectedRecord && <motion.span key="delete" className="history-action" data-action="delete"
        initial={reduceMotion ? false : { opacity: 0, scale: .9, width: 0, x: 4 }}
        animate={{ opacity: 1, scale: 1, width: 24, x: 0 }}
        exit={reduceMotion ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, scale: .9, width: 0, x: 4 }}
        transition={{ duration: reduceMotion ? 0 : .16, ease: 'easeOut' }}>{renderButton({ id: 'delete', label: deleteLabel, tooltip: t('clipboardDeleteConfirm'), icon: 'trash', disabled: protectedRecord || busy, onClick: () => { void onUpdate('delete'); } })}</motion.span>}
    </AnimatePresence>}
  </div>;
}

function ActionButton(props: ComponentProps<'button'>) {
  const present = useIsPresent();
  return <button {...props} disabled={props.disabled || !present} aria-hidden={!present} inert={!present} tabIndex={present ? props.tabIndex : -1} onClick={present ? props.onClick : undefined} />;
}
