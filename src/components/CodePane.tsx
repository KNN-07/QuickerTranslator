import { useEffect, useRef } from 'react';
import { Compartment, EditorState, StateEffect, StateField, Transaction } from '@codemirror/state';
import { Decoration, EditorView, keymap } from '@codemirror/view';
import type { DecorationSet } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { searchKeymap } from '@codemirror/search';
import type { TextRange, UiLocale } from '../lib/types';
import { validateRange } from '../lib/offsets';

const CM_VI_PHRASES: Record<string, string> = {
  Find: 'Tìm', Replace: 'Thay thế', next: 'tiếp', previous: 'trước', all: 'tất cả',
  'match case': 'phân biệt hoa/thường', regexp: 'biểu thức chính quy', 'by word': 'nguyên từ',
  replace: 'thay thế', 'replace all': 'thay tất cả', close: 'đóng',
  'Go to line': 'Đến dòng', go: 'đến', 'current match': 'kết quả hiện tại', 'on line': 'ở dòng',
  'replaced $ matches': 'đã thay $ kết quả', 'replaced match on line $': 'đã thay kết quả ở dòng $',
};

export interface CodeHighlight { id: string; range: TextRange; matched: boolean; selected: boolean; draggable?: boolean }
export interface CodeContext { x: number; y: number; segmentId: string | null; selection: string }
export interface CodePaneProps {
  text: string;
  label: string;
  locale: UiLocale;
  readOnly?: boolean;
  fontSize: number;
  wrap: boolean;
  highlights: CodeHighlight[];
  resetKey?: number;
  initialLine?: number;
  selection?: TextRange;
  getInitialState?: () => EditorState | null;
  onChange?: (text: string) => void;
  onSelection?: (range: TextRange) => void;
  onComposition?: (composing: boolean) => void;
  onSegmentClick?: (id: string) => void;
  onContext?: (context: CodeContext) => void;
  onMove?: (id: string, direction: -1 | 1) => void;
  onDrop?: (id: string, targetId: string) => void;
  onScroll?: (offset: number) => void;
  onView?: (view: EditorView | null) => void;
}
const replaceHighlights = StateEffect.define<DecorationSet>();
const highlightField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update: (value, transaction) => {
    let next = transaction.docChanged ? Decoration.none : value;
    for (const effect of transaction.effects) if (effect.is(replaceHighlights)) next = effect.value;
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export function CodePane(props: CodePaneProps) {
  const host = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const latest = useRef(props); latest.current = props;
  const appearance = useRef(new Compartment());
  const externalUpdate = useRef(false);
  const syncedText = useRef(props.text);
  const restoredLine = useRef<string | null>(null);
  useEffect(() => {
    if (!host.current) return;
    const settings = latest.current;
    let pointerDrag: { id: string; pointerId: number; x: number; y: number; moved: boolean } | null = null;
    const segmentAt = (position: number) => latest.current.highlights.find((mark) => mark.range.start <= position && mark.range.end > position);
    const positionAtEvent = (event: MouseEvent | DragEvent, view: EditorView) => view.posAtCoords({ x: event.clientX, y: event.clientY });
    const config = {
        doc: settings.text,
        extensions: [
          highlightField,
          EditorState.readOnly.of(Boolean(settings.readOnly)),
          EditorView.editable.of(!settings.readOnly),
          EditorView.contentAttributes.of({ 'aria-label': settings.label, role: 'textbox', 'aria-multiline': 'true', spellcheck: 'false', tabindex: '0' }),
          keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap]),
          ...(!settings.readOnly ? [history()] : []),
          appearance.current.of([EditorState.phrases.of(settings.locale === 'vi' ? CM_VI_PHRASES : {}), EditorView.theme({ '&': { fontSize: `${settings.fontSize}px` }, '.cm-content': { fontFamily: 'inherit' } }), ...(settings.wrap ? [EditorView.lineWrapping] : [])]),
          EditorView.updateListener.of((update) => {
            if (externalUpdate.current) return;
            if (update.docChanged) {
              syncedText.current = update.state.doc.toString();
              latest.current.onChange?.(syncedText.current);
            }
            if (update.selectionSet || update.docChanged) {
              const selection = update.state.selection.main;
              latest.current.onSelection?.({ start: selection.from, end: selection.to });
            }
          }),
          EditorView.domEventHandlers({
            compositionstart: () => { latest.current.onComposition?.(true); return false; },
            compositionend: () => { window.setTimeout(() => latest.current.onComposition?.(false), 0); return false; },
            click: (event, currentView) => {
              const position = positionAtEvent(event, currentView);
              const segment = position === null ? null : segmentAt(position);
              if (segment && latest.current.readOnly) latest.current.onSegmentClick?.(segment.id);
              return false;
            },
            contextmenu: (event, currentView) => {
              if (!latest.current.onContext) return false;
              event.preventDefault();
              const position = positionAtEvent(event, currentView);
              const segment = position === null ? null : segmentAt(position);
              const range = currentView.state.selection.main;
              latest.current.onContext({ x: event.clientX, y: event.clientY, segmentId: segment?.id ?? null,
                selection: currentView.state.sliceDoc(range.from, range.to) });
              return true;
            },
            keydown: (event, currentView) => {
              if (!event.altKey || !event.shiftKey || (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') || event.isComposing || !latest.current.onMove) return false;
              const position = currentView.state.selection.main.head;
              const segment = latest.current.highlights.find((mark) => mark.selected && mark.draggable) ?? segmentAt(position);
              if (!segment) return false;
              event.preventDefault(); latest.current.onMove(segment.id, event.key === 'ArrowLeft' ? -1 : 1); return true;
            },
            pointerdown: (event, currentView) => {
              if (!latest.current.onDrop || event.button !== 0 || event.shiftKey || event.ctrlKey || event.metaKey) return false;
              const element = event.target instanceof Element ? event.target.closest('[data-segment-id]') : null;
              const id = element?.getAttribute('data-segment-id');
              const mark = latest.current.highlights.find((item) => item.id === id && item.draggable);
              if (!mark) return false;
              pointerDrag = { id: mark.id, pointerId: event.pointerId, x: event.clientX, y: event.clientY, moved: false };
              currentView.contentDOM.setPointerCapture(event.pointerId);
              currentView.focus();
              currentView.dispatch({ selection: { anchor: mark.range.start, head: mark.range.end } });
              latest.current.onSegmentClick?.(mark.id);
              return true;
            },
            pointermove: (event, currentView) => {
              if (!pointerDrag || pointerDrag.pointerId !== event.pointerId) return false;
              if (Math.hypot(event.clientX - pointerDrag.x, event.clientY - pointerDrag.y) >= 5) {
                pointerDrag.moved = true;
                currentView.dom.classList.add('token-dragging');
              }
              return true;
            },
            pointerup: (event, currentView) => {
              if (!pointerDrag || pointerDrag.pointerId !== event.pointerId) return false;
              const drag = pointerDrag;
              pointerDrag = null;
              currentView.dom.classList.remove('token-dragging');
              if (currentView.contentDOM.hasPointerCapture(event.pointerId)) currentView.contentDOM.releasePointerCapture(event.pointerId);
              if (drag.moved) {
                const position = positionAtEvent(event, currentView);
                const target = position === null ? null : segmentAt(position);
                if (target) latest.current.onDrop?.(drag.id, target.id);
              }
              return true;
            },
            pointercancel: (_event, currentView) => {
              pointerDrag = null;
              currentView.dom.classList.remove('token-dragging');
              return false;
            },
            lostpointercapture: (_event, currentView) => {
              pointerDrag = null;
              currentView.dom.classList.remove('token-dragging');
              return false;
            },
          }),
        ],
    };
    const cached = settings.getInitialState?.();
    const state = cached ? cached.update({
      effects: StateEffect.reconfigure.of(config.extensions),
      ...(cached.doc.toString() === settings.text ? {} : { changes: { from: 0, to: cached.doc.length, insert: settings.text } }),
    }).state : EditorState.create(config);
    syncedText.current = settings.text;
    const view = new EditorView({ parent: host.current, state });
    const scroll = () => {
      const bounds = view.scrollDOM.getBoundingClientRect();
      const offset = view.posAtCoords({ x: bounds.left + 8, y: bounds.top + 4 }) ?? view.viewport.from;
      latest.current.onScroll?.(offset);
    };
    view.scrollDOM.addEventListener('scroll', scroll, { passive: true });
    viewRef.current = view; latest.current.onView?.(view);
    return () => {
      if (pointerDrag && view.contentDOM.hasPointerCapture(pointerDrag.pointerId)) view.contentDOM.releasePointerCapture(pointerDrag.pointerId);
      view.scrollDOM.removeEventListener('scroll', scroll);
      latest.current.onView?.(null); viewRef.current = null; view.destroy();
    };
  }, [props.resetKey]);
  useEffect(() => {
    const view = viewRef.current;
    if (!view || syncedText.current === props.text) return;
    externalUpdate.current = true;
    syncedText.current = props.text;
    try {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: props.text }, annotations: Transaction.addToHistory.of(false) });
    } finally { externalUpdate.current = false; }
  }, [props.text, props.resetKey]);
  useEffect(() => {
    const view = viewRef.current;
    const range = props.selection;
    if (!view || !range || !validateRange(props.text, range)) return;
    const current = view.state.selection.main;
    if (current.from === range.start && current.to === range.end) return;
    externalUpdate.current = true;
    try { view.dispatch({ selection: { anchor: range.start, head: range.end }, scrollIntoView: true }); }
    finally { externalUpdate.current = false; }
  }, [props.selection, props.text, props.resetKey]);
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.contentDOM.setAttribute('aria-label', props.label);
    view.dispatch({ effects: appearance.current.reconfigure([EditorState.phrases.of(props.locale === 'vi' ? CM_VI_PHRASES : {}), EditorView.theme({ '&': { fontSize: `${props.fontSize}px` }, '.cm-content': { fontFamily: 'inherit' } }), ...(props.wrap ? [EditorView.lineWrapping] : [])]) });
  }, [props.fontSize, props.wrap, props.locale, props.label, props.resetKey]);
  useEffect(() => {
    const view = viewRef.current;
    if (!view || props.initialLine === undefined || !props.text) return;
    const key = `${props.resetKey ?? 0}:${props.initialLine}`;
    if (restoredLine.current === key) return;
    restoredLine.current = key;
    const line = view.state.doc.line(Math.min(view.state.doc.lines, Math.max(1, props.initialLine + 1)));
    view.dispatch({ effects: EditorView.scrollIntoView(line.from, { y: 'start' }) });
  }, [props.initialLine, props.text, props.resetKey]);
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const marks = props.highlights.filter((mark) => mark.range.start < mark.range.end && validateRange(props.text, mark.range)).map((mark) => Decoration.mark({
      class: `${mark.matched ? 'aligned-match' : ''} ${mark.selected ? 'aligned-selection' : ''}`.trim(),
      attributes: { 'data-segment-id': mark.id, ...(mark.draggable ? { 'data-reorderable': 'true' } : {}) },
    }).range(mark.range.start, mark.range.end));
    view.dispatch({ effects: replaceHighlights.of(Decoration.set(marks, true)) });
  }, [props.highlights, props.text, props.resetKey]);
  return <div className="code-pane" ref={host} />;
}
