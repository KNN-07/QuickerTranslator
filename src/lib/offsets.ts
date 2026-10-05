import type { TextRange } from './types';

export function isScalarBoundary(text: string, offset: number): boolean {
  if (!Number.isInteger(offset) || offset < 0 || offset > text.length) return false;
  if (offset === 0 || offset === text.length) return true;
  const previous = text.charCodeAt(offset - 1);
  const current = text.charCodeAt(offset);
  return !(previous >= 0xd800 && previous <= 0xdbff && current >= 0xdc00 && current <= 0xdfff);
}

export function validateRange(text: string, range: TextRange): boolean {
  return range.start <= range.end && isScalarBoundary(text, range.start) && isScalarBoundary(text, range.end);
}

export function overlaps(left: TextRange, right: TextRange): boolean {
  return left.start < left.end && right.start < right.end
    && left.start < right.end && right.start < left.end;
}

export function selectedText(text: string, range: TextRange): string {
  if (!validateRange(text, range)) throw new RangeError('Selection splits a Unicode scalar or exceeds the document');
  return text.slice(range.start, range.end);
}
