import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Translate } from '../shared/i18n';
import { bindingFromEvent, shortcutActions, type KeyBinding, type ShortcutAction } from '../shared/shortcuts';

export const shortcutTitles = {
  selection: 'translateSelected',
  translate: 'translation',
  copyPrimary: 'copyPrimary',
  copySecondary: 'copySecondary',
  pin: 'pinWindow',
  actions: 'actions',
} as const;

export function shortcutMessage(code: string, t: Translate) {
  if (code === 'shortcut_registration') return t('clipboardShortcutRegistration');
  if (code.startsWith('storage_')) return t('clipboardSettingsSaveFailed');
  if (code === 'shortcut_reserved') return t('shortcutReserved');
  if (code === 'shortcut_modifier') return t('shortcutModifier');
  if (code === 'shortcut_clipboard') return t('shortcutClipboard');
  if (code.startsWith('shortcut_conflict:')) {
    const action = code.slice('shortcut_conflict:'.length) as ShortcutAction;
    return shortcutActions.includes(action) ? `${t('shortcutConflict')} ${t(shortcutTitles[action])}.` : t('shortcutKey');
  }
  return t('shortcutKey');
}

export function errorCode(error: unknown) {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return 'shortcut_key';
}

export function ShortcutCapture({ onCancel, onCommit, onClear, onCycle }: { onCancel: () => void; onCommit: (binding: KeyBinding) => void; onClear: () => void; onCycle?: (backward: boolean) => void }) {
  const cancel = useRef(onCancel);
  const commit = useRef(onCommit);
  const clear = useRef(onClear);
  const cycle = useRef(onCycle);
  cancel.current = onCancel;
  commit.current = onCommit;
  clear.current = onClear;
  cycle.current = onCycle;
  useEffect(() => {
    void invoke('translation_shortcut_capture', { active: true }).catch(() => {});
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat || event.isComposing) return;
      if (event.key === 'Tab' && !cycle.current) { cancel.current(); return; }
      event.preventDefault();
      event.stopPropagation();
      if (event.key === 'Escape') { cancel.current(); return; }
      if (event.key === 'Tab') { cycle.current?.(event.shiftKey); return; }
      if (event.key === 'Backspace' || event.key === 'Delete') { clear.current(); return; }
      const binding = bindingFromEvent(event);
      if (binding) commit.current(binding);
    };
    const onPointer = (event: MouseEvent) => {
      const row = document.querySelector('[data-recording]');
      if (row && event.target instanceof Node && row.contains(event.target)) return;
      cancel.current();
    };
    const onBlur = () => { if (!cycle.current) cancel.current(); };
    window.addEventListener('blur', onBlur);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('mousedown', onPointer, true);
    return () => {
      window.removeEventListener('blur', onBlur);
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('mousedown', onPointer, true);
      void invoke('translation_shortcut_capture', { active: false }).catch(() => {});
    };
  }, []);
  return null;
}
