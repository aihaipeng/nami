import { clipboardOpenError, clipboardOpenTarget } from './openTarget';
import { focusRecordList, useRecordPointer, useRecordsOpenFocus } from '../shared/recordFocus';
import { useCallback, useEffect, useEffectEvent, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import type { ClipboardResult } from '../shared/contracts';
import type { MessageKey } from '../shared/i18n';
import { ActivityBeam } from '../shared/ActivityBeam';
import { RecordToolbar } from '../shared/RecordToolbar';
import { VIRTUALIZE_THRESHOLD } from '../shared/virtualList';
import { useRecordBrowser } from '../shared/useRecordBrowser';
import { historyPreview, historyScrollTarget } from '../shared/recordPreview';
import { cn } from '../lib/utils';
import { RecordHeading } from '../shared/RecordHeading';
import { RecordActions } from '../shared/RecordActions';
import { RecordFeedback } from '../shared/RecordFeedback';
import { useClipboard, type ClipboardKind, type ClipboardRecord } from './useClipboard';
import { HistoryList, type HistoryListHandle } from '../shared/HistoryList';
import { clipboardCategoryArrowDirection, clipboardGlobalBindings, clipboardKeyAction, clipboardPanelShortcuts, clipboardShortcutAvailable, DEFAULT_HOVER_PREVIEW_DELAY_MS, isClipboardTranslateLaunchKey, isNonNamiClipboardBrowserKey } from '../shared/clipboardShortcuts';
import { translationShortcutEnabled } from '../shared/contracts';
import { commandKeyLabel, editsText, matchesBinding, sameBinding, shortcutActions, type KeyBinding } from '../shared/shortcuts';
import { HistoryFooter } from '../shared/HistoryFooter';
import { ClipboardMedia } from './ClipboardMedia';
import { ClipboardNotePreview } from './ClipboardNotePreview';
import { ClipboardTextPreview } from './ClipboardTextPreview';
import { ClipboardDeleteConfirm, markedDeleteDescription } from './ClipboardDeleteConfirm';
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
  const [commandHeld, setCommandHeld] = useState(false);
  const [kindFilter, setKindFilter] = useState<ClipboardKind | null>(null);
  const [processing, setProcessing] = useState<string | null>(null);
  const [processingError, setProcessingError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<string | null>(null);
  const [pointerOutside, setPointerOutside] = useState(false);
  const [editing, setEditing] = useState<{ id: string } | null>(null);
  const [resumingCapture, setResumingCapture] = useState(false);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<MessageKey | null>(null);
  const [pendingDelete, setPendingDelete] = useState<{ record: ClipboardRecord; source: HTMLElement | null } | null>(null);
  const [textless, setTextless] = useState<ReadonlySet<string>>(() => new Set());
  const operation = useRef(0);
  const processingRevision = useRef(0);
  const reveal = useRef<string | null>(null);
  const revealContext = useRef({ browsing: true });
  const mounted = useRef(true);
  const refocus = useRef<{ id: string; previous: HTMLElement; neighbor?: string } | null>(null);
  const editorReturn = useRef<string | null>(null);
  const rowRefs = useRef(new Map<string, HTMLButtonElement>());
  const search = useRef<HTMLInputElement>(null);
  const panel = useRef<HTMLElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const listHandle = useRef<HistoryListHandle>(null);
  const pendingFocus = useRef<string | null>(null);
  const pendingCategoryFocus = useRef<{ revealActions: boolean; origin: Element | null } | null>(null);
  const hoverPreviewTimer = useRef<number | null>(null);
  const hoverPreviewTarget = useRef<string | null>(null);
  const hoverPreviewOpening = useRef(false);
  const hoveredRow = useRef<string | null>(null);
  const suppressedHoverRow = useRef<string | null>(null);
  const wasDetailVisible = useRef(snapshot.window.clipboardDetailVisible);
  const hoverPreviewEnabled = Boolean(snapshot.clipboardControls.hoverPreviewEnabled);
  const hoverPreviewDelayMs = snapshot.clipboardControls.hoverPreviewDelayMs ?? DEFAULT_HOVER_PREVIEW_DELAY_MS;
  const cancelHoverPreview = useCallback(() => {
    if (hoverPreviewTimer.current !== null) {
      window.clearTimeout(hoverPreviewTimer.current);
      hoverPreviewTimer.current = null;
    }
    hoverPreviewTarget.current = null;
  }, []);
  const browser = useRecordBrowser(snapshot.window.recordsSession, active, list, () => clipboard.displayReady, () => clipboard.page.records, kindFilter);
  const { query, setQuery, favoritesOnly, setFavoritesOnly, remember, restoreScroll } = browser;
  const searchOptions = snapshot.clipboardControls.search;
  const searchKey = searchOptions && (searchOptions.mode !== 'exact' || !searchOptions.body || !searchOptions.note || !searchOptions.imageText)
    ? `${searchOptions.mode}:${+searchOptions.body}${+searchOptions.note}${+searchOptions.imageText}` : '';
  const clipboard = useClipboard(native, query, favoritesOnly, () => (list.current?.scrollTop ?? 0) > 0, kindFilter, searchKey, active);
  const records = clipboard.page.records;
  const menuRecord = menu ? records.find((record) => record.id === menu.id) : null;
  const textlessImage = (record: ClipboardRecord) => record.kind === 'image' && (record.imageTextEmpty || textless.has(record.id));
  const canTranslateRecord = (record: ClipboardRecord) => record.kind === 'text' ? !!record.text.trim() : record.kind === 'image' && snapshot.capabilities.imageOcr && !textlessImage(record);
  const canOcrRecord = (record: ClipboardRecord) => record.kind === 'image' && canTranslateRecord(record);
  const selection = snapshot.window.clipboardDetailVisible ? snapshot.window.clipboardDetailId : selected;
  const visibleIds = new Set(records.map(({ id }) => id));
  const ordinary = useMemo(() => records.filter((record) => !record.isPinned), [records]);
  const allUnpinned = useMemo(() => clipboard.retainedRecords.filter((record) => !record.isPinned), [clipboard.retainedRecords]);
  const pinned = records.filter((record) => record.isPinned);
  const retainedPins = clipboard.retainedRecords.filter((record) => record.isPinned);
  const ordered = [...pinned, ...ordinary];
  const quickPasteCandidates = ordinary.slice(0, 5);
  const quickPaste = native && snapshot.capabilities.platform === 'macos' && snapshot.capabilities.autoPaste && !snapshot.storageError;
  const showQuickPasteHints = commandHeld && quickPaste;
  const commandHints = commandHeld && !menu && !editing && !pendingDelete;
  const commandLetter = (binding: KeyBinding | null) => {
    if (!binding) return;
    const letter = commandKeyLabel(binding, snapshot.capabilities.platform === 'macos', t);
    if (!letter) return;
    const blocked = clipboardPanelShortcuts(snapshot).some((local) => local.enabled && !local.globalConflict && local.binding && clipboardShortcutAvailable(snapshot, native, local.action) && sameBinding(local.binding, binding));
    return blocked ? undefined : letter;
  };
  const windowShortcut = (action: 'pin' | 'focusSearch' | 'showShortcuts') => {
    const binding = snapshot.translationPreferences.shortcuts[action];
    return commandHints && translationShortcutEnabled(snapshot.translationPreferences, action) ? commandLetter(binding) : undefined;
  };
  const first = records.find((record) => !record.isPinned) ?? records[0];
  const targetOnMouseMove = useRecordPointer();
  const keyboardId = clipboard.displayReady ? (selected && visibleIds.has(selected) ? selected : first?.id) : undefined;
  const hoverPreviewContext = useRef({
    active,
    clipboardVisible: snapshot.window.clipboardVisible,
    detailVisible: snapshot.window.clipboardDetailVisible,
    enabled: hoverPreviewEnabled,
    delayMs: hoverPreviewDelayMs,
    blocked: Boolean(editing || menu || pendingDelete),
    displayReady: clipboard.displayReady,
    visibleIds,
  });
  hoverPreviewContext.current = {
    active,
    clipboardVisible: snapshot.window.clipboardVisible,
    detailVisible: snapshot.window.clipboardDetailVisible,
    enabled: hoverPreviewEnabled,
    delayMs: hoverPreviewDelayMs,
    blocked: Boolean(editing || menu || pendingDelete),
    displayReady: clipboard.displayReady,
    visibleIds,
  };
  const openHoveredDetail = useEffectEvent((recordId: string) => {
    hoverPreviewTimer.current = null;
    hoverPreviewTarget.current = null;
    const ctx = hoverPreviewContext.current;
    if (!ctx.active || !ctx.clipboardVisible || !ctx.enabled || ctx.detailVisible || hoverPreviewOpening.current || ctx.blocked || !ctx.displayReady || !ctx.visibleIds.has(recordId) || hoveredRow.current !== recordId || suppressedHoverRow.current === recordId) return;
    hoverPreviewOpening.current = true;
    void command('records_details_toggle', { recordId }).finally(() => { hoverPreviewOpening.current = false; });
  });
  function scheduleHoverPreview(recordId: string) {
    const ctx = hoverPreviewContext.current;
    if (!ctx.active || !ctx.clipboardVisible || !ctx.enabled || ctx.detailVisible || hoverPreviewOpening.current || ctx.blocked || !ctx.displayReady || !ctx.visibleIds.has(recordId) || suppressedHoverRow.current === recordId) {
      cancelHoverPreview();
      return;
    }
    if (hoverPreviewTarget.current === recordId && hoverPreviewTimer.current !== null) return;
    cancelHoverPreview();
    hoverPreviewTarget.current = recordId;
    if (ctx.delayMs <= 0) {
      openHoveredDetail(recordId);
      return;
    }
    hoverPreviewTimer.current = window.setTimeout(() => openHoveredDetail(recordId), ctx.delayMs);
  }
  const clipboardShown = useRef(false);
  useEffect(() => cancelHoverPreview, [cancelHoverPreview]);
  useEffect(() => {
    const visible = snapshot.window.clipboardDetailVisible;
    if (visible) {
      hoverPreviewOpening.current = false;
      cancelHoverPreview();
    } else if (wasDetailVisible.current && active && snapshot.window.clipboardVisible) {
      suppressedHoverRow.current = hoveredRow.current;
      cancelHoverPreview();
    }
    wasDetailVisible.current = visible;
  }, [snapshot.window.clipboardDetailVisible, active, snapshot.window.clipboardVisible, cancelHoverPreview]);
  useEffect(() => {
    const visible = active && snapshot.window.clipboardVisible;
    if (visible && !clipboardShown.current) setPointerOutside(false);
    if (!visible) {
      cancelHoverPreview();
      hoveredRow.current = null;
      suppressedHoverRow.current = null;
      hoverPreviewOpening.current = false;
    }
    clipboardShown.current = visible;
  }, [active, snapshot.window.clipboardVisible, cancelHoverPreview]);
  useEffect(() => {
    if (!active) return;
    const markOutside = () => {
      cancelHoverPreview();
      hoveredRow.current = null;
      suppressedHoverRow.current = null;
      setPointerOutside(true);
    };
    window.addEventListener('blur', markOutside);
    return () => window.removeEventListener('blur', markOutside);
  }, [active, cancelHoverPreview]);
  useEffect(() => { cancelHoverPreview(); }, [hoverPreviewEnabled, hoverPreviewDelayMs, snapshot.window.recordsSession, cancelHoverPreview]);
  useEffect(() => {
    if (editing || menu || pendingDelete) cancelHoverPreview();
  }, [editing, menu, pendingDelete, cancelHoverPreview]);
  useEffect(() => {
    if (!active) { setCommandHeld(false); return; }
    const update = (event: KeyboardEvent) => setCommandHeld(isQuickPasteModifier(event));
    const reset = () => setCommandHeld(false);
    window.addEventListener('keydown', update, true);
    window.addEventListener('keyup', update, true);
    window.addEventListener('blur', reset);
    return () => {
      window.removeEventListener('keydown', update, true);
      window.removeEventListener('keyup', update, true);
      window.removeEventListener('blur', reset);
    };
  }, [active]);
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
  useEffect(() => { cancelHoverPreview(); setMenu(null); setRevealed(null); }, [query, favoritesOnly, kindFilter, cancelHoverPreview]);
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
    pendingCategoryFocus.current = null;
    setKindFilter(null); setProcessing(null); setProcessingError(null);
    setEditing(null); setMenu(null); setRevealed(null); setFeedback(null); setSession((value) => value + 1);
  }, [snapshot.window.recordsSession]);
  useEffect(() => { if (!active) { ++operation.current; ++processingRevision.current; pendingCategoryFocus.current = null; setEditing(null); setMenu(null); setRevealed(null); setFeedback(null); setProcessing(null); setProcessingError(null); setSession((value) => value + 1); } }, [active]);
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

  async function useRecord(recordId: string, action: 'paste' | 'pastePlain' | 'pastePaths' | 'activate') {
    if (!native || !snapshot.capabilities.autoPaste) return;
    const current = ++operation.current;
    setSelected(recordId); setFeedback(null);
    try {
      const result = await invoke<ClipboardResult>('clipboard_record_use', { recordId, action });
      if (!mounted.current) return;
      if (result.copyStatus === 'current') clipboard.noteCopied(recordId);
      if (operation.current === current) setFeedback(clipboardUseFeedback(result));
    } catch (error) {
      if (mounted.current && operation.current === current) setFeedback(clipboardUseError(error));
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
        const message = clipboardProcessError(error, action);
        if (message) setProcessingError(message);
        else setTextless((current) => {
          const record = records.find(({ id }) => id === recordId);
          if (record?.kind !== 'image' || current.has(recordId)) return current;
          const next = new Set(current);
          next.add(recordId);
          return next;
        });
      }
    } finally {
      if (mounted.current && processingRevision.current === revision) setProcessing(null);
    }
  }

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (!active || event.defaultPrevented || composing.current || event.isComposing || event.keyCode === 229 || (editing && event.key === 'Escape')) return;
      if (matchesBinding({ code: 'Escape', modifiers: [] }, event)) {
        event.preventDefault();
        if (event.repeat) return;
        ++operation.current;
        if (snapshot.window.clipboardTranslationVisible) void command('clipboard_translation_close');
        else void command('window_hide', { region: 'clipboard' });
      }
    };
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [snapshot.window.clipboardTranslationVisible, command, editing, active]);
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
    const pending = pendingCategoryFocus.current;
    if (!pending) return;
    if (!active || editing || menu || pendingDelete || (document.activeElement !== pending.origin && document.activeElement !== document.body)) {
      pendingCategoryFocus.current = null; return;
    }
    if (!clipboard.displayReady || clipboard.loading) return;
    pendingCategoryFocus.current = null;
    if (!first) {
      setSelected(null);
      if (document.activeElement instanceof HTMLElement && document.activeElement.closest('.clipboard-panel .history-row')) {
        document.activeElement.blur();
      }
      return;
    }
    const button = rowRefs.current.get(first.id)
      ?? list.current?.querySelector<HTMLButtonElement>('.history-row:not([hidden]) .history-row-select');
    if (button) {
      const rowId = button.closest<HTMLElement>('[data-record-id]')?.dataset.recordId ?? first.id;
      if (pending.revealActions) setPointerOutside(false);
      setSelected(rowId);
      pendingFocus.current = null;
      button.focus({ preventScroll: true });
    }
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

  function applyCategoryFilter(nextFavorites: boolean, nextKind: ClipboardKind | null, revealActions: boolean) {
    if (nextFavorites === favoritesOnly && nextKind === kindFilter) return;
    remember();
    setFavoritesOnly(nextFavorites);
    setKindFilter(nextKind);
    pendingCategoryFocus.current = document.activeElement === search.current ? null : { revealActions, origin: document.activeElement };
  }

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

  function commitDelete(record: ClipboardRecord, origin: HTMLElement | null) {
    if (processing) return;
    setFeedback(null);
    const index = ordered.findIndex(({ id }) => id === record.id);
    if (origin?.closest('article')) {
      refocus.current = { id: record.id, previous: origin, neighbor: (ordered[index + 1] ?? ordered[index - 1])?.id };
    }
    void clipboard.update(record.id, 'delete');
  }
  function updateRecord(record: ClipboardRecord, action: 'favorite' | 'pin' | 'delete', source?: HTMLElement | null) {
    if (action === 'delete') {
      const origin = source ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
      if (record.isFavorite || record.isPinned) {
        setPendingDelete({ record, source: origin });
        return;
      }
      commitDelete(record, origin);
      return;
    }
    if (action === 'pin') {
      const origin = source ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
      if (origin?.closest('article')) refocus.current = { id: record.id, previous: origin };
    }
    setSelected(record.id);
    void clipboard.update(record.id, action);
  }

  const handleClipboardKeyDown = useEffectEvent((key: KeyboardEvent) => {
    if (!active || menu || editing || pendingDelete || key.defaultPrevented || composing.current || key.isComposing || key.keyCode === 229 || key.key === 'Dead') return;
    if (document.querySelector('.clipboard-editor-layer, .clipboard-context-menu, .history-shortcuts-panel, .clipboard-navigation-confirm, .clipboard-delete-confirm')) return;
    const target = key.target;
    const fromPage = target === document.body || target === document.documentElement || target === document || target === window;
    if (!fromPage && !(target instanceof Node && panel.current?.contains(target))) return;
    const input = search.current;
    const fromSearch = target === input;
    if (!fromSearch && target instanceof HTMLElement && target.closest('input, textarea, [contenteditable=true]')) return;
    const fromRecord = target instanceof HTMLButtonElement && target.classList.contains('history-row-select');
    const inRow = target instanceof Element ? target.closest<HTMLElement>('[data-record-id]') : null;
    const plain = !key.altKey && !key.ctrlKey && !key.metaKey && !key.shiftKey;
    const enter = key.key === 'Enter';
    const space = key.key === ' ' || key.code === 'Space';
    if (!fromRecord && target instanceof HTMLButtonElement && plain && (enter || space && !inRow)) {
      if (key.repeat) key.preventDefault();
      return;
    }
    const action = clipboardKeyAction(shortcuts, key);
    const globalBinding = clipboardGlobalBindings(snapshot).some((binding) => matchesBinding(binding, key));
    if (!action && globalBinding) return;

    const slot = /^Digit([1-5])$/.exec(key.code);
    if (quickPaste && slot && isQuickPasteModifier(key)) {
      key.preventDefault();
      const record = quickPasteCandidates[Number(slot[1]) - 1];
      if (!key.repeat && clipboard.displayReady && !clipboard.error && record) void useRecord(record.id, 'paste');
      return;
    }
    const letter = /^Key([BEGHIJLMQRUY])$/.exec(key.code)?.[1];
    const pinPaste = letter && (clipboard.page.pinPasteKeys ?? []).find((item) => item.key === letter);
    const letterTaken = editsText(key)
      || Boolean(action)
      || globalBinding
      || shortcutActions.some((action) => translationShortcutEnabled(snapshot.translationPreferences, action) && matchesBinding(snapshot.translationPreferences.shortcuts[action], key));
    if (quickPaste && pinPaste && isQuickPasteModifier(key) && !letterTaken) {
      key.preventDefault();
      if (!key.repeat && clipboard.displayReady && !clipboard.error) void useRecord(pinPaste.id, 'paste');
      return;
    }

    const categoryDirection = clipboardCategoryArrowDirection(shortcuts, snapshot, key, input);
    if (categoryDirection) {
      key.preventDefault();
      if (key.repeat) return;
      const categories = [null, 'favorites', 'text', 'image', 'files'] as const;
      const categoryIndex = categories.indexOf(favoritesOnly ? 'favorites' : kindFilter);
      const next = categories[(categoryIndex + (categoryDirection > 0 ? 1 : categories.length - 1)) % categories.length];
      applyCategoryFilter(next === 'favorites', next === 'favorites' ? null : next, true);
      return;
    }

    const id = inRow?.dataset.recordId ?? keyboardId;
    const rowButton = id ? rowRefs.current.get(id) : undefined;
    const index = ordered.findIndex((record) => record.id === id);
    const record = clipboard.displayReady ? ordered[index] : undefined;

    if (!action && fromRecord && record && plain && key.key === 'ContextMenu') {
      key.preventDefault();
      if (!key.repeat) {
        const bounds = target.getBoundingClientRect();
        setMenu({ id: record.id, x: bounds.left, y: bounds.bottom });
      }
      return;
    }

    if (plain && !action && (key.key === 'ArrowDown' || key.key === 'ArrowUp')) {
      key.preventDefault();
      if (!record) return;
      cancelHoverPreview();
      if (fromSearch) {
        if (key.key === 'ArrowDown') {
          setPointerOutside(false);
          focusRecordList(input?.closest('.history-panel'));
        }
        return;
      }
      const next = ordered[index + (key.key === 'ArrowDown' ? 1 : -1)];
      if (next) {
        setPointerOutside(false);
        focusRow(next.id);
        if (snapshot.window.recordsBrowsing === false) previewRecord(next.id, true);
      } else if (key.key === 'ArrowUp') {
        input?.focus();
      } else if (!fromRecord) {
        setPointerOutside(false);
        focusRow(record.id);
      }
      return;
    }

    if (action === 'preview' && (record || snapshot.window.clipboardDetailVisible)) {
      key.preventDefault();
      if (!key.repeat) void command('records_details_toggle', record ? { recordId: record.id } : undefined);
      return;
    }

    if (!action && enter) {
      if (plain || target instanceof HTMLButtonElement) key.preventDefault();
      if (record && !key.repeat && plain) {
        previewRecord(record.id, true);
        void useRecord(record.id, 'activate');
      }
      return;
    }

    if (action && action !== 'preview'
      && !(fromSearch && key.key === 'Backspace' && input?.value && action !== 'delete')) {
      if (!record) {
        if (!fromSearch) key.preventDefault();
        return;
      }
      key.preventDefault();
      if (key.repeat || !usableShortcut(action)) return;
      if (action === 'openTarget') { if (record.kind !== 'image' || record.hasFilePaths) void openRecordTarget(record.id); }
      else if (action === 'pastePlain') { if (record.kind === 'text') { previewRecord(record.id, true); void useRecord(record.id, 'pastePlain'); } }
      else if (action === 'note') setEditing({ id: record.id });
      else if (action === 'translate' || action === 'ocr') {
        if (action === 'translate' ? canTranslateRecord(record) : canOcrRecord(record)) void processRecord(record.id, action);
      } else if (action === 'delete') {
        updateRecord(record, 'delete', target instanceof HTMLButtonElement ? target : rowButton ?? null);
      } else {
        updateRecord(record, action, !fromSearch ? rowButton : null);
      }
      return;
    }

    if ((space && target instanceof HTMLButtonElement) || isNonNamiClipboardBrowserKey(key, snapshot.capabilities.primaryModifier, fromSearch)) {
      key.preventDefault();
    }
  });
  useEffect(() => {
    if (!active) return;
    const keydown = (event: KeyboardEvent) => handleClipboardKeyDown(event);
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [active]);
  const translateFromLaunchKey = useEffectEvent(() => {
    if (!active || !document.hasFocus() || editing || pendingDelete || composing.current || !clipboard.displayReady || clipboard.error) return;
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

  function pointerEnteredRow(event: ReactPointerEvent<HTMLElement>) {
    const focused = document.activeElement?.closest('.clipboard-panel .history-row');
    if (!focused || focused === event.currentTarget) setPointerOutside(false);
  }
  function pointerLeftRow(event: ReactPointerEvent<HTMLElement>, recordId: string) {
    const next = event.relatedTarget;
    if (next instanceof Node && event.currentTarget.contains(next)) return;
    if (hoverPreviewTarget.current === recordId) cancelHoverPreview();
    if (hoveredRow.current === recordId) hoveredRow.current = null;
    if (next instanceof Element && next.closest('.clipboard-panel .history-row')) return;
    suppressedHoverRow.current = null;
    setPointerOutside(true);
  }
  function loadNearEnd(element: HTMLDivElement) {
    if (element.clientHeight > 0 && element.scrollHeight - element.scrollTop - element.clientHeight < 180) void clipboard.loadMore();
  }
  function row(record: ClipboardRecord, tabbable: boolean) {
    const canTranslate = canTranslateRecord(record);
    const canOcr = canOcrRecord(record);
    const media = record.kind !== 'text' && <ClipboardMedia record={record} native={native} active={active} />;
    const kind = record.kind === 'image'
      ? t('clipboardImage')
      : record.kind === 'files'
        ? t('clipboardFiles')
        : record.hasRichText
          ? t('clipboardRichText')
          : t('clipboardPlainText');
    const keyboardCurrent = keyboardId === record.id;
    const slot = quickPaste && clipboard.displayReady && !clipboard.error ? quickPasteCandidates.findIndex(({ id }) => id === record.id) : -1;
    const quickHint = active && showQuickPasteHints && !menu && !editing && !pendingDelete
      ? (record.isPinned ? record.pinPasteKey : slot >= 0 ? String(slot + 1) : undefined)
      : undefined;
    const description = [
      record.kind !== 'text' && record.text ? kind : null,
      record.title ? `Note: ${record.title}` : null,
      record.isPinned ? 'Pinned' : null,
      record.isFavorite ? 'Favorite' : null,
      record.imageTextMatch ? t('clipboardImageTextMatch') : null,
    ].filter(Boolean).join(', ') || undefined;
    const translating = snapshot.window.clipboardTranslationVisible && snapshot.window.clipboardTranslationRecordId === record.id;
    return <article data-record-target={active && clipboard.displayReady && visibleIds.has(record.id) && (menu?.id ?? selected) === record.id || undefined} data-translating={translating || undefined} data-keyboard-current={keyboardCurrent || undefined} onFocus={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) { setSelected(record.id); previewRecord(record.id); } }} data-note-revealed={record.title ? revealed === record.id : undefined} onMouseEnter={() => { if (record.title) setRevealed(record.id); }} onPointerEnter={pointerEnteredRow} onMouseDown={() => { cancelHoverPreview(); suppressedHoverRow.current = record.id; }} onMouseMove={(event) => {
      if (!active || editing || menu || !clipboard.displayReady || !visibleIds.has(record.id)) return;
      const overActions = event.target instanceof Element && Boolean(event.target.closest('.history-row-actions'));
      if (targetOnMouseMove(event, () => { setSelected(record.id); previewRecord(record.id); })) {
        setPointerOutside(false);
        hoveredRow.current = record.id;
        if (suppressedHoverRow.current && suppressedHoverRow.current !== record.id) suppressedHoverRow.current = null;
        if (overActions) cancelHoverPreview();
        else scheduleHoverPreview(record.id);
      } else if (overActions && hoverPreviewTarget.current === record.id) {
        cancelHoverPreview();
      }
    }} onPointerLeave={(event) => pointerLeftRow(event, record.id)} onMouseLeave={() => setRevealed((current) => current === record.id ? null : current)} onContextMenu={(event) => { event.preventDefault(); setMenu({ id: record.id, x: event.clientX, y: event.clientY }); }} key={record.id} hidden={!visibleIds.has(record.id)} data-record-id={record.id} className={cn('history-row', selection === record.id && 'selected', menu?.id === record.id && 'context-target', record.isPinned && 'pinned')} style={{ '--history-actions-width': `${99 + 25 * (Number(canTranslate) + Number(canOcr))}px` } as CSSProperties}>
      <button type="button" tabIndex={tabbable ? 0 : -1} ref={(element) => { if (element) rowRefs.current.set(record.id, element); else rowRefs.current.delete(record.id); }} className="history-row-select" draggable={native && snapshot.capabilities.clipboardDrag && !editing && !menu} onDragStart={(event) => { if (!native || !snapshot.capabilities.clipboardDrag || editing || menu) return; cancelHoverPreview(); event.preventDefault(); void dragRecord(record.id); }} aria-label={`${t('clipboard')}: ${record.text || kind}`} aria-description={description} aria-keyshortcuts={quickHint ? `Meta+${quickHint}` : undefined} aria-current={keyboardCurrent || undefined} aria-pressed={selection === record.id} aria-haspopup="menu" onClick={() => { previewRecord(record.id, true); void useRecord(record.id, 'activate'); }}>
        <RecordHeading date={record.date} label={kind} sourceApp={record.sourceApp} quickPasteHint={quickHint} />
        {record.title ? <ClipboardNotePreview record={record} revealed={revealed === record.id} native={native} media={media} /> : media ? media : <ClipboardTextPreview text={historyPreview(record.text)} className="history-preview history-text-preview" />}
        {record.imageTextMatch && <span className="clipboard-search-match">{t('clipboardImageTextMatch')}</span>}
      </button>
      <RecordActions tabIndex={-1} allowMarkedDelete t={t} native={native} favorite={record.isFavorite} pinned={record.isPinned} deleteLabel={t('deleteClipboard')}
        onNote={() => setEditing({ id: record.id })} busy={processing !== null} translateActive={translating} onTranslate={canTranslate ? () => { void processRecord(record.id, 'translate'); } : undefined} onOcr={canOcr ? () => { void processRecord(record.id, 'ocr'); } : undefined}
        onUpdate={(action) => { updateRecord(record, action); }} />
    </article>;
  }
  return <section ref={panel} data-focus-ready={clipboard.displayReady || undefined} data-pointer-outside={pointerOutside || undefined} inert={editing !== null} onCompositionStartCapture={() => { composing.current = true; }} onCompositionEndCapture={() => { composing.current = false; }} className="history-panel clipboard-panel" style={{ '--history-preview-lines': snapshot.clipboardControls.previewLines?.clipboard } as CSSProperties} aria-label={t('clipboard')}>
    <ActivityBeam label={t('working')} active={active && clipboard.showActivity} />
    <div className="drag-strip" data-tauri-drag-region />
    <RecordToolbar t={t} snapshot={snapshot} command={command} query={query} onQuery={setQuery} search={search} onDetailsToggle={() => { void toggleDetails(); }} searchHint={windowShortcut('focusSearch')} pinHint={windowShortcut('pin')} />
    {clipboard.error && <p role="alert" className="history-error">{t(clipboard.searchError === 'invalidRegex' ? 'clipboardSearchInvalidRegex' : clipboard.searchError === 'tooBroad' ? 'clipboardSearchTooBroad' : 'nativeError')}
      {!clipboard.searchError && <> <button onClick={() => void clipboard.refresh()}>{t('retry')}</button></>}
    </p>}
    <HistoryList
      onScrollOwnerChange={restoreScroll}
      active={active}
      onScroll={() => { cancelHoverPreview(); remember(); clipboard.refreshDeferred(); }}
      list={list}
      pinnedList={pinnedList}
      handleRef={listHandle}
      pinnedCount={pinned.length}
      pinned={(retainedPins.length <= VIRTUALIZE_THRESHOLD ? retainedPins : pinned).map((record) => row(record, keyboardId === record.id))}
      virtualItems={{
        items: ordinary,
        allItems: allUnpinned,
        itemKey: clipboardRecordKey,
        renderItem: row,
        tabStopId: pinned.some(({ id }) => id === keyboardId) ? undefined : keyboardId,
        resetKey: snapshot.clipboardControls.previewLines?.clipboard,
      }}
      onNearEnd={loadNearEnd}
    >
      {!clipboard.loading && !clipboard.error && !records.length && <div className="history-empty clipboard-empty" role="status">
        <p>{query ? t('clipboardNoMatches') : favoritesOnly ? t('clipboardFavoritesEmpty') : kindFilter ? t(kindFilter === 'text' ? 'clipboardTextEmpty' : kindFilter === 'image' ? 'clipboardImagesEmpty' : 'clipboardFilesEmpty') : !snapshot.capabilities.clipboardWrite ? t('platformUnavailable') : snapshot.clipboardCaptureEnabled ? t('clipboardEmpty') : t('clipboardRecordingPaused')}</p>
        {kindFilter && <button type="button" onClick={() => { applyCategoryFilter(false, null, false); }}>{t('clearClipboardFilter')}</button>}
        {!query && !favoritesOnly && !kindFilter && native && !snapshot.storageError && snapshot.capabilities.clipboardWrite && !snapshot.clipboardCaptureEnabled && <button type="button" disabled={resumingCapture} onClick={() => {
          if (resumingCapture) return;
          setResumingCapture(true);
          setFeedback(null);
          void invoke('clipboard_capture_set', { enabled: true }).catch(() => { if (mounted.current) setFeedback('clipboardSettingsSaveFailed'); }).finally(() => { if (mounted.current) setResumingCapture(false); });
        }}>{t('clipboardResumeRecording')}</button>}
      </div>}
    </HistoryList>
    <HistoryFooter key={session} t={t} snapshot={snapshot} command={command} native={native} active={active && !menu && !editing && !pendingDelete} search={search} favoritesOnly={favoritesOnly} onFavorites={() => { applyCategoryFilter(kindFilter !== null || !favoritesOnly, null, false); }} kind={kindFilter} onKindChange={(next) => { applyCategoryFilter(false, next, false); }} total={clipboard.page.totalRecords} clipboard shortcutsHint={windowShortcut('showShortcuts')} />
    {menu && menuRecord && <ClipboardMenu key={menuRecord.id} record={menuRecord} snapshot={snapshot} x={menu.x} y={menu.y} canPaste={native && snapshot.capabilities.autoPaste} canTranslate={canTranslateRecord(menuRecord)} canOcr={canOcrRecord(menuRecord)} canOpenTarget={usableShortcut('openTarget')} t={t} busy={processing !== null} onClose={() => setMenu(null)} onReturnFocus={() => focusRow(menuRecord.id)} onAction={(action) => {
      setMenu(null);
      if (action === 'paste' || action === 'pastePlain' || action === 'pastePaths') void useRecord(menuRecord.id, action);
      else if (action === 'translate' || action === 'ocr') void processRecord(menuRecord.id, action);
      else if (action === 'openTarget') void openRecordTarget(menuRecord.id);
      else if (action === 'note') setEditing({ id: menuRecord.id });
      else updateRecord(menuRecord, action, rowRefs.current.get(menuRecord.id));
    }} />}
    {editing && <ClipboardEditor recordId={editing.id} onClose={() => setEditing(null)} onSaved={(detail) => clipboard.applyEdit(editing.id, detail)} onReturnFocus={() => { editorReturn.current = editing.id; }} />}
    {pendingDelete && <ClipboardDeleteConfirm t={t} description={markedDeleteDescription(t, pendingDelete.record)} onCancel={() => setPendingDelete(null)} onConfirm={() => { const pending = pendingDelete; setPendingDelete(null); commitDelete(pending.record, pending.source); }} />}
    <RecordFeedback message={active ? processingError ?? (feedback ? t(feedback) : null) : null} />
  </section>;
}
