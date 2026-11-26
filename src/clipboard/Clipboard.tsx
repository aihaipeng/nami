import { clipboardOpenError, clipboardOpenTarget } from './openTarget';
import { focusRecordList, useRecordPointer, useRecordsOpenFocus } from '../shared/recordFocus';
import { useRecordsPreviewShortcut } from '../shared/recordPreviewShortcut';
import { useCallback, useEffect, useEffectEvent, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import type { ClipboardResult } from '../shared/contracts';
import type { MessageKey } from '../shared/i18n';
import { ActivityBeam } from '../shared/ActivityBeam';
import { RecordToolbar } from '../shared/RecordToolbar';
import { useRecordBrowser } from '../shared/useRecordBrowser';
import { historyPreview, historyScrollTarget } from '../shared/recordPreview';
import { cn } from '../lib/utils';
import { RecordHeading } from '../shared/RecordHeading';
import { RecordActions } from '../shared/RecordActions';
import { RecordFeedback } from '../shared/RecordFeedback';
import { useClipboard, type ClipboardKind, type ClipboardRecord } from './useClipboard';
import { HistoryList, type HistoryListHandle } from '../shared/HistoryList';
import { clipboardCategoryDirection, clipboardKeyAction, clipboardPanelShortcuts, clipboardShortcutAvailable, isClipboardTranslateLaunchKey } from '../shared/clipboardShortcuts';
import { matchesBinding } from '../shared/shortcuts';
import { HistoryFooter } from '../shared/HistoryFooter';
import { ClipboardMedia } from './ClipboardMedia';
import { ClipboardNotePreview } from './ClipboardNotePreview';
import { ClipboardTextPreview } from './ClipboardTextPreview';
import { ClipboardEditor } from './ClipboardEditor';
import { ClipboardMenu } from './ClipboardMenu';
import { clipboardProcessError, clipboardUseError, clipboardUseFeedback } from './feedback';
import './clipboard.css';

const clipboardRecordKey = (record: ClipboardRecord) => record.id;

function isQuickPasteModifier(event: KeyboardEvent) {
  return event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey && !event.isComposing && event.keyCode !== 229;
}

export function Clipboard({ t, snapshot, command, native, active = true }: ViewProps & { active?: boolean }) {
  const composing = useRef(false);
  const [session, setSession] = useState(0);
  const [showQuickPasteHints, setShowQuickPasteHints] = useState(false);
  const [kindFilter, setKindFilter] = useState<ClipboardKind | null>(null);
  const [processing, setProcessing] = useState<string | null>(null);
  const [processingError, setProcessingError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ id: string } | null>(null);
  const [savingImage, setSavingImage] = useState(false);
  const [resumingCapture, setResumingCapture] = useState(false);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<MessageKey | null>(null);
  const operation = useRef(0);
  const processingRevision = useRef(0);
  const reveal = useRef<string | null>(null);
  const revealContext = useRef({ browsing: true });
  const mounted = useRef(true);
  const refocus = useRef<{ id: string; previous: HTMLElement; neighbor?: string } | null>(null);
  const editorReturn = useRef<string | null>(null);
  const rowRefs = useRef(new Map<string, HTMLButtonElement>());
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const listHandle = useRef<HistoryListHandle>(null);
  const pendingFocus = useRef<string | null>(null);
  const browser = useRecordBrowser(snapshot.window.recordsSession, active, list, () => clipboard.displayReady, () => clipboard.page.records, kindFilter);
  const { query, setQuery, favoritesOnly, setFavoritesOnly, remember, restoreScroll } = browser;
  const searchOptions = snapshot.clipboardControls.search;
  const searchKey = searchOptions && (searchOptions.mode !== 'exact' || !searchOptions.body || !searchOptions.note || !searchOptions.imageText)
    ? `${searchOptions.mode}:${+searchOptions.body}${+searchOptions.note}${+searchOptions.imageText}` : '';
  const clipboard = useClipboard(native, query, favoritesOnly, () => (list.current?.scrollTop ?? 0) > 0, kindFilter, searchKey);
  const records = clipboard.page.records;
  const [imageFiles, setImageFiles] = useState(new Set<string>());
  const onImageFile = useCallback((id: string, eligible: boolean) => {
    setImageFiles((previous) => {
      if (previous.has(id) === eligible) return previous;
      const next = new Set(previous);
      if (eligible) next.add(id); else next.delete(id);
      return next;
    });
  }, []);
  const menuRecord = menu ? records.find((record) => record.id === menu.id) : null;
  const canTranslateRecord = (record: ClipboardRecord) => record.kind === 'text' ? !!record.text.trim() : snapshot.capabilities.imageOcr && (record.kind === 'image' || imageFiles.has(record.id));
  const canOcrRecord = (record: ClipboardRecord) => record.kind !== 'text' && canTranslateRecord(record) && snapshot.capabilities.clipboardWrite;
  const selection = snapshot.window.clipboardDetailVisible ? snapshot.window.clipboardDetailId : selected;
  const visibleIds = new Set(records.map(({ id }) => id));
  const ordinary = useMemo(() => records.filter((record) => !record.isPinned), [records]);
  const allUnpinned = useMemo(() => clipboard.retainedRecords.filter((record) => !record.isPinned), [clipboard.retainedRecords]);
  const ordered = [...records.filter((record) => record.isPinned), ...ordinary];
  const quickPasteCandidates = ordinary.slice(0, 5);
  const quickPaste = native && snapshot.capabilities.platform === 'macos' && snapshot.capabilities.autoPaste && !snapshot.storageError;
  const first = records.find((record) => !record.isPinned) ?? records[0];
  const targetOnMouseMove = useRecordPointer();
  const keyboardId = clipboard.displayReady ? (selected && visibleIds.has(selected) ? selected : first?.id) : undefined;
  useEffect(() => {
    if (!active || !quickPaste) { setShowQuickPasteHints(false); return; }
    const update = (event: KeyboardEvent) => setShowQuickPasteHints(isQuickPasteModifier(event));
    const reset = () => setShowQuickPasteHints(false);
    window.addEventListener('keydown', update, true);
    window.addEventListener('keyup', update, true);
    window.addEventListener('blur', reset);
    return () => {
      window.removeEventListener('keydown', update, true);
      window.removeEventListener('keyup', update, true);
      window.removeEventListener('blur', reset);
    };
  }, [active, quickPaste]);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let remove: (() => void) | undefined;
    void listen<{ label: string; visible: boolean }>('window://visibility', ({ payload }) => {
      if (disposed || payload.label !== 'clipboard' || snapshot.window.recordsSession !== undefined) return;
      ++operation.current;
      ++processingRevision.current;
      setEditing(null); setMenu(null); setRevealed(null); setSelected(null); setFeedback(null);
      setQuery(''); setFavoritesOnly(false); setKindFilter(null); setProcessing(null); setProcessingError(null); setSession((value) => value + 1);
      list.current?.scrollTo(0, 0); pinnedList.current?.scrollTo(0, 0);
      if (!payload.visible) (document.activeElement as HTMLElement | null)?.blur();
    }).then((unlisten) => { if (disposed) unlisten(); else remove = unlisten; }).catch(() => { if (!disposed) setFeedback('nativeError'); });
    return () => { disposed = true; remove?.(); };
  }, [native, snapshot.window.recordsSession]);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const subscription = listen<ClipboardResult>('clipboard_use_failed', ({ payload }) => {
      if (disposed) return;
      setFeedback(clipboardUseFeedback(payload));
    });
    return () => { disposed = true; void subscription.then((remove) => remove()); };
  }, [native]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++operation.current; }; }, []);
  useEffect(() => { setMenu(null); setRevealed(null); }, [query, favoritesOnly, kindFilter]);
  useEffect(() => {
    const dismiss = (event: Event) => {
      if (!(event.target instanceof Element && event.target.closest('.record-context-menu-keys'))) setMenu(null);
    };
    window.addEventListener('scroll', dismiss, true);
    return () => window.removeEventListener('scroll', dismiss, true);
  }, []);
  useEffect(() => {
    ++operation.current;
    ++processingRevision.current;
    setKindFilter(null); setProcessing(null); setProcessingError(null);
    setEditing(null); setMenu(null); setRevealed(null); setFeedback(null); setSession((value) => value + 1);
  }, [snapshot.window.recordsSession]);
  useEffect(() => { if (!active) { ++operation.current; ++processingRevision.current; setEditing(null); setMenu(null); setRevealed(null); setFeedback(null); setProcessing(null); setProcessingError(null); setSession((value) => value + 1); } }, [active]);
  useRecordsOpenFocus(search, active, snapshot.window.clipboardVisible, snapshot.window.recordsFocus, snapshot.clipboardControls.recordsOpenFocus === 'list',
    clipboard.displayReady, !clipboard.loading, setSelected);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<string>('clipboard_note_requested', ({ payload }) => {
      if (!disposed) { setMenu(null); setEditing({ id: payload }); }
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native]);

  revealContext.current = { browsing: snapshot.window.recordsBrowsing !== false };
  function previewRecord(id: string, explicit = false) {
    if (!explicit && snapshot.window.recordsBrowsing === false) return;
    if (!snapshot.window.clipboardDetailVisible || !explicit && snapshot.window.clipboardDetailId === id || editing) return;
    void command('clipboard_details_select', { recordId: id, ...(explicit ? { browsing: true } : {}) });
  }
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<string>('clipboard_text_created', ({ payload }) => {
      if (disposed || !payload || revealContext.current.browsing) return;
      reveal.current = payload;
      setKindFilter(null); setFavoritesOnly(false); setQuery('');
      list.current?.scrollTo(0, 0);
      clipboard.resetFilters({ query: '', favoritesOnly: false, kind: null });
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native, clipboard.resetFilters, setFavoritesOnly, setQuery]);
  useEffect(() => {
    const id = reveal.current;
    if (snapshot.window.recordsBrowsing) { reveal.current = null; return; }
    if (!active || !id || query || favoritesOnly || kindFilter || clipboard.loading || !clipboard.displayReady) return;
    if (!records.some((record) => record.id === id)) { reveal.current = null; return; }
    reveal.current = null;
    setSelected(id);
    if (document.activeElement?.closest('.clipboard-panel .history-row')) rowRefs.current.get(id)?.focus({ preventScroll: true });
    scrollToRecord(id);
  }, [active, records, query, favoritesOnly, kindFilter, clipboard.loading, clipboard.displayReady, snapshot.window.recordsBrowsing]);
  function scrollToRecord(id: string) {
    const element = rowRefs.current.get(id)?.closest<HTMLElement>('.history-row');
    const scroller = element?.closest<HTMLElement>('.history-list');
    if (element && scroller) {
      const bounds = scroller.getBoundingClientRect();
      const top = historyScrollTarget(scroller.scrollTop, { top: bounds.top, bottom: bounds.bottom }, element.getBoundingClientRect());
      if (top !== null) scroller.scrollTo({ top, behavior: 'instant' });
      return;
    }
    listHandle.current?.scrollToId(id, 'instant');
  }
  useEffect(() => {
    if (!active || !snapshot.window.clipboardDetailVisible || clipboard.loading || !clipboard.displayReady || reveal.current) return;
    if (snapshot.window.clipboardDetailId && records.some(({ id }) => id === snapshot.window.clipboardDetailId)) return;
    if ((first?.id ?? null) !== (snapshot.window.clipboardDetailId ?? null)) void command('clipboard_details_select', { recordId: first?.id ?? null });
  }, [active, snapshot.window.clipboardDetailVisible, snapshot.window.clipboardDetailId, records, clipboard.loading, clipboard.displayReady, first, command]);
  async function toggleDetails() {
    if (!snapshot.window.clipboardDetailVisible) {
      await command('clipboard_details_select', { recordId: keyboardId ?? null, browsing: true });
    }
    await command('records_details_toggle');
  }

  async function useRecord(recordId: string, action: 'copy' | 'copyPlain' | 'paste' | 'pastePlain' | 'pastePaths' | 'activate' | 'activateAlternate') {
    const needsPaste = action === 'paste' || action === 'pastePlain' || action === 'pastePaths'
      || (action === 'activateAlternate' && snapshot.clipboardControls.activation === 'copy');
    if (!native || (needsPaste ? !snapshot.capabilities.autoPaste : !snapshot.capabilities.clipboardWrite)) return false;
    const current = ++operation.current;
    setSelected(recordId); setFeedback(null);
    try {
      const result = await invoke<ClipboardResult>('clipboard_record_use', { recordId, action });
      if (!mounted.current) return false;
      // Paste promotes the original; a standalone copy arrives as a new record through clipboard_changed.
      if (result.copyStatus === 'current' && result.status !== 'copied') clipboard.noteCopied(recordId);
      if (operation.current !== current) return false;
      setFeedback(clipboardUseFeedback(result));
      return result.copyStatus === 'current' && result.status === 'copied';
    } catch (error) {
      if (mounted.current && operation.current === current) setFeedback(clipboardUseError(error));
      return false;
    }
  }

  async function saveImage(recordId: string) {
    if (!native || !snapshot.capabilities.clipboardImageSave || savingImage) return;
    const current = ++operation.current;
    focusRow(recordId); setFeedback(null); setProcessingError(null); setSavingImage(true);
    try {
      const saved = await invoke<boolean>('clipboard_image_save', { recordId });
      if (saved && mounted.current && operation.current === current) setFeedback('clipboardImageSaved');
    } catch (error) {
      if (mounted.current && operation.current === current) {
        const code = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
        setFeedback(code === 'clipboard_image_missing' || code === 'storage_corrupt' ? 'clipboardImageUnavailable' : 'clipboardImageSaveFailed');
      }
    } finally {
      if (mounted.current) setSavingImage(false);
    }
  }

  async function dragRecord(recordId: string) {
    if (!native || !snapshot.capabilities.clipboardDrag) return;
    const current = ++operation.current;
    setFeedback(null);
    setProcessingError(null);
    try {
      await invoke('clipboard_record_drag', { recordId });
    } catch (error) {
      if (!mounted.current || operation.current !== current) return;
      const code = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
      if (code === 'cancelled') return;
      setFeedback(code === 'clipboard_file_missing' || code === 'clipboard_image_missing' || code === 'storage_corrupt'
        ? clipboardUseError(error)
        : 'clipboardDragFailed');
    }
  }

  async function processRecord(recordId: string, action: 'translate' | 'ocr') {
    if (!native || processing !== null) return;
    const current = ++operation.current;
    const revision = ++processingRevision.current;
    setProcessing(recordId); setProcessingError(null); setFeedback(null);
    try {
      await invoke('clipboard_record_process', { recordId, action });
    } catch (error) {
      if (mounted.current && operation.current === current && String(error) !== 'cancelled') {
        setProcessingError(clipboardProcessError(t, error, action));
      }
    } finally {
      if (mounted.current && processingRevision.current === revision) setProcessing(null);
    }
  }

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (!active || event.defaultPrevented || event.isComposing || event.keyCode === 229 || (editing && event.key === 'Escape')) return;
      if (event.key === 'Escape' || matchesBinding({ code: 'KeyW', modifiers: [snapshot.capabilities.primaryModifier] }, event)) {
        event.preventDefault(); ++operation.current;
        if (snapshot.window.clipboardTranslationVisible) void command('clipboard_translation_close');
        else void command('window_hide', { region: 'clipboard' });
      }
    };
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [snapshot.capabilities.primaryModifier, snapshot.window.clipboardTranslationVisible, command, editing, active]);
  function focusRow(id: string) {
    setSelected(id);
    const button = rowRefs.current.get(id);
    if (button) {
      pendingFocus.current = null;
      button.focus();
      return;
    }
    if (listHandle.current?.scrollToId(id, 'instant')) pendingFocus.current = id;
  }
  const returnFocus = useEffectEvent((id: string | null) => {
    if (!active || editing) return;
    if (clipboard.displayReady && id && visibleIds.has(id)) focusRow(id);
    else search.current?.focus({ preventScroll: true });
  });
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const subscription = listen<string | null>('clipboard_focus_requested', ({ payload }) => { if (!disposed) returnFocus(payload); });
    return () => { disposed = true; void subscription.then((remove) => remove()).catch(() => {}); };
  }, [native]);
  useLayoutEffect(() => {
    const id = pendingFocus.current;
    if (!id) return;
    if (!active || !visibleIds.has(id)) { pendingFocus.current = null; return; }
    const button = rowRefs.current.get(id);
    if (button) { pendingFocus.current = null; button.focus({ preventScroll: true }); }
  });
  useLayoutEffect(() => {
    const pending = refocus.current;
    if (!pending) return;
    if (!active || document.activeElement !== pending.previous && document.activeElement !== document.body) { refocus.current = null; return; }
    if (pending.previous.isConnected) return;
    refocus.current = null;
    const next = visibleIds.has(pending.id) ? pending.id : pending.neighbor;
    if (next && visibleIds.has(next)) focusRow(next);
    else search.current?.focus();
  });
  useLayoutEffect(() => {
    if (editing !== null || !editorReturn.current) return;
    const id = editorReturn.current;
    editorReturn.current = null;
    if (active && visibleIds.has(id)) focusRow(id);
    else if (active) search.current?.focus({ preventScroll: true });
  });
  const shortcuts = clipboardPanelShortcuts(snapshot);
  const usableShortcut = (action: typeof shortcuts[number]['action']) => clipboardShortcutAvailable(snapshot, native, action);

  async function openRecordTarget(recordId: string) {
    const revision = operation.current;
    setFeedback(null); setProcessingError(null);
    try {
      const result = await clipboardOpenTarget(recordId, true);
      if (result !== 'files' || !mounted.current || operation.current !== revision) return;
      if (snapshot.window.clipboardDetailVisible) {
        await command('clipboard_details_select', { recordId, browsing: true });
      } else {
        await command('records_details_toggle', { recordId });
      }
    } catch (error) {
      if (mounted.current && operation.current === revision) setFeedback(clipboardOpenError(error));
    }
  }

  function updateRecord(record: ClipboardRecord, action: 'favorite' | 'pin' | 'delete', source?: HTMLElement | null) {
    if (action === 'delete') {
      if (processing) return;
      setFeedback(null);
      const index = ordered.findIndex(({ id }) => id === record.id);
      const origin = source ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
      if (origin?.closest('article')) {
        refocus.current = { id: record.id, previous: origin, neighbor: (ordered[index + 1] ?? ordered[index - 1])?.id };
      }
      void clipboard.update(record.id, 'delete');
      return;
    }
    if (action === 'pin') {
      const origin = source ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
      if (origin?.closest('article')) refocus.current = { id: record.id, previous: origin };
    }
    setSelected(record.id);
    void clipboard.update(record.id, action);
  }

  const canPreview = snapshot.window.clipboardDetailVisible || (clipboard.displayReady && ordered.length > 0);
  useRecordsPreviewShortcut({
    active,
    blocked: Boolean(editing || menu),
    canTrigger: canPreview,
    excludedContainers: '.translation-panel, .clipboard-right-panes, .preview-bar',
    snapshot,
    native,
    onPreview: (rowId) => {
      const targetId = rowId ?? keyboardId;
      const record = clipboard.displayReady ? (ordered.find(({ id }) => id === targetId) ?? first) : undefined;
      if (record) void command('records_details_toggle', { recordId: record.id });
      else if (snapshot.window.clipboardDetailVisible) void command('records_details_toggle');
    },
  });

  function recordKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    const key = event.nativeEvent;
    if (!active || menu || editing || event.defaultPrevented || key.isComposing || event.keyCode === 229 || key.key === 'Dead') return;
    const input = search.current;
    const fromSearch = event.target === input;
    const target = event.target;
    const fromRecord = target instanceof HTMLButtonElement && target.classList.contains('history-row-select');
    const control = target instanceof HTMLButtonElement ? target.closest<HTMLElement>('.history-toolbar, .history-footer, .history-row-actions') : null;
    if (!fromSearch && !fromRecord && !control) return;
    const slot = /^Digit([1-5])$/.exec(key.code);
    if (quickPaste && slot && isQuickPasteModifier(key)) {
      event.preventDefault();
      const record = quickPasteCandidates[Number(slot[1]) - 1];
      if (!key.repeat && clipboard.displayReady && !clipboard.error && record && !(target as HTMLElement).closest('[role=dialog]')) void useRecord(record.id, 'paste');
      return;
    }
    const id = fromRecord || control ? (target as HTMLElement).closest<HTMLElement>('[data-record-id]')?.dataset.recordId ?? keyboardId : keyboardId;
    const rowButton = id ? rowRefs.current.get(id) : undefined;
    const action = clipboardKeyAction(shortcuts, key);
    const plain = !key.altKey && !key.ctrlKey && !key.metaKey && !key.shiftKey;
    const categoryDirection = clipboardCategoryDirection(shortcuts, key);
    if ((fromRecord || fromSearch || control?.matches('.history-toolbar, .history-footer')) && categoryDirection) {
      event.preventDefault();
      if (key.repeat || control?.closest('.history-footer')?.querySelector('[role=dialog]')) return;
      const categories = [null, 'favorites', 'text', 'image', 'files'] as const;
      const index = categories.indexOf(favoritesOnly ? 'favorites' : kindFilter);
      const next = categories[(index + (categoryDirection > 0 ? 1 : categories.length - 1)) % categories.length];
      remember(); setFavoritesOnly(next === 'favorites'); setKindFilter(next === 'favorites' ? null : next);
      search.current?.focus({ preventScroll: true });
      return;
    }
    const activate = (fromSearch || fromRecord) && !action && plain && key.key === 'Enter';
    const alternateActivate = (fromSearch || fromRecord) && !action && key.altKey && !key.ctrlKey && !key.metaKey && !key.shiftKey && key.key === 'Enter';
    // The row button must not paste on Space or activate a stale result on Return.
    if (fromRecord && ((key.key === ' ' && !(action === 'preview' && canPreview)) || activate || alternateActivate)) event.preventDefault();
    const index = ordered.findIndex((record) => record.id === id);
    const record = ordered[index];
    if (!clipboard.displayReady || !record) return;
    if ((fromSearch || fromRecord) && plain && !action && (key.key === 'ArrowDown' || key.key === 'ArrowUp')) {
      event.preventDefault();
      if (fromSearch) {
        if (key.key === 'ArrowDown') focusRecordList(input?.closest('.history-panel'));
        return;
      }
      const next = ordered[index + (key.key === 'ArrowDown' ? 1 : -1)];
      if (next) {
        focusRow(next.id);
        previewRecord(next.id, true);
      } else if (!fromSearch && key.key === 'ArrowUp') input?.focus();
      return;
    }
    if (activate || alternateActivate) {
      event.preventDefault();
      if (!key.repeat) { previewRecord(record.id, true); void useRecord(record.id, alternateActivate ? 'activateAlternate' : 'activate'); }
      return;
    }
    if (!action || action === 'preview' || (action === 'copyRecord' && document.getSelection()?.isCollapsed === false)) return;
    if (fromSearch && key.key === 'Backspace' && input?.value && action !== 'delete') return;
    event.preventDefault();
    if (key.repeat || !usableShortcut(action)) return;
    if (action === 'openTarget') { if (record.kind !== 'image' || record.hasFilePaths) void openRecordTarget(record.id); }
    else if (action === 'pastePlain') { if (record.kind === 'text') { previewRecord(record.id, true); void useRecord(record.id, 'pastePlain'); } }
    else if (action === 'saveImage') { if (record.kind === 'image') void saveImage(record.id); }
    else if (action === 'note') setEditing({ id: record.id });
    else if (action === 'copyRecord') rowButton?.closest('article')?.querySelector<HTMLButtonElement>('[data-action=copy] button')?.click();
    else if (action === 'translate' || action === 'ocr') {
      if (action === 'translate' ? canTranslateRecord(record) : canOcrRecord(record)) void processRecord(record.id, action);
    } else if (action === 'delete') {
      updateRecord(record, 'delete', target instanceof HTMLButtonElement ? target : null);
    } else {
      updateRecord(record, action, !fromSearch ? rowButton : null);
    }
  }
  const translateFromLaunchKey = useEffectEvent(() => {
    if (!active || !document.hasFocus() || editing || composing.current || !clipboard.displayReady || clipboard.error) return;
    const shortcut = shortcuts.find(({ action }) => action === 'translate');
    if (!shortcut?.enabled || shortcut.globalConflict || !shortcut.binding || !isClipboardTranslateLaunchKey(shortcut.binding) || !usableShortcut('translate')) return;
    const record = menu ? menuRecord : ordered.find(({ id }) => id === keyboardId);
    if (!record || !canTranslateRecord(record) || processing !== null) return;
    setMenu(null);
    void processRecord(record.id, 'translate');
  });
  useEffect(() => {
    if (!native || !active) return;
    let disposed = false;
    const subscription = listen('clipboard_translate_requested', () => { if (!disposed) translateFromLaunchKey(); });
    const resetComposition = () => { composing.current = false; };
    window.addEventListener('blur', resetComposition);
    return () => {
      disposed = true;
      resetComposition();
      window.removeEventListener('blur', resetComposition);
      void subscription.then((remove) => remove());
    };
  }, [native, active]);

  function loadNearEnd(element: HTMLDivElement) {
    if (element.clientHeight > 0 && element.scrollHeight - element.scrollTop - element.clientHeight < 180) void clipboard.loadMore();
  }
  function row(record: ClipboardRecord) {
    const canTranslate = canTranslateRecord(record);
    const canOcr = canOcrRecord(record);
    const media = record.kind !== 'text' && <ClipboardMedia record={record} native={native} active={active} onImageFile={onImageFile} />;
    const kind = record.kind === 'image'
      ? t('clipboardImage')
      : record.kind === 'files'
        ? t('clipboardFiles')
        : record.hasRichText
          ? t('clipboardRichText')
          : t('clipboardPlainText');
    const keyboardCurrent = keyboardId === record.id;
    const slot = quickPaste && clipboard.displayReady && !clipboard.error ? quickPasteCandidates.findIndex(({ id }) => id === record.id) : -1;
    const description = [
      record.kind !== 'text' && record.text ? kind : null,
      record.title ? `Note: ${record.title}` : null,
      record.isPinned ? 'Pinned' : null,
      record.isFavorite ? 'Favorite' : null,
      record.imageTextMatch ? t('clipboardImageTextMatch') : null,
    ].filter(Boolean).join(', ') || undefined;
    const translating = snapshot.window.clipboardTranslationVisible && snapshot.window.clipboardTranslationRecordId === record.id;
    return <article data-record-target={active && clipboard.displayReady && visibleIds.has(record.id) && (menu?.id ?? selected) === record.id || undefined} data-translating={translating || undefined} data-keyboard-current={keyboardCurrent || undefined} onFocus={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) { setSelected(record.id); previewRecord(record.id); } }} data-note-revealed={record.title ? revealed === record.id : undefined} onMouseEnter={() => { if (record.title) setRevealed(record.id); }} onMouseMove={(event) => {
      if (!active || editing || menu || !clipboard.displayReady || !visibleIds.has(record.id)) return;
      targetOnMouseMove(event, () => { setSelected(record.id); previewRecord(record.id); });
    }} onMouseLeave={() => setRevealed((current) => current === record.id ? null : current)} onContextMenu={(event) => { event.preventDefault(); setMenu({ id: record.id, x: event.clientX, y: event.clientY }); }} key={record.id} hidden={!visibleIds.has(record.id)} data-record-id={record.id} className={cn('history-row', selection === record.id && 'selected', menu?.id === record.id && 'context-target', record.isPinned && 'pinned')} style={{ '--history-actions-width': `${124 + 25 * (Number(canTranslate) + Number(canOcr))}px` } as CSSProperties}>
      <button type="button" ref={(element) => { if (element) rowRefs.current.set(record.id, element); else rowRefs.current.delete(record.id); }} className="history-row-select" draggable={native && snapshot.capabilities.clipboardDrag && !editing && !menu} onDragStart={(event) => { if (!native || !snapshot.capabilities.clipboardDrag || editing || menu) return; event.preventDefault(); void dragRecord(record.id); }} aria-label={`${t('clipboard')}: ${record.text || kind}`} aria-description={description} aria-keyshortcuts={slot >= 0 ? `Meta+${slot + 1}` : undefined} aria-current={keyboardCurrent || undefined} aria-pressed={selection === record.id} onClick={() => { previewRecord(record.id, true); void useRecord(record.id, 'activate'); }}>
        <RecordHeading date={record.date} label={kind} sourceApp={record.sourceApp} quickPasteHint={active && showQuickPasteHints && !menu && !editing && slot >= 0 ? slot + 1 : undefined} />
        {record.title ? <ClipboardNotePreview record={record} revealed={revealed === record.id} native={native} media={media} /> : media ? media : <ClipboardTextPreview text={historyPreview(record.text)} className="history-preview history-text-preview" />}
        {record.imageTextMatch && <span className="clipboard-search-match">{t('clipboardImageTextMatch')}</span>}
      </button>
      <RecordActions allowMarkedDelete recordId={record.id} t={t} native={native} favorite={record.isFavorite} pinned={record.isPinned} deleteLabel={t('deleteClipboard')}
        canCopy={snapshot.capabilities.clipboardWrite} onCopy={() => useRecord(record.id, 'copy')} onNote={() => setEditing({ id: record.id })} busy={processing !== null} translateActive={translating} onTranslate={canTranslate ? () => { void processRecord(record.id, 'translate'); } : undefined} onOcr={canOcr ? () => { void processRecord(record.id, 'ocr'); } : undefined}
        onUpdate={(action) => { updateRecord(record, action); }} />
    </article>;
  }
  return <section data-focus-ready={clipboard.displayReady || undefined} inert={editing !== null} onKeyDown={recordKeyDown} onCompositionStartCapture={() => { composing.current = true; }} onCompositionEndCapture={() => { composing.current = false; }} className="history-panel clipboard-panel" style={{ '--history-preview-lines': snapshot.clipboardControls.previewLines?.clipboard } as CSSProperties} aria-label={t('clipboard')}>
    <ActivityBeam label={t('working')} active={active && clipboard.showActivity} />
    <div className="drag-strip" data-tauri-drag-region />
    <RecordToolbar t={t} snapshot={snapshot} command={command} query={query} onQuery={setQuery} search={search} onDetailsToggle={() => { void toggleDetails(); }} />
    {clipboard.error && <p role="alert" className="history-error">{t(clipboard.searchError === 'invalidRegex' ? 'clipboardSearchInvalidRegex' : clipboard.searchError === 'tooBroad' ? 'clipboardSearchTooBroad' : 'nativeError')}
      {!clipboard.searchError && <> <button onClick={() => void clipboard.refresh()}>{t('retry')}</button></>}
    </p>}
    <HistoryList
      onScrollOwnerChange={restoreScroll}
      active={active}
      onScroll={() => { remember(); clipboard.refreshDeferred(); }}
      list={list}
      pinnedList={pinnedList}
      handleRef={listHandle}
      pinnedCount={records.filter((record) => record.isPinned).length}
      pinned={clipboard.retainedRecords.filter((record) => record.isPinned).map(row)}
      virtualItems={{
        items: ordinary,
        allItems: allUnpinned,
        itemKey: clipboardRecordKey,
        renderItem: row,
        resetKey: snapshot.clipboardControls.previewLines?.clipboard,
      }}
      onNearEnd={loadNearEnd}
    >
      {!clipboard.loading && !clipboard.error && !records.length && <div className="history-empty clipboard-empty" role="status">
        <p>{query ? t('clipboardNoMatches') : favoritesOnly ? t('clipboardFavoritesEmpty') : kindFilter ? t(kindFilter === 'text' ? 'clipboardTextEmpty' : kindFilter === 'image' ? 'clipboardImagesEmpty' : 'clipboardFilesEmpty') : !snapshot.capabilities.clipboardWrite ? t('platformUnavailable') : snapshot.clipboardCaptureEnabled ? t('clipboardEmpty') : t('clipboardRecordingPaused')}</p>
        {kindFilter && <button type="button" onClick={() => { remember(); setKindFilter(null); }}>{t('clearClipboardFilter')}</button>}
        {!query && !favoritesOnly && !kindFilter && native && !snapshot.storageError && snapshot.capabilities.clipboardWrite && !snapshot.clipboardCaptureEnabled && <button type="button" disabled={resumingCapture} onClick={() => {
          if (resumingCapture) return;
          setResumingCapture(true);
          setFeedback(null);
          void invoke('clipboard_capture_set', { enabled: true }).catch(() => { if (mounted.current) setFeedback('clipboardSettingsSaveFailed'); }).finally(() => { if (mounted.current) setResumingCapture(false); });
        }}>{t('clipboardResumeRecording')}</button>}
      </div>}
    </HistoryList>
    <HistoryFooter key={session} t={t} snapshot={snapshot} command={command} native={native} active={active && !menu && !editing} search={search} favoritesOnly={favoritesOnly} onFavorites={() => { remember(); setKindFilter(null); setFavoritesOnly(kindFilter !== null || !favoritesOnly); }} kind={kindFilter} onKindChange={(next) => { remember(); setFavoritesOnly(false); setKindFilter(next); }} total={clipboard.page.totalRecords} clipboard activation={snapshot.clipboardControls.activation} />
    {menu && menuRecord && <ClipboardMenu key={menuRecord.id} record={menuRecord} snapshot={snapshot} x={menu.x} y={menu.y} canPaste={native && snapshot.capabilities.autoPaste} canTranslate={canTranslateRecord(menuRecord)} canOcr={canOcrRecord(menuRecord)} canCopy={native && snapshot.capabilities.clipboardWrite} canSave={native && snapshot.capabilities.clipboardImageSave && !savingImage} canOpenTarget={usableShortcut('openTarget')} t={t} busy={processing !== null} onClose={() => setMenu(null)} onReturnFocus={() => focusRow(menuRecord.id)} onAction={(action) => {
      setMenu(null);
      if (action === 'paste' || action === 'pastePlain' || action === 'pastePaths' || action === 'copy' || action === 'copyPlain') void useRecord(menuRecord.id, action);
      else if (action === 'save') void saveImage(menuRecord.id);
      else if (action === 'translate' || action === 'ocr') void processRecord(menuRecord.id, action);
      else if (action === 'openTarget') void openRecordTarget(menuRecord.id);
      else if (action === 'note') setEditing({ id: menuRecord.id });
      else updateRecord(menuRecord, action, rowRefs.current.get(menuRecord.id));
    }} />}
    {editing && <ClipboardEditor recordId={editing.id} onClose={() => setEditing(null)} onSaved={(detail) => clipboard.applyEdit(editing.id, detail)} onReturnFocus={() => { editorReturn.current = editing.id; }} />}
    <RecordFeedback message={active ? processingError ?? (feedback ? t(feedback) : null) : null} />
  </section>;
}
