import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { initialSnapshot, type AppSnapshot, type Locale, type TranslationPreferences, type WindowState, type WindowTarget } from './shared/contracts';
import { resolveLanguage, translator } from './shared/i18n';
import { Translation } from './translation/Translation';
import { HistoryView } from './translation/HistoryView';
import { RecordsWorkspace } from './shared/RecordsWorkspace';
import { Settings } from './settings/Settings';
import { Waves } from 'lucide-react';
import { cn } from './lib/utils';
import { ActivityBeam } from './shared/ActivityBeam';

type ClipboardBindingChange = { binding: AppSnapshot['clipboardBinding']; enabled: boolean; shortcutErrors: string[] };

export const native = isTauri();
document.documentElement.dataset.native = String(native);

type ViewTarget = WindowTarget | 'history';

function currentTarget(): ViewTarget {
  const value = native ? getCurrentWebview().label : new URLSearchParams(location.search).get('window');
  return value === 'clipboard' || value === 'settings' || value === 'history' ? value : 'translation';
}

export default function App() {
  const [target, setTarget] = useState<ViewTarget>(currentTarget);
  const [snapshot, setSnapshot] = useState<AppSnapshot>(initialSnapshot);
  const [connected, setConnected] = useState(!native);
  const [error, setError] = useState(false);
  const [pending, setPending] = useState(0);
  const t = translator(snapshot.locale);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const removers: (() => void)[] = [];
    let latestWindow: WindowState | undefined;
    let latestLocale: Locale | undefined;
    let latestEngine: AppSnapshot['translationEngine'] | undefined;
    let latestPreferences: TranslationPreferences | undefined;
    let latestCapture: boolean | undefined;
    let latestRecordingTypes: AppSnapshot['clipboardRecordingTypes'] | undefined;
    let latestClipboardControls: AppSnapshot['clipboardControls'] | undefined;
    let latestClipboardBinding: ClipboardBindingChange | undefined;
    async function connect() {
      for (const subscribe of [
        () => getCurrentWindow().onMoved(() => window.dispatchEvent(new Event('nami:window-moved'))),
        () => listen<'edge' | 'google'>('translation_engine_changed', ({ payload }) => {
          latestEngine = payload;
          if (!disposed) setSnapshot((current) => ({ ...current, translationEngine: payload, translationPreferences: { ...current.translationPreferences, engine: payload } }));
        }),
        () => listen<TranslationPreferences>('translation_preferences_changed', ({ payload }) => {
          latestPreferences = payload;
          latestEngine = payload.engine;
          if (!disposed) setSnapshot((current) => ({ ...current, translationEngine: payload.engine, translationPreferences: payload }));
        }),
        () => listen<boolean>('clipboard_capture_changed', ({ payload }) => { latestCapture = payload; if (!disposed) setSnapshot((current) => ({ ...current, clipboardCaptureEnabled: payload })); }),
        ...(getCurrentWebview().label === 'settings' ? [
          () => listen<AppSnapshot['clipboardRecordingTypes']>('clipboard_recording_types_changed', ({ payload }) => {
            latestRecordingTypes = payload;
            if (!disposed) setSnapshot((current) => ({ ...current, clipboardRecordingTypes: payload }));
          }),
        ] : []),
        () => listen<ClipboardBindingChange>('clipboard_binding_changed', ({ payload }) => {
          latestClipboardBinding = payload;
          if (!disposed) setSnapshot((current) => ({ ...current, clipboardBinding: payload.binding, clipboardShortcutEnabled: payload.enabled, shortcutErrors: payload.shortcutErrors }));
        }),
        () => listen<AppSnapshot['clipboardControls']>('clipboard_controls_changed', ({ payload }) => {
          latestClipboardControls = payload;
          if (!disposed) setSnapshot((current) => ({ ...current, clipboardControls: payload }));
        }),
        () => listen<string>('storage_error', ({ payload }) => { if (!disposed) setSnapshot((current) => ({ ...current, storageError: payload })); }),
        () => listen('layout_error', () => { if (!disposed) setError(true); }),
        () => listen<WindowState>('window_state', ({ payload }) => {
          latestWindow = payload;
          if (!disposed) setSnapshot((current) => ({ ...current, window: payload }));
        }),
        () => listen<Locale>('locale_changed', ({ payload }) => {
          latestLocale = payload;
          if (!disposed) setSnapshot((current) => ({ ...current, locale: payload }));
        }),
      ]) {
        const remove = await subscribe();
        if (disposed) { remove(); return; }
        removers.push(remove);
      }
      const data = await invoke<AppSnapshot>('app_snapshot');
      if (!disposed) {
        document.documentElement.dataset.nativeMaterial = String(data.capabilities.nativeMaterial);
        document.documentElement.dataset.nativeHistory = String(data.capabilities.nativeHistory);
        setSnapshot({ ...data, clipboardControls: latestClipboardControls ?? data.clipboardControls, clipboardCaptureEnabled: latestCapture ?? data.clipboardCaptureEnabled, clipboardRecordingTypes: latestRecordingTypes ?? data.clipboardRecordingTypes, clipboardBinding: latestClipboardBinding ? latestClipboardBinding.binding : data.clipboardBinding, clipboardShortcutEnabled: latestClipboardBinding?.enabled ?? data.clipboardShortcutEnabled, shortcutErrors: latestClipboardBinding?.shortcutErrors ?? data.shortcutErrors, locale: latestLocale ?? data.locale, translationEngine: latestEngine ?? data.translationEngine, translationPreferences: latestPreferences ?? data.translationPreferences, window: latestWindow ?? data.window });
        setConnected(true);
      }
    }
    void connect().catch(() => { if (!disposed) setError(true); });
    return () => { disposed = true; removers.forEach((remove) => remove()); };
  }, []);

  useEffect(() => {
    if (connected) {
      document.documentElement.dataset.nativeMaterial = String(snapshot.capabilities.nativeMaterial);
      document.documentElement.dataset.nativeHistory = String(snapshot.capabilities.nativeHistory);
    }
  }, [connected, snapshot.capabilities]);

  useEffect(() => {
    document.documentElement.lang = resolveLanguage(snapshot.locale);
    document.title = `Nami · ${t(target)}`;
  }, [snapshot.locale, target, t]);

  async function command(name: string, args?: Record<string, unknown>) {
    setError(false);
    if (!native) {
      if (name === 'records_view_set') setSnapshot((value) => {
        const keep = value.window.historyVisible && value.window.translationVisible;
        return { ...value, window: { ...value.window, recordView: args?.view as 'clipboard' | 'translation', historyVisible: true, historyExtent: value.window.historyWidth, translationVisible: keep && value.window.translationVisible, translationExtent: keep ? value.window.translationExtent : 0, clipboardDetailId: keep ? value.window.clipboardDetailId : null } };
      });
      if (name === 'clipboard_details_select') setSnapshot((value) => ({ ...value, window: { ...value.window, clipboardDetailId: String(args?.recordId) } }));
      if (name === 'records_details_toggle') setSnapshot((value) => {
        const recordId = args?.recordId as string | undefined;
        const visible = !value.window.translationVisible || Boolean(recordId && value.window.clipboardDetailId !== recordId);
        return { ...value, window: { ...value.window, translationVisible: visible, translationExtent: visible ? value.window.translationWidth : 0, ...(recordId ? { clipboardDetailId: recordId, recordsBrowsing: false } : {}) } };
      });
      if (name === 'history_visibility_set' || name === 'history_toggle') setSnapshot((value) => {
        const visible = name === 'history_toggle' ? value.window.recordView !== 'translation' || !value.window.historyVisible : Boolean(args?.visible);
        return { ...value, window: { ...value.window, recordView: 'translation', translationVisible: true, translationExtent: value.window.translationWidth, historyVisible: visible, historyExtent: visible ? value.window.historyWidth : 0 } };
      });
      if (name === 'history_width_set') setSnapshot((value) => ({ ...value, window: { ...value.window, historyWidth: Math.min(720, Math.max(280, Number(args?.width))), historyExtent: value.window.historyVisible ? Math.min(720, Math.max(280, Number(args?.width))) : 0 } }));
      if (name === 'translation_columns_set') setSnapshot((value) => ({ ...value, window: { ...value.window, secondaryEnabled: Boolean(args?.secondaryEnabled) } }));
      if (name === 'window_pin') setSnapshot((value) => ({ ...value, window: { ...value.window, [`${String(args?.region)}Pinned`]: Boolean(args?.pinned) } }));
      if (name === 'locale_set') setSnapshot((value) => ({ ...value, locale: args?.locale as Locale }));
      if (name === 'translation_preferences_set') setSnapshot((value) => {
        const next = { ...value.translationPreferences, ...args, engine: (args?.engine as TranslationPreferences['engine']) ?? value.translationPreferences.engine };
        return { ...value, translationEngine: next.engine, translationPreferences: next };
      });
      if (name === 'window_show') {
        setTarget(args?.target as WindowTarget);
        if (args?.target === 'translation') setSnapshot((value) => ({ ...value, window: { ...value.window, translationVisible: true, translationExtent: value.window.translationWidth } }));
      }
      if (name === 'window_hide') setSnapshot((value) => ({ ...value, window: { ...value.window, translationVisible: false, translationExtent: 0, historyVisible: value.window.historyVisible && (value.window.historyPinned || Boolean(args?.keepHistory)), historyExtent: value.window.historyVisible && (value.window.historyPinned || Boolean(args?.keepHistory)) ? value.window.historyWidth : 0 } }));
      return;
    }
    const showsActivity = !['records_view_set', 'records_details_toggle', 'clipboard_details_select', 'clipboard_note_open', 'history_toggle', 'history_visibility_set', 'history_width_set', 'history_sample_select', 'translation_columns_set', 'window_pin', 'window_hide', 'window_show'].includes(name);
    if (showsActivity) setPending((count) => count + 1);
    try {
      await invoke(name, args);
      if (showsActivity) {
        const refreshed = await invoke<AppSnapshot>('app_snapshot');
        setSnapshot((current) => ({ ...current, capabilities: refreshed.capabilities, accessibility: refreshed.accessibility, shortcutErrors: refreshed.shortcutErrors, translationEngine: refreshed.translationEngine, translationPreferences: refreshed.translationPreferences, storageError: refreshed.storageError, clipboardCaptureEnabled: refreshed.clipboardCaptureEnabled }));
      }
    } catch {
      setError(true);
    } finally {
      if (showsActivity) setPending((count) => count - 1);
    }
  }

  function activity(delta: 1 | -1) { setPending((count) => count + delta); }
  const common = { t, snapshot, command, native, activity };
  return <>
    {!native && <nav className="preview-bar" aria-label={t('preview')}>
      <span className="preview-brand"><Waves /><strong>Nami</strong><span>{t('preview')}</span></span>
      <div className="preview-navigation">{(['translation', 'clipboard', 'settings'] as const).map((name) => <button key={name} aria-pressed={target === name} onClick={() => setTarget(name)}>{t(name)}</button>)}</div>
    </nav>}
    {snapshot.storageError && <div role="alert" className="global-error">{t('historyStorageError')}</div>}
    {error && <div role="alert" className="global-error">{t('nativeError')}</div>}
    <main className={cn('app-surface', target)}>
      <ActivityBeam label={t('working')} active={!connected || pending > 0} />
      {connected && target === 'translation' && <Translation {...common} />}
      {connected && target === 'history' && <HistoryView {...common} />}
      {connected && target === 'clipboard' && <RecordsWorkspace {...common} />}
      {connected && target === 'settings' && <Settings {...common} />}
    </main>
  </>;
}

export interface ViewProps {
  t: ReturnType<typeof translator>;
  snapshot: AppSnapshot;
  command: (name: string, args?: Record<string, unknown>) => Promise<void>;
  native: boolean;
  activity: (delta: 1 | -1) => void;
}
