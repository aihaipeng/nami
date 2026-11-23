import { useEffect, useRef } from 'react';
import type { ViewProps } from '../App';
import { Clipboard } from '../clipboard/Clipboard';
import { HistoryPanel } from '../translation/HistoryPanel';
import type { HistoryRecord } from '../translation/history';
import { editsText, matchesBinding } from './shortcuts';
import { translationShortcutEnabled } from './contracts';
import { clipboardKeyAction, clipboardPanelShortcuts } from './clipboardShortcuts';

export function RecordsWorkspace({ selectedId = null, onSelect, ...props }: ViewProps & {
  selectedId?: string | null;
  onSelect?: (record: HistoryRecord, retranslate?: boolean, preview?: boolean) => void;
}) {
  const { snapshot, command } = props;
  const view = snapshot.window.recordView ?? 'clipboard';
  const visible = snapshot.window.historyVisible;
  const workspace = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!visible) return;
    const keydown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.key === 'Dead') return;
      const target = event.target;
      if (document.querySelector('.clipboard-editor-layer, .clipboard-context-menu, .history-shortcuts-panel')) return;
      const root = workspace.current;
      if (!root || (target instanceof HTMLElement && target !== document.body && !root.contains(target))) return;
      const preferences = snapshot.translationPreferences;
      if (!snapshot.capabilities.selection && !editsText(event) && translationShortcutEnabled(preferences, 'toggleHistory')
        && matchesBinding(preferences.shortcuts.toggleHistory, event)) {
        event.preventDefault();
        if (!event.repeat) void command('history_toggle');
        return;
      }
      const action = clipboardKeyAction(clipboardPanelShortcuts(snapshot), event);
      if (target instanceof HTMLElement && target !== document.body && !target.closest('.history-row-select, .history-search input')) return;
      if (action !== 'switchView') return;
      event.preventDefault();
      if (!event.repeat) void command('records_view_set', { view: view === 'clipboard' ? 'translation' : 'clipboard' });
    };
    const copy = (event: ClipboardEvent) => {
      // WebKit can deliver the native Copy command without a keydown.
      if (snapshot.capabilities.platform !== 'macos' || event.defaultPrevented || !document.getSelection()?.isCollapsed) return;
      const row = document.activeElement?.closest('.history-row-select');
      if (!row || !workspace.current?.contains(row) || row.closest('[hidden], [inert]')) return;
      const key = new KeyboardEvent('keydown', { key: 'c', code: 'KeyC', metaKey: true, bubbles: true, cancelable: true });
      if (clipboardKeyAction(clipboardPanelShortcuts(snapshot), key) === 'copyRecord' && !row.dispatchEvent(key)) event.preventDefault();
    };
    window.addEventListener('keydown', keydown, true);
    window.addEventListener('copy', copy);
    return () => {
      window.removeEventListener('keydown', keydown, true);
      window.removeEventListener('copy', copy);
    };
  }, [visible, snapshot, view, command]);
  useEffect(() => {
    if (!visible || view !== 'translation') return;
    const dismiss = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
      if (event.key === 'Escape' || matchesBinding({ code: 'KeyW', modifiers: [snapshot.capabilities.primaryModifier] }, event)) {
        event.preventDefault(); void command('window_hide', { region: 'translation' });
      }
    };
    window.addEventListener('keydown', dismiss);
    return () => window.removeEventListener('keydown', dismiss);
  }, [visible, view, snapshot.capabilities.primaryModifier, command]);
  return <div ref={workspace} className="records-workspace" data-record-view={view}>
    <div className="record-view" hidden={view !== 'clipboard'} inert={!visible || view !== 'clipboard'}>
      <Clipboard {...props} active={visible && view === 'clipboard'} />
    </div>
    <div className="record-view" hidden={view !== 'translation'} inert={!visible || view !== 'translation'}>
      <HistoryPanel {...props} active={visible && view === 'translation'} selectedId={selectedId}
        onSelect={onSelect ?? ((record, retranslate = false, preview = false) => { void command('history_sample_select', { sampleId: record.id, retranslate, ...(preview ? { preview: true } : {}) }); })}
        onKeyDown={() => {}} />
    </div>
  </div>;
}
