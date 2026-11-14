import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { initialSnapshot, type AppSnapshot, type Locale, type WindowState, type WindowTarget } from './shared/contracts';
import { resolveLanguage, translator } from './shared/i18n';
import { Translation } from './translation/Translation';
import { HistoryView } from './translation/HistoryView';
import { Clipboard } from './clipboard/Clipboard';
import { Settings } from './settings/Settings';
import { Waves } from 'lucide-react';
import { cn } from './lib/utils';
import { ActivityBeam } from './shared/ActivityBeam';

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
    async function connect() {
      for (const subscribe of [
        () => getCurrentWindow().onMoved(() => window.dispatchEvent(new Event('nami:window-moved'))),
        () => listen<'edge' | 'google'>('translation_engine_changed', ({ payload }) => {
          latestEngine = payload;
          if (!disposed) setSnapshot((current) => ({ ...current, translationEngine: payload }));
        }),
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
      if (!disposed) setSnapshot({ ...data, locale: latestLocale ?? data.locale, translationEngine: latestEngine ?? data.translationEngine, window: latestWindow ?? data.window });
    }
    void connect().catch(() => { if (!disposed) setError(true); });
    return () => { disposed = true; removers.forEach((remove) => remove()); };
  }, []);

  useEffect(() => {
    document.documentElement.lang = resolveLanguage(snapshot.locale);
    document.title = `Nami · ${t(target)}`;
  }, [snapshot.locale, target, t]);

  async function command(name: string, args?: Record<string, unknown>) {
    setError(false);
    if (!native) {
      if (name === 'history_visibility_set' || name === 'history_toggle') setSnapshot((value) => {
        const visible = name === 'history_toggle' ? !value.window.historyVisible : Boolean(args?.visible);
        return { ...value, window: { ...value.window, translationVisible: true, translationExtent: value.window.translationWidth, historyVisible: visible, historyExtent: visible ? value.window.historyWidth : 0 } };
      });
      if (name === 'history_width_set') setSnapshot((value) => ({ ...value, window: { ...value.window, historyWidth: Math.min(720, Math.max(280, Number(args?.width))), historyExtent: value.window.historyVisible ? Math.min(720, Math.max(280, Number(args?.width))) : 0 } }));
      if (name === 'translation_columns_set') setSnapshot((value) => ({ ...value, window: { ...value.window, secondaryEnabled: Boolean(args?.secondaryEnabled) } }));
      if (name === 'window_pin') setSnapshot((value) => ({ ...value, window: { ...value.window, [`${String(args?.region)}Pinned`]: Boolean(args?.pinned) } }));
      if (name === 'locale_set') setSnapshot((value) => ({ ...value, locale: args?.locale as Locale }));
      if (name === 'window_show') {
        setTarget(args?.target as WindowTarget);
        if (args?.target === 'translation') setSnapshot((value) => ({ ...value, window: { ...value.window, translationVisible: true, translationExtent: value.window.translationWidth } }));
      }
      if (name === 'window_hide') setSnapshot((value) => ({ ...value, window: { ...value.window, translationVisible: false, translationExtent: 0, historyVisible: value.window.historyVisible && (value.window.historyPinned || Boolean(args?.keepHistory)), historyExtent: value.window.historyVisible && (value.window.historyPinned || Boolean(args?.keepHistory)) ? value.window.historyWidth : 0 } }));
      return;
    }
    const showsActivity = !['history_toggle', 'history_visibility_set', 'history_width_set', 'history_sample_select', 'translation_columns_set', 'window_pin', 'window_hide', 'window_show'].includes(name);
    if (showsActivity) setPending((count) => count + 1);
    try {
      await invoke(name, args);
      if (showsActivity) {
        const refreshed = await invoke<AppSnapshot>('app_snapshot');
        setSnapshot((current) => ({ ...current, accessibility: refreshed.accessibility, shortcutErrors: refreshed.shortcutErrors, translationEngine: refreshed.translationEngine }));
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
    {error && <div role="alert" className="global-error">{t('nativeError')}</div>}
    <main className={cn('app-surface', target)}>
      {pending > 0 && <ActivityBeam label={t('working')} />}
      {target === 'translation' && <Translation {...common} />}
      {target === 'history' && <HistoryView {...common} />}
      {target === 'clipboard' && <Clipboard {...common} />}
      {target === 'settings' && <Settings {...common} />}
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
