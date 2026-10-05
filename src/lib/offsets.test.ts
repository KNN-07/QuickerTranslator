import { describe, expect, it } from 'vitest';
import { isScalarBoundary, overlaps, selectedText, validateRange } from './offsets';

describe('editor UTF-16 boundaries', () => {
  it('selects an astral scalar as two units and rejects a split surrogate', () => {
    const source = '你好。🙂𠀀';
    expect(selectedText(source, { start: 3, end: 5 })).toBe('🙂');
    expect(selectedText(source, { start: 5, end: 7 })).toBe('𠀀');
    expect(isScalarBoundary(source, 4)).toBe(false);
    expect(() => selectedText(source, { start: 4, end: 5 })).toThrow(RangeError);
    expect(validateRange(source, { start: 5, end: 4 })).toBe(false);
    expect(validateRange(source, { start: -1, end: 1 })).toBe(false);
    expect(validateRange(source, { start: 0, end: 8 })).toBe(false);
  });
  it('does not highlight touching half-open ranges', () => {
    expect(overlaps({ start: 1, end: 3 }, { start: 3, end: 5 })).toBe(false);
    expect(overlaps({ start: 1, end: 3 }, { start: 2, end: 5 })).toBe(true);
    expect(overlaps({ start: 2, end: 2 }, { start: 1, end: 3 })).toBe(false);
    expect(overlaps({ start: 1, end: 3 }, { start: 2, end: 2 })).toBe(false);
  });
});
