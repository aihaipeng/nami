import { expect, it } from 'vitest';
import { displaySource, editSource } from './source';

it('displays pasted line breaks on one line without changing the source', () => {
  const source = 'first\r\n\r\nthird 🌊\n';
  expect(displaySource(source)).toBe('first  third 🌊 ');
  expect(editSource(source, displaySource(source))).toBe(source);
  expect(editSource(source, 'First  third 🌊 ')).toBe('First\r\n\r\nthird 🌊\n');
  expect(editSource(source, 'first  last 🌊 ')).toBe('first\r\n\r\nlast 🌊\n');
});

it('preserves inserted multiline text and handles replacement and deletion', () => {
  expect(editSource('hello', 'hello\n\nworld\n')).toBe('hello\n\nworld\n');
  expect(editSource('a\r\nb', 'ab')).toBe('ab');
  expect(editSource('a\r\nb', 'x')).toBe('x');
  expect(editSource('🌊\ntext', '🌊 more text')).toBe('🌊\nmore text');
});
