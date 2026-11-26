import { useEffect, useRef } from 'react';
import type { ViewProps } from '../App';
import { Clipboard } from '../clipboard/Clipboard';
import { clipboardKeyAction, clipboardPanelShortcuts } from './clipboardShortcuts';

export function RecordsWorkspace(props: ViewProps) {
  const { snapshot } = props;
  const visible = snapshot.window.clipboardVisible;
  const workspace = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!visible) return;
    const copy = (event: ClipboardEvent) => {
      // WebKit can deliver the native Copy command without a keydown.
      if (snapshot.capabilities.platform !== 'macos' || event.defaultPrevented || !document.getSelection()?.isCollapsed) return;
      if (event.target instanceof Element && event.target.closest('input, textarea, [contenteditable=true]')) return;
      const row = document.activeElement?.closest('.history-row-select');
      if (!row || !workspace.current?.contains(row) || row.closest('[hidden], [inert]')) return;
      const key = new KeyboardEvent('keydown', { key: 'c', code: 'KeyC', metaKey: true, bubbles: true, cancelable: true });
      if (clipboardKeyAction(clipboardPanelShortcuts(snapshot), key) === 'copyRecord' && !row.dispatchEvent(key)) event.preventDefault();
    };
    window.addEventListener('copy', copy);
    return () => {
      window.removeEventListener('copy', copy);
    };
  }, [visible, snapshot]);
  return <div ref={workspace} className="records-workspace" data-record-view="clipboard">
    <div className="record-view" inert={!visible}>
      <Clipboard {...props} active={visible} />
    </div>
  </div>;
}
