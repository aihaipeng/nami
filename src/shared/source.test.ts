import { expect, it } from 'vitest';
import { displaySource, dragSourceSelection, editSource, sourceDragDelta, sourceIndexAt, type SourceDragField } from './source';

it('displays pasted line breaks on one line without changing the source', () => {
  const source = 'first\r\n\r\nthird 🌊\n';
  expect(displaySource(source)).toBe('first  third 🌊 ');
  expect(editSource(source, displaySource(source))).toBe(source);
  expect(editSource(source, 'First  third 🌊 ')).toBe('First\r\n\r\nthird 🌊\n');
  expect(editSource(source, 'first  last 🌊 ')).toBe('first\r\n\r\nlast 🌊\n');
});

it('scrolls a dragged source selection toward hidden text in either direction', () => {
  const width = (prefix: string) => prefix.length * 10;
  expect(sourceDragDelta(50, 0, 120)).toBe(0);
  expect(sourceIndexAt('abcd', 25, width)).toBe(2);
  expect(sourceIndexAt('ab', 100, width)).toBe(2);
  const field = {
    value: 'abcdefghijklmnopqrst',
    scrollLeft: 0,
    scrollWidth: 200,
    clientWidth: 100,
    selection: [4, 4, 'none'] as [number, number, 'forward' | 'backward' | 'none' | undefined],
    setSelectionRange(this: SourceDragField & { selection: [number, number, 'forward' | 'backward' | 'none' | undefined] }, start: number, end: number, direction?: 'forward' | 'backward' | 'none') {
      this.selection = [start, end, direction];
    },
  };
  dragSourceSelection(field, { pointerX: 160, left: 0, right: 100, paddingLeft: 0, anchor: 4, width });
  expect(field.scrollLeft).toBe(24);
  expect(field.selection).toEqual([4, 12, 'forward']);
  field.scrollLeft = 80;
  dragSourceSelection(field, { pointerX: -20, left: 0, right: 100, paddingLeft: 0, anchor: 12, width });
  expect(field.scrollLeft).toBe(58);
  expect(field.selection?.[2]).toBe('backward');
  expect(field.selection?.[0]).toBeLessThan(12);
  field.scrollLeft = 0;
  dragSourceSelection(field, { pointerX: -40, left: 0, right: 100, paddingLeft: 0, anchor: 2, width });
  expect(field.scrollLeft).toBe(0);
  field.scrollLeft = 100;
  dragSourceSelection(field, { pointerX: 180, left: 0, right: 100, paddingLeft: 0, anchor: 10, width });
  expect(field.scrollLeft).toBe(100);
});

it('preserves inserted multiline text and handles replacement and deletion', () => {
  expect(editSource('hello', 'hello\n\nworld\n')).toBe('hello\n\nworld\n');
  expect(editSource('a\r\nb', 'ab')).toBe('ab');
  expect(editSource('a\r\nb', 'x')).toBe('x');
  expect(editSource('🌊\ntext', '🌊 more text')).toBe('🌊\nmore text');
});
