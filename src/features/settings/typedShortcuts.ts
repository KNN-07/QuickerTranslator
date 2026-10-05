import { Extension } from '@tiptap/core';
import { Plugin, PluginKey } from '@tiptap/pm/state';
import { closeHistory } from '@tiptap/pm/history';
import { canSplit } from '@tiptap/pm/transform';
import type { ShortcutRecord } from '../../lib/dictionaryTypes';

export interface TypedToken { from: number; to: number; text: string }
export interface TypedInput { from: number; to: number; text: string; typed: boolean; composing: boolean; boundary: boolean }
export interface TypedTransition { token: TypedToken | null; replacement: { from: number; to: number; text: string } | null }
/** Only a continuously typed token, not pre-existing or pasted text, can expand. Positions are ProseMirror positions. */
export function typedShortcutTransition(token: TypedToken | null, input: TypedInput, shortcuts: readonly ShortcutRecord[]): TypedTransition {
  if (!input.typed || input.composing || input.from !== input.to || input.text.length !== (input.text.codePointAt(0)! > 0xffff ? 2 : 1)) return { token: null, replacement: null };
  const delimiter = /^[\s\p{P}\p{S}]$/u.test(input.text);
  if (delimiter) {
    const key = token?.text.toLowerCase();
    const record = token?.to === input.from ? shortcuts.find((candidate) => candidate.key === key) : undefined;
    if (record && token) return { token: null, replacement: { from: token.from, to: token.to, text: record.value } };
    // Punctuation inside an imported key is still typed text, not a premature token break.
    if (!/\s/u.test(input.text) && (token?.to === input.from || input.boundary)) {
      const text = (token?.to === input.from ? token.text : '') + input.text;
      const prefix = text.toLowerCase();
      if (shortcuts.some((candidate) => candidate.key.startsWith(prefix))) return {
        token: { from: token?.to === input.from ? token.from : input.from, to: input.from + input.text.length, text }, replacement: null,
      };
    }
    return { token: null, replacement: null };
  }
  if (token?.to === input.from) return { token: { from: token.from, to: input.from + input.text.length, text: token.text + input.text }, replacement: null };
  return { token: input.boundary ? { from: input.from, to: input.from + input.text.length, text: input.text } : null, replacement: null };
}
const typedKey = new PluginKey<TypedToken | null>('typed-vietnamese-shortcuts');
export function typedShortcuts(getShortcuts: () => readonly ShortcutRecord[]) {
  return Extension.create({
    name: 'typedVietnameseShortcuts',
    addProseMirrorPlugins() {
      let composition = false;
      let compositionTimer: number | undefined;
      return [new Plugin<TypedToken | null>({
        key: typedKey,
        state: {
          init: () => null,
          apply: (transaction, token) => {
            if (transaction.getMeta(typedKey) !== undefined) return transaction.getMeta(typedKey) as TypedToken | null;
            if (transaction.docChanged || (transaction.selectionSet && transaction.selection.head !== token?.to)) return null;
            return token;
          },
        },
        props: {
          handleDOMEvents: {
            compositionstart: (view) => { composition = true; window.clearTimeout(compositionTimer); view.dispatch(view.state.tr.setMeta(typedKey, null)); return false; },
            compositionend: () => { compositionTimer = window.setTimeout(() => { composition = false; }, 0); return false; },
            paste: (view) => { view.dispatch(view.state.tr.setMeta(typedKey, null)); return false; },
            drop: (view) => { view.dispatch(view.state.tr.setMeta(typedKey, null)); return false; },
          },
          handleTextInput: (view, from, to, text) => {
            if (view.composing || composition || text.length !== (text.codePointAt(0)! > 0xffff ? 2 : 1)) return false;
            const resolved = view.state.doc.resolve(from);
            const previous = resolved.parent.textBetween(Math.max(0, resolved.parentOffset - 2), resolved.parentOffset);
            const boundary = resolved.parentOffset === 0 || /[\s\p{P}\p{S}]$/u.test(previous);
            const transition = typedShortcutTransition(typedKey.getState(view.state) ?? null, { from, to, text, typed: true, composing: false, boundary }, getShortcuts());
            let transaction = view.state.tr;
            if (transition.replacement) {
              const replacement = transition.replacement;
              transaction = closeHistory(transaction.insertText(replacement.text + text, replacement.from, to)).setMeta(typedKey, null);
              view.dispatch(transaction); view.dispatch(closeHistory(view.state.tr));
            } else view.dispatch(transaction.insertText(text, from, to).setMeta(typedKey, transition.token));
            return true;
          },
          handleKeyDown: (view, event) => {
            if (event.key !== 'Enter' || event.shiftKey || event.ctrlKey || event.metaKey || event.altKey || event.isComposing || view.composing || composition) return false;
            const { from, to } = view.state.selection;
            const token = typedKey.getState(view.state) ?? null;
            const transition = typedShortcutTransition(token, { from, to, text: '\n', typed: true, composing: false, boundary: false }, getShortcuts());
            if (!transition.replacement) return false;
            const replacement = transition.replacement;
            const transaction = view.state.tr.insertText(replacement.text, replacement.from, replacement.to);
            const position = replacement.from + replacement.text.length;
            if (!canSplit(transaction.doc, position)) return false;
            view.dispatch(closeHistory(transaction.split(position)).setMeta(typedKey, null)); view.dispatch(closeHistory(view.state.tr));
            return true;
          },
        },
        view: () => ({ destroy: () => window.clearTimeout(compositionTimer) }),
      })];
    },
  });
}
