import type { ClipboardResult } from '../shared/contracts';
import type { MessageKey } from '../shared/i18n';

export function clipboardUseFeedback(result: ClipboardResult): MessageKey | null {
  if (result.copyStatus === 'superseded') return 'clipboardChanged';
  if (result.status === 'copied' || result.status === 'dispatched') return null;
  if (result.code === 'clipboard_file_missing') return 'clipboardFileUnavailable';
  if (result.code === 'clipboard_image_missing') return 'clipboardImageUnavailable';
  return result.copyStatus === 'current' ? 'pasteManual' : 'copyFailed';
}

export function clipboardUseError(error: unknown): MessageKey {
  const code = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
  return code === 'clipboard_file_missing' ? 'clipboardFileUnavailable' : code === 'clipboard_image_missing' ? 'clipboardImageUnavailable' : 'nativeError';
}
