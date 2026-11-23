import { invoke } from '@tauri-apps/api/core';

export function clipboardOpenTarget(recordId: string, execute: boolean, fileIndex?: number) {
  return invoke<'link' | 'email' | 'file' | 'files' | null>('clipboard_open_target', { recordId, execute, fileIndex });
}

export function clipboardOpenError(error: unknown) {
  return (error instanceof Error ? error.message : error) === 'clipboard_file_missing' ? 'clipboardTargetFileMissing' : 'clipboardOpenFailed';
}
