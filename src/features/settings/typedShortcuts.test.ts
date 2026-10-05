import { describe, expect, it } from 'vitest';
import { typedShortcutTransition } from './typedShortcuts';
import type { TypedInput, TypedToken } from './typedShortcuts';

const shortcuts = [{ key: 'xc', value: 'Xin chào' }];
function input(from: number, text: string, extras: Partial<TypedInput> = {}): TypedInput {
  return { from, to: from, text, typed: true, composing: false, boundary: from === 8, ...extras };
}
describe('only typed target tokens expand', () => {
  it('matches case-insensitively on a typed delimiter using real node positions', () => {
    const first = typedShortcutTransition(null, input(8, 'X'), shortcuts);
    const second = typedShortcutTransition(first.token, input(9, 'c'), shortcuts);
    expect(second.token).toEqual({ from: 8, to: 10, text: 'Xc' });
    expect(typedShortcutTransition(second.token, input(10, ' '), shortcuts)).toEqual({ token: null, replacement: { from: 8, to: 10, text: 'Xin chào' } });
    expect(typedShortcutTransition(second.token, input(10, '\n'), shortcuts).replacement?.text).toBe('Xin chào');
  });
  it('never expands paste, IME input, selections, or text that was already in the target', () => {
    const token: TypedToken = { from: 8, to: 10, text: 'xc' };
    expect(typedShortcutTransition(token, input(10, ' ', { typed: false }), shortcuts).replacement).toBeNull();
    expect(typedShortcutTransition(token, input(10, ' ', { composing: true }), shortcuts).replacement).toBeNull();
    expect(typedShortcutTransition(token, input(10, ' ', { to: 12 }), shortcuts).replacement).toBeNull();
    expect(typedShortcutTransition(null, input(8, 'xc'), shortcuts).token).toBeNull();
    expect(typedShortcutTransition(null, input(10, ' '), shortcuts).replacement).toBeNull();
    expect(typedShortcutTransition(null, input(10, 'c', { boundary: false }), shortcuts).token).toBeNull();
  });
  it('drops tracking after cursor movement and keeps scalar lengths in UTF-16 positions', () => {
    const token: TypedToken = { from: 8, to: 10, text: 'xc' };
    expect(typedShortcutTransition(token, input(20, ' '), shortcuts).replacement).toBeNull();
    expect(typedShortcutTransition(null, input(8, '𠀀'), shortcuts).token).toEqual({ from: 8, to: 10, text: '𠀀' });
  });
  it('preserves typed punctuation in imported keys and returns dictionary text literally', () => {
    const records = [{ key: 'x-c', value: '<b>chào</b>' }];
    let token = typedShortcutTransition(null, input(8, 'X'), records).token;
    token = typedShortcutTransition(token, input(9, '-'), records).token;
    token = typedShortcutTransition(token, input(10, 'c'), records).token;
    expect(typedShortcutTransition(token, input(11, ' '), records).replacement).toEqual({ from: 8, to: 11, text: '<b>chào</b>' });
  });
});
