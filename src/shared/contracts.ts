export type Locale = 'system' | 'zh-CN' | 'en';
export type Region = 'translation' | 'history' | 'clipboard' | 'settings';
export type WindowTarget = Exclude<Region, 'history'>;

export interface WindowState {
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
  clipboardWrite: boolean;
  autoPaste: boolean;
  accessibilityPermission: boolean;
  primaryModifier: 'meta' | 'control';
}

export interface AppSnapshot {
  capabilities: PlatformCapabilities;
  locale: Locale;
  translationEngine: 'edge' | 'google';
  storageError: string | null;
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
  capabilities: { platform: 'other', nativeHistory: false, nativeChrome: false, nativeMaterial: false, selection: false, clipboardWrite: false, autoPaste: false, accessibilityPermission: false, primaryModifier: 'control' },
  locale: 'en',
  translationEngine: 'edge',
  storageError: null,
  accessibility: false,
  shortcutErrors: [],
  window: {
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
