import { useEffect, useId, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'lucide-react';
import type { Translate } from '../shared/i18n';
import { bindingFromEvent, keycapLabels, shortcutActions, shortcutTitles, type KeyBinding, type ShortcutAction } from '../shared/shortcuts';

export function shortcutMessage(code: string, t: Translate) {
  if (code === 'shortcut_clipboard_panel') return t('clipboardShortcutConflict');
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

export function ShortcutRow({ title, hint, unavailable, label, binding, macos, recording, enabled, saving, onEnabled, onToggle, onClear, t }: {
  title: string; hint?: string; unavailable?: string; label: string; binding: KeyBinding | null; macos: boolean;
  recording: boolean; enabled: boolean; saving?: boolean; onEnabled: (enabled: boolean) => void;
  onToggle: () => void; onClear: () => void; t: Translate;
}) {
  const id = useId();
  return <div className="settings-field settings-shortcut-row">
    <label className="settings-field-content" htmlFor={unavailable ? undefined : id} title={hint}>{title}</label>
    {unavailable ? <span className="settings-badge">{unavailable}</span> : <>
      <ShortcutRecorder label={label} binding={binding} macos={macos} recording={recording}
        saving={saving} onToggle={onToggle} onClear={onClear} t={t} />
      <input id={id} className="settings-checkbox" type="checkbox" aria-description={hint} aria-disabled={saving || undefined}
        checked={enabled} onChange={(event) => { if (!saving) onEnabled(event.target.checked); }} />
    </>}
  </div>;
}

function ShortcutRecorder({ label, binding, macos, recording, saving, onToggle, onClear, t }: {
  label: string; binding: KeyBinding | null; macos: boolean; recording: boolean; saving?: boolean;
  onToggle: () => void; onClear: () => void; t: Translate;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const keys = binding ? keycapLabels(binding, macos) : [];
  useEffect(() => { if (recording) trigger.current?.focus({ preventScroll: true }); }, [recording]);

  return <div className="settings-recorder-wrap" data-recording={recording || undefined}>
    <button ref={trigger} type="button" className="settings-recorder" aria-label={label}
      aria-description={recording ? t('shortcutRecordingInstructions') : keys.join(' ') || undefined}
      aria-pressed={recording} aria-disabled={saving || undefined}
      onClick={() => { if (!saving) onToggle(); }}>
      {recording ? <span className="settings-recorder-prompt">{t('pressShortcut')}</span>
        : binding ? <span className="keycaps" aria-hidden="true">{keys.map((key) => <kbd key={key}>{key}</kbd>)}</span>
          : <span className="settings-recorder-empty">{t('addHotkey')}</span>}
    </button>
    {binding && <button type="button" className="settings-clear" aria-label={t('clearShortcut')}
      disabled={saving} onClick={() => { trigger.current?.focus({ preventScroll: true }); onClear(); }}><X aria-hidden="true" /></button>}
  </div>;
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
      if ((event.key === 'Backspace' || event.key === 'Delete') && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) { clear.current(); return; }
      const binding = bindingFromEvent(event);
      if (binding) commit.current(binding);
    };
    const onPointer = (event: MouseEvent) => {
      const row = document.querySelector('[data-recording]');
      if (row && event.target instanceof Node && row.contains(event.target)) return;
      cancel.current();
    };
    const onBlur = () => cancel.current();
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
