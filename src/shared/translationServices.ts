import appleIcon from '../assets/translation/apple-translate.svg?url';
import edgeIcon from '../assets/translation/microsoft-edge-translate.svg?url';
import googleIcon from '../assets/translation/google-translate.svg?url';
import { supportsTranslation, type TranslationEngine } from './languages';

export const translationServices = {
  edge: { code: 'edge', name: 'Microsoft Edge Translate', icon: edgeIcon, retry: 'retryWithEdge' },
  google: { code: 'google', name: 'Google Translate', icon: googleIcon, retry: 'retryWithGoogle' },
  apple: { code: 'apple', name: 'Apple Translate', icon: appleIcon, monochrome: true, retry: 'retryWithApple' },
} as const;

export function availableTranslationServices(appleAvailable: boolean) {
  return Object.values(translationServices).filter(({ code }) => code !== 'apple' || appleAvailable);
}

export function translationRetryServices(appleAvailable: boolean, current: TranslationEngine, source: string, targets: string[]) {
  return availableTranslationServices(appleAvailable).filter(({ code }) => code !== current && supportsTranslation(code, source, targets));
}

export function appleLanguagePackMissing(error: string | null | undefined) {
  return error?.startsWith('Apple Translate language pack is not downloaded.') ?? false;
}
