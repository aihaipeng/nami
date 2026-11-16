import { defaultTranslationShortcuts, type TranslationShortcuts } from './shortcuts';

export type Locale = 'system' | 'zh-CN' | 'en';
export type Region = 'translation' | 'history' | 'clipboard' | 'settings';
export type WindowTarget = Exclude<Region, 'history'>;

export interface WindowState {
  recordsEditorOpen?: boolean;
  clipboardDetailId?: string | null;
  recordView?: 'clipboard' | 'translation';
  recordsSession?: number;
  recordsFocus?: number;
  translationVisible: boolean;
  historyVisible: boolean;
  translationPinned: boolean;
  historyPinned: boolean;
  clipboardPinned: boolean;
  historyWidth: number;
  translationWidth: number;
  historyExtent: number;
  translationExtent: number;
  secondaryEnabled: boolean;
}

export interface PlatformCapabilities {
  platform: 'macos' | 'windows' | 'linux' | 'other';
  nativeHistory: boolean;
  nativeChrome: boolean;
  nativeMaterial: boolean;
  selection: boolean;
  imageOcr: boolean;
  clipboardWrite: boolean;
  autoPaste: boolean;
  accessibilityPermission: boolean;
  primaryModifier: 'meta' | 'control';
}

export interface TranslationPreferences {
  engine: 'edge' | 'google';
  sourceLanguage: string;
  primaryLanguage: string;
  secondaryLanguage: string;
  proxyUrl: string;
  proxyEnabled: boolean;
  shortcuts: TranslationShortcuts;
}

export interface AppSnapshot {
  capabilities: PlatformCapabilities;
  locale: Locale;
  translationEngine: 'edge' | 'google';
  translationPreferences: TranslationPreferences;
  storageError: string | null;
  clipboardCaptureEnabled: boolean;
  accessibility: boolean;
  shortcutErrors: string[];
  window: WindowState;
}

export interface ClipboardResult {
  status: 'copied' | 'dispatched' | 'failed';
  copyStatus: 'not_written' | 'current' | 'superseded';
  code: string | null;
}

export const initialSnapshot: AppSnapshot = {
  capabilities: { platform: 'other', nativeHistory: false, nativeChrome: false, nativeMaterial: false, selection: false, imageOcr: false, clipboardWrite: false, autoPaste: false, accessibilityPermission: false, primaryModifier: 'control' },
  locale: 'en',
  translationEngine: 'edge',
  translationPreferences: {
    engine: 'edge',
    sourceLanguage: 'auto',
    primaryLanguage: 'en',
    secondaryLanguage: 'zh-CN',
    proxyUrl: '',
    proxyEnabled: false,
    shortcuts: defaultTranslationShortcuts('control'),
  },
  storageError: null,
  clipboardCaptureEnabled: false,
  accessibility: false,
  shortcutErrors: [],
  window: {
    recordView: 'clipboard',
    recordsSession: 0,
    recordsFocus: 0,
    translationVisible: true,
    historyVisible: false,
    translationPinned: false,
    historyPinned: false,
    clipboardPinned: false,
    historyWidth: 360,
    translationWidth: 920,
    historyExtent: 0,
    translationExtent: 920,
    secondaryEnabled: true,
  },
};
