import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'lucide-react';
import type { Translate } from '../shared/i18n';
import { SettingsDialog } from './SettingsDialog';
import { bindingFromEvent, keycapLabels, modifierKey, shortcutActions, shortcutTitles, type KeyBinding, type ShortcutAction, type ShortcutModifier } from '../shared/shortcuts';

export function shortcutMessage(code: string, t: Translate) {
  if (code === 'shortcut_clipboard_panel') return t('clipboardShortcutConflict');
  if (code === 'shortcut_registration') return t('clipboardShortcutRegistration');
  if (code.startsWith('storage_')) return t('clipboardSettingsSaveFailed');
  if (code === 'shortcut_reserved_quick_paste') return t('shortcutReservedQuickPaste');
  if (code === 'shortcut_reserved_record_menu') return t('shortcutReservedRecordMenu');
  if (code === 'shortcut_reserved_close_window') return t('shortcutReservedCloseWindow');
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

export function ShortcutRow({ title, hint, unavailable, label, binding, macos, recording, enabled, saving, onEnabled, onToggle, onClear, onManual, t }: {
  title: string; hint?: string; unavailable?: string; label: string; binding: KeyBinding | null; macos: boolean;
  recording: boolean; enabled: boolean; saving?: boolean; onEnabled: (enabled: boolean) => void;
  onToggle: () => void; onClear: () => void; onManual: () => void; t: Translate;
}) {
  const id = useId();
  return <div className="settings-field settings-shortcut-row">
    <label className="settings-field-content" htmlFor={unavailable ? undefined : id} title={hint}>{title}</label>
    {unavailable ? <span className="settings-badge">{unavailable}</span> : <>
      <ShortcutRecorder label={label} binding={binding} macos={macos} recording={recording}
        saving={saving} onToggle={onToggle} onClear={onClear} onManual={onManual} t={t} />
      <input id={id} className="settings-checkbox" type="checkbox" aria-description={hint} aria-disabled={saving || undefined}
        checked={enabled} onChange={(event) => { if (!saving) onEnabled(event.target.checked); }} />
    </>}
  </div>;
}

function ShortcutRecorder({ label, binding, macos, recording, saving, onToggle, onClear, onManual, t }: {
  label: string; binding: KeyBinding | null; macos: boolean; recording: boolean; saving?: boolean;
  onToggle: () => void; onClear: () => void; onManual: () => void; t: Translate;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const keys = binding ? keycapLabels(binding, macos, t) : [];
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
    {recording && <button type="button" className="settings-action settings-manual-shortcut" onClick={() => { trigger.current?.focus({ preventScroll: true }); onManual(); }}>{t('chooseShortcut')}</button>}
    {binding && <button type="button" className="settings-clear" aria-label={t('clearShortcut')}
      disabled={saving} onClick={() => { trigger.current?.focus({ preventScroll: true }); onClear(); }}><X aria-hidden="true" /></button>}
  </div>;
}

export function ShortcutCapture({ onCancel, onCommit }: { onCancel: () => void; onCommit: (binding: KeyBinding) => void }) {
  const cancel = useRef(onCancel);
  const commit = useRef(onCommit);
  cancel.current = onCancel;
  commit.current = onCommit;
  useEffect(() => {
    void invoke('translation_shortcut_capture', { active: true }).catch(() => {});
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat || event.isComposing) return;
      const binding = bindingFromEvent(event);
      if (!binding || modifierKey(binding.code)) return;
      event.preventDefault();
      event.stopPropagation();
      commit.current(binding);
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

const chooseModifiers: readonly [ShortcutModifier, string, string][] = [
  ['control', 'Control', '⌃'], ['alt', 'Option', '⌥'], ['shift', 'Shift', '⇧'], ['meta', 'Command', '⌘'],
];
const chooseCodes = [
  ...Array.from('ABCDEFGHIJKLMNOPQRSTUVWXYZ', (letter) => `Key${letter}`),
  ...Array.from('0123456789', (digit) => `Digit${digit}`),
  'Space', 'Enter', 'Tab', 'Escape', 'Backspace', 'Delete', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
  'Home', 'End', 'PageUp', 'PageDown', 'Insert', 'CapsLock',
  'Backquote', 'Minus', 'Equal', 'BracketLeft', 'BracketRight', 'Backslash', 'Semicolon', 'Quote', 'Comma', 'Period', 'Slash',
  ...Array.from({ length: 24 }, (_, index) => `F${index + 1}`),
  ...chooseModifiers.flatMap(([_, name]) => {
    const code = name === 'Command' ? 'Meta' : name === 'Option' ? 'Alt' : name;
    return [`${code}Left`, `${code}Right`];
  }),
];

export function ManualShortcut({ macos, t, onCancel, onCommit }: {
  macos: boolean; t: Translate; onCancel: () => void; onCommit: (binding: KeyBinding) => void;
}) {
  const [code, setCode] = useState('');
  const [modifiers, setModifiers] = useState<ShortcutModifier[]>([]);
  return <SettingsDialog role="dialog" title={t('chooseShortcut')} onClose={onCancel}>
    <form onSubmit={(event) => { event.preventDefault(); if (code) onCommit({ code, modifiers: modifierKey(code) ? [] : modifiers }); }}>
      <div className="settings-manual-keys">
        <label>{t('shortcutMainKey')}<select data-initial-focus value={code} onChange={(event) => setCode(event.target.value)}>
          <option value="" disabled>{t('shortcutMainKey')}</option>
          {chooseCodes.map((key) => <option key={key} value={key}>{keycapLabels({ code: key, modifiers: [] }, macos, t)[0]}</option>)}
        </select></label>
        <fieldset disabled={Boolean(modifierKey(code))}><legend>{t('shortcutModifiers')}</legend>
          {chooseModifiers.map(([modifier, name, symbol]) => <label key={modifier}>
            <input type="checkbox" className="settings-checkbox" checked={modifiers.includes(modifier)} onChange={(event) => {
              setModifiers(event.target.checked ? [...modifiers, modifier] : modifiers.filter((current) => current !== modifier));
            }} />{macos ? `${symbol} ${name}` : name}
          </label>)}
        </fieldset>
      </div>
      <div className="settings-dialog-actions"><button type="button" className="settings-action" onClick={onCancel}>{t('shortcutManualCancel')}</button><button type="submit" className="settings-action" disabled={!code}>{t('shortcutManualSave')}</button></div>
    </form>
  </SettingsDialog>;
}
