import { defaultClipboardShortcuts, type ClipboardControls } from './clipboardShortcuts';
import { defaultClipboardBinding, defaultTranslationShortcuts, type KeyBinding, type ShortcutAction, type TranslationShortcuts } from './shortcuts';

export type Locale = 'system' | 'zh-CN' | 'en';
export type WindowPositionMode = 'lastPosition' | 'center' | 'mouse' | 'cursor';
export type Region = 'translation' | 'clipboard' | 'settings';
export type WindowTarget = Region;
export type TranslationScope = 'standalone' | 'clipboard';

export interface LanguageDetection {
  rawLanguage: string;
  language: string;
  score: number | null;
}

export interface TranslationMetadata {
  requestedSource: string;
  engine: 'edge' | 'google';
  targetEngines?: ('edge' | 'google')[];
  strategyVersion: number;
  detections: LanguageDetection[];
}

export interface TranslationOutput {
  id: string;
  text: string;
  sourceLanguage: string;
  translation: TranslationMetadata;
  targetLanguages: string[];
  translations: string[];
}

export interface WindowState {
  recordsBrowsing?: boolean;
  recordsEditorOpen?: boolean;
  clipboardDetailId?: string | null;
  clipboardTranslationRecordId?: string | null;
  recordsSession?: number;
  recordsFocus?: number;
  standaloneTranslationVisible: boolean;
  clipboardVisible: boolean;
  clipboardDetailVisible: boolean;
  clipboardTranslationVisible: boolean;
  translationPinned: boolean;
  clipboardPinned: boolean;
  clipboardWidth: number;
  clipboardDetailWidth: number;
  clipboardTranslationWidth: number;
  translationWidth: number;
  clipboardExtent: number;
  rightExtent: number;
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
  clipboardOpenTarget: boolean;
  clipboardImageSave: boolean;
  clipboardDrag: boolean;
  localBackup: boolean;
  launchAtLogin: boolean;
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
  selectionEnabled: boolean;
  disabledShortcuts: ShortcutAction[];
  shortcuts: TranslationShortcuts;
}

export function translationShortcutEnabled(preferences: TranslationPreferences, action: ShortcutAction) {
  return action === 'selection' ? preferences.selectionEnabled : !preferences.disabledShortcuts.includes(action);
}

export interface AppSnapshot {
  capabilities: PlatformCapabilities;
  locale: Locale;
  windowPositionMode: WindowPositionMode;
  launchAtLogin: boolean;
  translationEngine: 'edge' | 'google';
  translationPreferences: TranslationPreferences;
  storageError: string | null;
  clipboardCaptureEnabled: boolean;
  clipboardRecordingTypes: { text: boolean; image: boolean; files: boolean };
  clipboardBinding: KeyBinding | null;
  clipboardShortcutEnabled: boolean;
  clipboardControls: ClipboardControls;
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
  capabilities: { platform: 'other', nativeHistory: false, nativeChrome: false, nativeMaterial: false, selection: false, imageOcr: false, clipboardWrite: false, autoPaste: false, clipboardOpenTarget: false, clipboardImageSave: false, clipboardDrag: false, localBackup: false, launchAtLogin: false, accessibilityPermission: false, primaryModifier: 'control' },
  locale: 'en',
  windowPositionMode: 'lastPosition',
  launchAtLogin: false,
  translationEngine: 'edge',
  translationPreferences: {
    engine: 'edge',
    sourceLanguage: 'auto',
    primaryLanguage: 'en',
    secondaryLanguage: 'zh-CN',
    proxyUrl: '',
    proxyEnabled: false,
    selectionEnabled: true,
    disabledShortcuts: [],
    shortcuts: defaultTranslationShortcuts('control'),
  },
  storageError: null,
  clipboardCaptureEnabled: false,
  clipboardRecordingTypes: { text: true, image: true, files: true },
  clipboardBinding: defaultClipboardBinding,
  clipboardShortcutEnabled: true,
  clipboardControls: { recordsOpenFocus: 'list', previewLines: { clipboard: 3 }, activation: 'paste', shortcuts: defaultClipboardShortcuts('control') },
  accessibility: false,
  shortcutErrors: [],
  window: {
    recordsBrowsing: true,
    clipboardDetailId: null,
    clipboardTranslationRecordId: null,
    recordsSession: 0,
    recordsFocus: 0,
    standaloneTranslationVisible: true,
    clipboardVisible: false,
    clipboardDetailVisible: false,
    clipboardTranslationVisible: false,
    translationPinned: false,
    clipboardPinned: false,
    clipboardWidth: 360,
    clipboardDetailWidth: 480,
    clipboardTranslationWidth: 440,
    translationWidth: 920,
    clipboardExtent: 0,
    rightExtent: 920,
    secondaryEnabled: true,
  },
};
