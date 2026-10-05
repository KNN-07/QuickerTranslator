import { useEffect, useRef } from 'react';
import { EditorSelection } from '@codemirror/state';
import type { InterfacePreferences, TraversalAction } from '../../app/preferences';
import { nativeAvailable } from '../../lib/ipc';
import { TRAVERSAL_ACTIONS } from '../../app/preferences';
import type { WorkspaceController } from '../workspace/useWorkspaceController';
import type { DocumentController } from './useDocuments';

export function traverseSource(controller: WorkspaceController, action: TraversalAction) {
  const state = controller.getState();
  const forward = action.endsWith('Next');
  const position = state.sourceSelection.start;
  if (action.startsWith('word') && state.sourceLanguage === 'ja' && state.result?.sourceRevision === state.sourceRevision) {
    const segments = state.result.segments;
    for (let index = forward ? 0 : segments.length - 1; index >= 0 && index < segments.length; index += forward ? 1 : -1) {
      const segment = segments[index]!;
      if ((forward ? segment.sourceRange.start > position : segment.sourceRange.start < position) && !/^[\s\p{P}\p{S}]+$/u.test(segment.surface)) {
        controller.selectSegment(segment.id); return;
      }
    }
    return;
  }
  if (action.startsWith('line') && controller.sourceView) {
    const view = controller.sourceView;
    const next = view.moveToLineBoundary(view.moveVertically(EditorSelection.cursor(position), forward), false, true).head;
    controller.setSourceSelection({ start: next, end: next }); return;
  }
  const word = action.startsWith('word');
  // Hán Việt words advance one Han scalar; paragraph moves skip empty lines and leading delimiters.
  const pattern = word ? /\p{Script=Han}|[^\p{Script=Han}\p{P}\p{S}\s]+/gu : /^[\s\p{P}\p{S}]*([^\s\p{P}\p{S}])/gmu;
  const limit = word ? position : position > 0 ? state.sourceText.lastIndexOf('\n', position - 1) + 1 : 0;
  if (forward) {
    if (word) pattern.lastIndex = position;
    else {
      const newline = state.sourceText.indexOf('\n', position);
      if (newline < 0) return;
      pattern.lastIndex = newline + 1;
    }
  }
  let next = position;
  for (let match = pattern.exec(state.sourceText); match; match = pattern.exec(state.sourceText)) {
    const start = word ? match.index : match.index + match[0].length - match[1]!.length;
    if (forward) {
      if (start > position) { next = start; break; }
    } else {
      if (start >= limit) break;
      next = start;
    }
  }
  controller.setSourceSelection({ start: next, end: next });
}
export function useKeyboard({ controller, documents, preferences, showSource, findSource, showTarget }: {
  controller: WorkspaceController; documents: DocumentController; preferences: InterfacePreferences;
  showSource: () => void; findSource: () => void; showTarget: () => void;
}) {
  const latest = useRef({ controller, documents, preferences, showSource, findSource, showTarget });
  latest.current = { controller, documents, preferences, showSource, findSource, showTarget };
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (event.isComposing || event.defaultPrevented || document.querySelector('dialog[open]')) return;
      const { controller: c, documents: d, preferences: p, showSource: reveal, findSource: find, showTarget: target } = latest.current;
      const key = event.key.toLowerCase(); const modified = event.ctrlKey || event.metaKey;
      if (modified && !event.altKey) {
        if (event.shiftKey && key === 'n') { event.preventDefault(); void d.createWindow(); return; }
        if (key === 'o' && !event.shiftKey) { event.preventDefault(); void d.openFile(); return; }
        if (key === 's') { event.preventDefault(); void d.saveDocument(event.shiftKey); return; }
        if (key === 'e' && !event.shiftKey) { event.preventDefault(); if (nativeAvailable()) d.setExportOpen(true); return; }
        if (key === 'f' && !event.shiftKey) { event.preventDefault(); find(); return; }
        if (!event.shiftKey && event.ctrlKey) {
          const action = TRAVERSAL_ACTIONS.find((candidate) => p.keybindings[candidate].toLowerCase() === key);
          if (action) { event.preventDefault(); reveal(); traverseSource(c, action); return; }
          if (/^[0-6]$/u.test(key)) {
            event.preventDefault(); const state = c.getState(); const result = state.result;
            if (!result || result.sourceRevision !== state.sourceRevision) return;
            const selected = result.segments.filter((segment) => segment.id === state.selectedSegmentId || (segment.sourceRange.start < state.sourceSelection.end && segment.sourceRange.end > state.sourceSelection.start));
            const active = selected.find((segment) => segment.id === state.selectedSegmentId) ?? selected.find((segment) => segment.meanings.length > 0);
            const text = key === '0' && selected.length > 0
              ? result.readings.slice(selected[0]!.readingsRange.start, selected.at(-1)!.readingsRange.end)
              : active?.meanings[Number(key) - 1];
            if (text) { target(); c.insertTarget(text); } return;
          }
        }
      }
      if (event.altKey && !event.shiftKey && !modified && (key === 'arrowleft' || key === 'arrowright')) {
        event.preventDefault(); void d.navigate(key === 'arrowleft' ? -1 : 1); return;
      }
      if (!event.altKey && !modified && !event.shiftKey && /^f[1-9]$/u.test(key)) {
        event.preventDefault(); const snippet = p.snippets[Number(key.slice(1)) - 1]; if (snippet) { target(); c.insertTarget(snippet); }
      }
    };
    window.addEventListener('keydown', keydown, true);
    return () => window.removeEventListener('keydown', keydown, true);
  }, []);
}
