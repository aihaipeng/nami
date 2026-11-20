import { invoke } from '@tauri-apps/api/core';

export const openTargetLabels = {
  link: 'clipboardOpenLink', email: 'clipboardComposeEmail', file: 'clipboardRevealFile', files: 'clipboardChooseFile',
} as const;

export function clipboardOpenTarget(recordId: string, execute: boolean, fileIndex?: number) {
  return invoke<keyof typeof openTargetLabels | null>('clipboard_open_target', { recordId, execute, fileIndex });
}

export function clipboardOpenError(error: unknown) {
  return (error instanceof Error ? error.message : error) === 'clipboard_file_missing' ? 'clipboardTargetFileMissing' : 'clipboardOpenFailed';
}
