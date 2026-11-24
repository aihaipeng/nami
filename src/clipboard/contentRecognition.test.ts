import { describe, expect, it } from 'vitest';
import { recognizeClipboardText } from './contentRecognition';

describe('clipboard L1 text content recognition', () => {
  it('recognizes valid CSS colors and marks translucent colors for checkerboard preview', () => {
    expect(recognizeClipboardText('#09f').match).toEqual({ kind: 'color', cssColor: '#09f', hasAlpha: false });
    expect(recognizeClipboardText('  #1A2b3C  ')).toEqual({
      leading: '  ',
      trimmed: '#1A2b3C',
      trailing: '  ',
      match: { kind: 'color', cssColor: '#1a2b3c', hasAlpha: false },
    });
    expect(recognizeClipboardText('#ff0088cc').match).toEqual({ kind: 'color', cssColor: '#ff0088cc', hasAlpha: true });
    expect(recognizeClipboardText('#ff0088ff').match).toEqual({ kind: 'color', cssColor: '#ff0088ff', hasAlpha: false });
    expect(recognizeClipboardText('#f08a').match).toEqual({ kind: 'color', cssColor: '#f08a', hasAlpha: true });
    expect(recognizeClipboardText('rgb(255, 128, 0)').match).toEqual({ kind: 'color', cssColor: 'rgb(255 128 0)', hasAlpha: false });
    expect(recognizeClipboardText('rgba(255, 128, 0, 0.4)').match).toEqual({ kind: 'color', cssColor: 'rgb(255 128 0 / 0.4)', hasAlpha: true });
    expect(recognizeClipboardText('rgb(100% 50% 0% / 50%)').match).toEqual({ kind: 'color', cssColor: 'rgb(100% 50% 0% / 50%)', hasAlpha: true });
    expect(recognizeClipboardText('hsl(210, 50%, 40%)').match).toEqual({ kind: 'color', cssColor: 'hsl(210deg 50% 40%)', hasAlpha: false });
    expect(recognizeClipboardText('hsla(210deg 50% 40% / 0.8)').match).toEqual({ kind: 'color', cssColor: 'hsl(210deg 50% 40% / 0.8)', hasAlpha: true });
    expect(recognizeClipboardText('oklch(0.65 0.18 250)').match).toEqual({ kind: 'color', cssColor: 'oklch(0.65 0.18 250deg)', hasAlpha: false });
    expect(recognizeClipboardText('oklch(65% 0.18 250 / 80%)').match).toEqual({ kind: 'color', cssColor: 'oklch(65% 0.18 250deg / 80%)', hasAlpha: true });
  });

  it('rejects out-of-range, numeric 4-digit references, named words, and injected color strings', () => {
    for (const value of [
      '#2026',
      '#8080',
      '#12345',
      '#gggggg',
      'red',
      'orange',
      'rgb(256, 0, 0)',
      'rgb(10, 20)',
      'rgb(10, 20, 30, 1.5)',
      'hsl(210, 120%, 40%)',
      'oklch(1.5 0.2 240)',
      'oklch(0.5, 0.2, 240)',
      '#ff0000; background: url(https://evil.test)',
      'rgb(0, 0, 0)\n#fff',
    ]) {
      expect(recognizeClipboardText(value).match.kind, value).toBe('plain');
    }
  });

  it('recognizes single links and emails matching backend open_target rules while preserving exact text slices', () => {
    const cases: Array<[string, { kind: 'link'; prefix: string; host: string; rest: string } | { kind: 'email'; local: string; domain: string }]> = [
      ['https://github.com/aihaipeng/nami?tab=code#readme', { kind: 'link', prefix: 'https://', host: 'github.com', rest: '/aihaipeng/nami?tab=code#readme' }],
      ['http://localhost:5173/settings', { kind: 'link', prefix: 'http://', host: 'localhost:5173', rest: '/settings' }],
      ['www.example.test/abc', { kind: 'link', prefix: '', host: 'www.example.test', rest: '/abc' }],
      ['HTTPS://Docs.Example.Test', { kind: 'link', prefix: 'HTTPS://', host: 'Docs.Example.Test', rest: '' }],
      ['a+b@example.test', { kind: 'email', local: 'a+b', domain: 'example.test' }],
    ];
    for (const [input, expected] of cases) {
      const result = recognizeClipboardText(input);
      expect(result.match).toEqual(expected);
      if (result.match.kind === 'link') {
        expect(result.leading + result.match.prefix + result.match.host + result.match.rest + result.trailing).toBe(input);
      } else if (result.match.kind === 'email') {
        expect(`${result.leading}${result.match.local}@${result.match.domain}${result.trailing}`).toBe(input);
      }
    }

    for (const rejected of [
      'example.test',
      'see https://example.test',
      'https://example.test\nappendix',
      'https:///example.test',
      'https://user:pass@example.test',
      'file:///tmp/test',
      'javascript:alert(1)',
      'mailto:a@example.test?bcc=x',
      'a@example.test,b@example.test',
      'a..b@example.test',
      'www.localhost',
    ]) {
      expect(recognizeClipboardText(rejected).match.kind, rejected).toBe('plain');
    }
  });

  it('recognizes JSON, code, SQL, markup, and CLI commands without false-positiving on prose', () => {
    for (const code of [
      '{"name":"nami","version":2}',
      '{\n  "name": "nami",\n  "truncated": "long value without closing brace',
      '[1, 2, 3]',
      'import { useState } from "react";',
      'const total = items.reduce((sum, item) => sum + item.price, 0);',
      'pub fn compute_window_origin(mode: WindowPositionMode) -> ScreenPoint {',
      'SELECT id, text FROM clipboard_records WHERE is_pinned = 1;',
      'git commit -m "Add L1 recognition"',
      'cargo test --manifest-path src-tauri/Cargo.toml',
      'curl -s https://api.example.test/v1/health',
      '<div class="card"><span>Hello</span></div>',
      '.history-row { border-radius: 10px; }',
      'if (ready) {\n  runTask(item);\n  done = true;\n}',
    ]) {
      expect(recognizeClipboardText(code).match.kind, code).toBe('code');
    }

    for (const prose of [
      'Hello world',
      '今天下午三点开会，记得带上设计稿。',
      'Meeting notes:\n  Time: 10:00\n  Owner: Alice',
      'go to the store and buy milk',
      'find a good place for lunch',
      'make sure the window stays centered',
      'docker is a container tool',
      '[1]',
      'First line of a note.\nSecond line of a note.\nThird line of a note.',
    ]) {
      expect(recognizeClipboardText(prose).match.kind, prose).toBe('plain');
    }
  });
});
