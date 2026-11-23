import { Fragment, useRef, type ReactNode } from 'react';
import { keycapLabels, matchesBinding, modifierKey, type KeyBinding } from './shortcuts';
import type { Translate } from './i18n';
import { FloatingPanel } from './FloatingPanel';

export interface RecordMenuItem { action: string; label: string; icon: ReactNode; description?: string; disabled?: boolean; destructive?: boolean; active?: boolean; separatorBefore?: boolean; shortcut?: { binding: KeyBinding; label: string; aria?: string } | null }

const navigation = ['ArrowDown', 'ArrowUp', 'Home', 'End'];

function recordMenuAllowsBinding(action: string, { code, modifiers }: KeyBinding) {
  return code !== 'Escape' && code !== 'Tab' && (modifiers.length > 0 || (code === 'Enter' ? action === 'paste' : code !== 'Space' && !navigation.includes(code)));
}

export function recordMenuItems(groups: readonly (readonly RecordMenuItem[])[], mac: boolean, t: Translate, bindingFor: (action: string) => KeyBinding | null | undefined) {
  return groups.filter((group) => group.length).flatMap((group, index) => group.map((item, row) => {
    const binding = !item.disabled && bindingFor(item.action);
    const separatorBefore = index > 0 && row === 0;
    if (!binding || !recordMenuAllowsBinding(item.action, binding)) return { ...item, separatorBefore, shortcut: null };
    const keys = keycapLabels(binding, mac, t);
    const symbol = keys.at(-1) ?? '';
    const ariaKey = /^[!-~]$/.test(symbol) ? symbol : binding.code;
    const aria = modifierKey(binding.code) ? undefined : [...binding.modifiers.map((modifier) => modifier[0].toUpperCase() + modifier.slice(1)), ariaKey].join('+');
    return { ...item, separatorBefore, shortcut: { binding, label: keys.join(mac ? '' : '+'), aria } };
  }));
}

export function RecordMenu({ x, y, items, onClose, onReturnFocus, onAction }: {
  x: number; y: number; items: readonly RecordMenuItem[];
  onClose: () => void; onReturnFocus: () => void; onAction: (action: string) => void;
}) {
  const anchor = useRef<HTMLSpanElement>(null);
  const showKeys = items.some((item) => item.shortcut !== undefined);
  return <>
    <span ref={anchor} tabIndex={-1} onFocus={onReturnFocus} className="record-menu-anchor" style={{ left: x, top: y }} />
    <FloatingPanel anchor={anchor} onClose={onClose} label="Actions" role="menu" width={showKeys ? 220 : 176} side="bottom" className={`record-context-menu clipboard-context-menu${showKeys ? ' record-context-menu-keys' : ''}`}>
      <div onKeyDown={(event) => {
        if (event.defaultPrevented || event.nativeEvent.isComposing || event.keyCode === 229 || event.key === 'Dead') return;
        const item = items.find((item) => !item.disabled && item.shortcut && matchesBinding(item.shortcut.binding, event));
        if (item) {
          event.preventDefault(); event.stopPropagation();
          if (!event.repeat) { onReturnFocus(); onAction(item.action); }
          return;
        }
        // An unbound modified Return is not the focused button's activation key.
        if (event.key === 'Enter' && (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey)) {
          event.preventDefault();
          return;
        }
        if (!navigation.includes(event.key)) return;
        event.preventDefault();
        const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
        const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
        buttons[next]?.focus();
      }}>
        {items.map((item) => <Fragment key={item.action}>
          {item.separatorBefore && <div role="separator" className="record-menu-divider" />}
          <button type="button" role="menuitem" disabled={item.disabled} onClick={() => { if (item.action !== 'note') onReturnFocus(); onAction(item.action); }}
            title={item.description} aria-description={item.description} aria-keyshortcuts={item.shortcut?.aria}
            className={item.destructive ? 'clipboard-menu-delete' : item.action === 'favorite' && item.active ? 'record-favorite' : undefined}>
            {item.icon}<span className="record-menu-label">{item.label}</span>
            {item.shortcut !== undefined && <kbd className="record-menu-shortcut" aria-hidden="true">{item.shortcut?.label ?? '—'}</kbd>}
          </button>
        </Fragment>)}
      </div>
    </FloatingPanel>
  </>;
}
