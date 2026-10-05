import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useEditor } from '@tiptap/react';
import type { Editor, JSONContent } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import TextAlign from '@tiptap/extension-text-align';
import { TextStyleKit } from '@tiptap/extension-text-style';
import { closeHistory } from '@tiptap/pm/history';
import { EditorState as TargetEditorState } from '@tiptap/pm/state';
import type { SelectionBookmark } from '@tiptap/pm/state';
import { EditorView } from '@codemirror/view';
import { EditorState as CodeEditorState, Transaction } from '@codemirror/state';
import { isolateHistory } from '@codemirror/commands';
import { listen } from '@tauri-apps/api/event';
import { readText } from '@tauri-apps/plugin-clipboard-manager';
import { invokeCommand, nativeAvailable, toAppError } from '../../lib/ipc';
import { overlaps, validateRange } from '../../lib/offsets';
import { DEFAULT_TRANSLATION_OPTIONS } from '../../lib/types';
import type { AppError, SourceLanguage, TextRange, TranslationOptions, TranslationResult, UiLocale } from '../../lib/types';
import { acceptsTranslation, canApplyTarget, DEFAULT_VIEW_STATE, emptyDraft, moveDraftToken, orderedTokens, projectDraft } from './state';
import type { DraftEdits, DraftProjection, WorkspaceDocument, WorkspaceSnapshot, WorkspaceViewState } from './state';
import type { ShortcutRecord } from '../../lib/dictionaryTypes';
import { typedShortcuts } from '../settings/typedShortcuts';

export type AlignedPane = 'source' | 'readings' | 'phrases' | 'singleMeaning';
export interface TargetCapture {
  bookmark: SelectionBookmark;
  documentId: string;
  sourceRevision: number;
  targetRevision: number;
  from: number;
  to: number;
  text: string;
}
export interface WorkspaceControllerInputs {
  locale: UiLocale;
  documentId?: string;
  sourceRevision?: number;
  targetRevision?: number;
  dictionaryRevision?: number;
  japaneseAvailable?: boolean;
  shortcuts?: readonly ShortcutRecord[];
  confirmDiscardDraft: () => Promise<boolean>;
  onState?: (state: WorkspaceSnapshot) => void;
  onInvalidate?: () => void;
}
export interface WorkspaceController {
  state: WorkspaceSnapshot;
  targetEditor: Editor | null;
  sourceView: EditorView | null;
  draft: DraftProjection;
  selectedIds: string[];
  setSourceText: (text: string, fromEditor?: boolean) => void;
  setSourceLanguage: (language: SourceLanguage) => Promise<void>;
  setSourceSelection: (range: TextRange) => void;
  setComposing: (composing: boolean) => void;
  selectSegment: (id: string) => void;
  chooseMeaning: (id: string, meaning: string) => Promise<void>;
  moveToken: (id: string, direction: -1 | 1) => void;
  dropToken: (id: string, targetId: string) => void;
  translate: (explicit?: boolean) => Promise<void>;
  translateClipboard: () => Promise<void>;
  cancel: () => void;
  setOptions: (options: TranslationOptions) => void;
  loadPreferences: (viewState: WorkspaceViewState, options: TranslationOptions) => void;
  setViewState: (viewState: WorkspaceViewState) => void;
  registerView: (pane: AlignedPane, view: EditorView | null) => void;
  getSourceEditorState: () => CodeEditorState | null;
  syncScroll: (pane: AlignedPane, offset: number) => void;
  captureTarget: () => TargetCapture | null;
  insertTarget: (text: string) => boolean;
  applyTarget: (text: string, capture: TargetCapture, wholeDocument?: boolean) => boolean;
  getTargetJSON: () => JSONContent;
  serialize: () => WorkspaceDocument;
  restore: (document: WorkspaceDocument) => void;
  markSaved: () => void;
  markDirty: () => void;
  reportError: (error: AppError | null) => void;
  getState: () => WorkspaceSnapshot;
  flushObservations: () => Promise<void>;
}
const EMPTY_TARGET: JSONContent = { type: 'doc', content: [{ type: 'paragraph' }] };
function textContent(text: string): JSONContent[] {
  return text.split('\n').map((line) => ({ type: 'paragraph', ...(line ? { content: [{ type: 'text', text: line }] } : {}) }));
}

export function useWorkspaceController(inputs: WorkspaceControllerInputs): WorkspaceController {
  const inputsRef = useRef(inputs); inputsRef.current = inputs;
  const native = nativeAvailable();
  const [state, setState] = useState<WorkspaceSnapshot>(() => ({
    documentId: inputs.documentId ?? crypto.randomUUID(), sourceLanguage: 'zh', sourceText: '', sourceRevision: inputs.sourceRevision ?? 0,
    targetRevision: inputs.targetRevision ?? 0, editorEpoch: 0, targetDocument: EMPTY_TARGET, targetText: '', sourceSelection: { start: 0, end: 0 },
    selectedSegmentId: null, result: null, draftEdits: emptyDraft(inputs.sourceRevision ?? 0, ''), draftDirty: false, dirty: false,
    translating: false, composing: false, dictionaryRevision: inputs.dictionaryRevision ?? 0,
    options: { ...DEFAULT_TRANSLATION_OPTIONS }, viewState: structuredClone(DEFAULT_VIEW_STATE), error: null, needsRetranslation: false,
  }));
  const stateRef = useRef(state);
  const generation = useRef(0);
  const mounted = useRef(true);
  const nativeGate = useRef<Promise<void>>(Promise.resolve());
  const bookmark = useRef<SelectionBookmark | null>(null);
  const restoring = useRef(false);
  const views = useRef<Partial<Record<AlignedPane, EditorView>>>({});
  const sourceCache = useRef<{ epoch: number; state: CodeEditorState } | null>(null);
  const sourceViewEpoch = useRef(0);
  const [sourceView, setSourceView] = useState<EditorView | null>(null);
  const scrollGuard = useRef(0);
  const pendingDraft = useRef<DraftEdits | null>(null);
  const commit = useCallback((change: Partial<WorkspaceSnapshot>) => {
    stateRef.current = { ...stateRef.current, ...change };
    if (mounted.current) setState(stateRef.current);
  }, []);
  const reportError = useCallback((error: AppError | null) => commit({ error }), [commit]);
  const confirmDraftDiscard = useCallback(async (snapshot: WorkspaceSnapshot) => {
    try {
      const approved = await inputsRef.current.confirmDiscardDraft();
      return mounted.current && approved
        && stateRef.current.sourceRevision === snapshot.sourceRevision
        && stateRef.current.draftEdits === snapshot.draftEdits;
    } catch (failure: unknown) {
      if (mounted.current) reportError(toAppError(failure));
      return false;
    }
  }, [reportError]);
  const enqueueNative = useCallback((operation: () => Promise<unknown>) => {
    nativeGate.current = nativeGate.current.catch(() => {}).then(operation).then(() => {}).catch((failure: unknown) => {
      if (mounted.current) reportError(toAppError(failure));
    });
  }, [reportError]);
  const flushObservations = useCallback(() => nativeGate.current, []);
  const observeSource = useCallback(() => {
    if (!native || !inputsRef.current.documentId) return;
    const current = stateRef.current;
    enqueueNative(() => invokeCommand('observe_document_source', {
      documentId: current.documentId, sourceRevision: current.sourceRevision,
      sourceLanguage: current.sourceLanguage, sourceText: current.sourceText, options: current.options,
    }));
  }, [native, enqueueNative]);
  const cancel = useCallback(() => {
    generation.current += 1;
    commit({ translating: false });
    inputsRef.current.onInvalidate?.();
    if (native && inputsRef.current.documentId) enqueueNative(() => invokeCommand('cancel_offline_translation'));
  }, [native, enqueueNative, commit]);
  const targetEditor = useEditor({
    extensions: [StarterKit.configure({ link: false }), TextStyleKit, TextAlign.configure({ types: ['paragraph', 'heading'] }), typedShortcuts(() => inputsRef.current.shortcuts ?? [])],
    content: EMPTY_TARGET,
    editorProps: { attributes: { class: 'vietnamese-editor', spellcheck: 'true', lang: 'vi', role: 'textbox', 'aria-multiline': 'true', 'aria-label': inputs.locale === 'vi' ? 'Việt' : 'Vietnamese' } },
    onTransaction: ({ editor, transaction }) => {
      if (transaction.docChanged && bookmark.current) bookmark.current = bookmark.current.map(transaction.mapping);
      if (transaction.selectionSet && !restoring.current) bookmark.current = editor.state.selection.getBookmark();
    },
    onSelectionUpdate: ({ editor }) => { bookmark.current = editor.state.selection.getBookmark(); },
    onUpdate: ({ editor }) => {
      if (restoring.current) return;
      const current = stateRef.current;
      const targetRevision = current.targetRevision + 1;
      commit({ targetDocument: editor.getJSON(), targetText: editor.getText({ blockSeparator: '\n' }), targetRevision, dirty: true });
      if (native && inputsRef.current.documentId) enqueueNative(() => invokeCommand('observe_document_target', { documentId: current.documentId, targetRevision }));
    },
  });
  const targetRef = useRef(targetEditor); targetRef.current = targetEditor;
  const setSourceText = useCallback((sourceText: string, fromEditor = false) => {
    const current = stateRef.current;
    if (sourceText === current.sourceText) return;
    if (!fromEditor) {
      const view = views.current.source;
      const annotations = [Transaction.userEvent.of('input.replace'), isolateHistory.of('full')];
      if (view) {
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: sourceText }, annotations });
        return; // CodePane's real editor update commits the new source once.
      }
      const cache = sourceCache.current;
      if (cache?.epoch === current.editorEpoch) cache.state = cache.state.update({ changes: { from: 0, to: cache.state.doc.length, insert: sourceText }, annotations }).state;
    }
    cancel(); pendingDraft.current = null;
    const sourceRevision = current.sourceRevision + 1;
    commit({ sourceText, sourceRevision, sourceSelection: { start: 0, end: 0 }, selectedSegmentId: null,
      result: current.draftDirty ? current.result : null, dirty: true, error: null, needsRetranslation: true,
      ...(current.draftDirty ? {} : { draftEdits: emptyDraft(sourceRevision, sourceText) }) });
    observeSource();
  }, [cancel, commit, observeSource]);
  const setSourceLanguage = useCallback(async (sourceLanguage: SourceLanguage) => {
    const current = stateRef.current;
    if (sourceLanguage === current.sourceLanguage) return;
    if (current.draftDirty && !(await confirmDraftDiscard(current))) return;
    cancel(); pendingDraft.current = null;
    const sourceRevision = current.sourceRevision + 1;
    commit({ sourceLanguage, sourceRevision, result: null, selectedSegmentId: null, draftEdits: emptyDraft(sourceRevision, current.sourceText),
      draftDirty: false, dirty: true, error: null, needsRetranslation: true });
    observeSource();
  }, [cancel, commit, observeSource, confirmDraftDiscard]);
  const setComposing = useCallback((composing: boolean) => { if (composing) cancel(); commit({ composing }); }, [cancel, commit]);
  const setSourceSelection = useCallback((sourceSelection: TextRange) => {
    if (!validateRange(stateRef.current.sourceText, sourceSelection)) return;
    const result = stateRef.current.result;
    const selected = result?.sourceRevision === stateRef.current.sourceRevision
      ? result.segments.find((segment) => overlaps(segment.sourceRange, sourceSelection)
        || (sourceSelection.start === sourceSelection.end && segment.sourceRange.start <= sourceSelection.start && segment.sourceRange.end > sourceSelection.start)) : null;
    commit({ sourceSelection, selectedSegmentId: selected?.id ?? null });
  }, [commit]);
  const selectSegment = useCallback((id: string) => {
    const current = stateRef.current;
    if (current.result?.sourceRevision !== current.sourceRevision) return;
    const segment = current.result.segments.find((item) => item.id === id);
    if (!segment) return;
    commit({ selectedSegmentId: id, sourceSelection: segment.sourceRange });
    const view = views.current.source;
    if (view && segment.sourceRange.end <= view.state.doc.length) {
      view.dispatch({ selection: { anchor: segment.sourceRange.start, head: segment.sourceRange.end }, scrollIntoView: true });
    }
  }, [commit]);
  const translate = useCallback(async (explicit = true) => {
    let current = stateRef.current;
    if (current.composing || !native || !inputsRef.current.documentId) return;
    if (current.sourceLanguage === 'ja' && !inputsRef.current.japaneseAvailable) {
      cancel(); commit({ result: null, needsRetranslation: false }); return;
    }
    if (current.draftDirty && pendingDraft.current === null) {
      if (!explicit) { commit({ needsRetranslation: true }); return; }
      if (!(await confirmDraftDiscard(current))) return;
      commit({ draftDirty: false, draftEdits: emptyDraft(current.sourceRevision, current.sourceText) });
    }
    cancel();
    current = stateRef.current;
    if (current.sourceText.length === 0) { commit({ result: null, needsRetranslation: false }); return; }
    const ticket = { generation: generation.current, documentId: current.documentId, sourceRevision: current.sourceRevision,
      sourceLanguage: current.sourceLanguage, dictionaryRevision: current.dictionaryRevision };
    const request = { documentId: current.documentId, sourceRevision: current.sourceRevision, sourceLanguage: current.sourceLanguage, sourceText: current.sourceText, options: current.options };
    commit({ translating: true, needsRetranslation: false, error: null });
    await nativeGate.current;
    if (ticket.generation !== generation.current || !mounted.current) return;
    try {
      const result = await invokeCommand<TranslationResult>('translate_offline', { request });
      if (!mounted.current || !acceptsTranslation(stateRef.current, ticket, generation.current, result)) return;
      const restoredDraft = pendingDraft.current;
      pendingDraft.current = null;
      const draftEdits = restoredDraft ? { ...restoredDraft, sourceRevision: result.sourceRevision } : emptyDraft(result.sourceRevision, request.sourceText);
      const projection = projectDraft(request.sourceText, result, draftEdits, request.options);
      commit({ result, dictionaryRevision: result.dictionaryRevision, translating: false, draftEdits,
        draftDirty: restoredDraft !== null, needsRetranslation: !projection.aligned });
    } catch (failure: unknown) {
      if (mounted.current && ticket.generation === generation.current) commit({ translating: false, error: toAppError(failure) });
    }
  }, [native, cancel, commit, confirmDraftDiscard]);
  const translateClipboard = useCallback(async () => {
    if (!native) { reportError({ code: 'native_unavailable', message: 'Native clipboard is unavailable.' }); return; }
    try {
      const text = await readText();
      if (text === null) return;
      const current = stateRef.current;
      if (current.draftDirty && !(await confirmDraftDiscard(current))) return;
      pendingDraft.current = null;
      commit({ draftDirty: false, draftEdits: emptyDraft(current.sourceRevision, current.sourceText) });
      setSourceText(text); await translate(true);
    } catch (failure: unknown) { reportError(toAppError(failure)); }
  }, [native, reportError, commit, setSourceText, translate, confirmDraftDiscard]);
  const chooseMeaning = useCallback(async (id: string, meaning: string) => {
    const current = stateRef.current;
    if (!current.result || current.result.sourceRevision !== current.sourceRevision || !current.result.segments.some((segment) => segment.id === id)) return;
    let draftEdits = current.draftEdits;
    const discardStale = !projectDraft(current.sourceText, current.result, draftEdits, current.options).aligned;
    if (discardStale) {
      if (!(await confirmDraftDiscard(current))) return;
      pendingDraft.current = null;
      draftEdits = emptyDraft(current.sourceRevision, current.sourceText);
    }
    commit({ draftEdits: { ...draftEdits, sourceRevision: current.sourceRevision, sourceText: current.sourceText, overrides: { ...draftEdits.overrides, [id]: meaning } },
      draftDirty: true, dirty: true, ...(discardStale ? { needsRetranslation: false } : {}) });
  }, [commit, confirmDraftDiscard]);
  const dropToken = useCallback((id: string, targetId: string) => {
    const current = stateRef.current;
    if (!current.result || current.result.sourceRevision !== current.sourceRevision) return;
    const draftEdits = moveDraftToken(current.sourceText, current.result, current.draftEdits, id, targetId);
    if (draftEdits !== current.draftEdits) {
      commit({ draftEdits, draftDirty: true, dirty: true });
      selectSegment(id);
    }
  }, [commit, selectSegment]);
  const moveToken = useCallback((id: string, direction: -1 | 1) => {
    const current = stateRef.current;
    if (!current.result) return;
    const tokens = orderedTokens(current.result, current.draftEdits);
    const index = tokens.findIndex((token) => token.id === id);
    const target = tokens[index + direction];
    if (index >= 0 && target) dropToken(id, target.id);
  }, [dropToken]);
  const setOptions = useCallback((options: TranslationOptions) => {
    cancel(); commit({ options, needsRetranslation: true, dirty: true }); observeSource();
  }, [cancel, commit, observeSource]);
  const setViewState = useCallback((viewState: WorkspaceViewState) => { commit({ viewState }); }, [commit]);
  const loadPreferences = useCallback((viewState: WorkspaceViewState, options: TranslationOptions) => {
    commit({ viewState: structuredClone(viewState), options: { ...options } }); observeSource();
  }, [commit, observeSource]);
  const registerView = useCallback((pane: AlignedPane, view: EditorView | null) => {
    if (pane === 'source') {
      const previous = views.current.source;
      if (!view && previous) sourceCache.current = { epoch: sourceViewEpoch.current, state: previous.state };
      if (view) sourceViewEpoch.current = stateRef.current.editorEpoch;
      setSourceView(view);
      if (!view && stateRef.current.composing) commit({ composing: false });
    }
    if (view) views.current[pane] = view; else delete views.current[pane];
  }, [commit]);
  const getSourceEditorState = useCallback(() => sourceCache.current?.epoch === stateRef.current.editorEpoch ? sourceCache.current.state : null, []);
  const syncScroll = useCallback((pane: AlignedPane, offset: number) => {
    const current = stateRef.current;
    if (!current.viewState.autoScroll || performance.now() < scrollGuard.current || current.result?.sourceRevision !== current.sourceRevision) return;
    const projection = projectDraft(current.sourceText, current.result, current.draftEdits, current.options);
    if (pane === 'singleMeaning' && !projection.aligned) return;
    const rangeFor = (segment: TranslationResult['segments'][number], kind: AlignedPane) => kind === 'source' ? segment.sourceRange
      : kind === 'readings' ? segment.readingsRange : kind === 'phrases' ? segment.phrasesRange : projection.ranges.get(segment.id) ?? segment.singleMeaningRange;
    const segments = [...current.result.segments].sort((a, b) => rangeFor(a, pane).start - rangeFor(b, pane).start);
    const segment = segments.find((item) => rangeFor(item, pane).end > offset);
    if (!segment) return;
    scrollGuard.current = performance.now() + 160;
    for (const [kind, view] of Object.entries(views.current) as [AlignedPane, EditorView][]) {
      if (kind === pane || view.dom.clientHeight === 0 || (kind === 'singleMeaning' && !projection.aligned)) continue;
      const range = rangeFor(segment, kind);
      if (range.start <= view.state.doc.length) view.dispatch({ effects: EditorView.scrollIntoView(range.start, { y: 'start' }) });
    }
  }, []);
  const captureTarget = useCallback((): TargetCapture | null => {
    const editor = targetRef.current;
    if (!editor) return null;
    const selection = (bookmark.current ?? editor.state.selection.getBookmark()).resolve(editor.state.doc);
    return { bookmark: selection.getBookmark(), documentId: stateRef.current.documentId, sourceRevision: stateRef.current.sourceRevision,
      targetRevision: stateRef.current.targetRevision, from: selection.from, to: selection.to,
      text: editor.state.doc.textBetween(selection.from, selection.to, '\n') };
  }, []);
  const writeTarget = useCallback((text: string, capture: TargetCapture | null, wholeDocument: boolean, guarded: boolean): boolean => {
    const editor = targetRef.current; const current = stateRef.current;
    if (!editor || !capture || (guarded && !canApplyTarget(current, capture))) return false;
    const selection = capture.bookmark.resolve(editor.state.doc);
    editor.view.dispatch(closeHistory(editor.state.tr));
    let success: boolean;
    if (wholeDocument) success = editor.commands.setContent({ type: 'doc', content: textContent(text) });
    else if (text.length === 0) success = editor.chain().setTextSelection({ from: selection.from, to: selection.to }).deleteSelection().run();
    else success = editor.chain().setTextSelection({ from: selection.from, to: selection.to }).insertContent(text.includes('\n') ? textContent(text) : { type: 'text', text }).run();
    editor.view.dispatch(closeHistory(editor.state.tr));
    editor.commands.focus();
    bookmark.current = editor.state.selection.getBookmark();
    return success;
  }, []);
  const insertTarget = useCallback((text: string) => writeTarget(text, captureTarget(), false, false), [writeTarget, captureTarget]);
  const applyTarget = useCallback((text: string, capture: TargetCapture, wholeDocument = false) => writeTarget(text, capture, wholeDocument, true), [writeTarget]);
  const getTargetJSON = useCallback(() => targetRef.current?.getJSON() ?? stateRef.current.targetDocument, []);
  const serialize = useCallback((): WorkspaceDocument => {
    const current = stateRef.current;
    const preview = projectDraft(current.draftDirty ? current.draftEdits.sourceText : current.sourceText, current.result, current.draftEdits, current.options);
    return { sourceLanguage: current.sourceLanguage, sourceText: current.sourceText,
      targetDocument: getTargetJSON(), draftEdits: { ...structuredClone(current.draftEdits), previewText: preview.text }, viewState: structuredClone(current.viewState) };
  }, [getTargetJSON]);
  const restore = useCallback((document: WorkspaceDocument) => {
    const editor = targetRef.current;
    if (!editor) throw new Error('Target editor is not ready');
    // Validate content against the editor schema before changing any active state.
    editor.schema.nodeFromJSON(document.targetDocument).check();
    cancel(); restoring.current = true;
    try {
      editor.commands.setContent(document.targetDocument, { emitUpdate: false, errorOnInvalidContent: true });
      editor.view.updateState(TargetEditorState.create({ schema: editor.schema, doc: editor.state.doc, plugins: editor.state.plugins }));
    }
    finally { restoring.current = false; }
    bookmark.current = editor.state.selection.getBookmark();
    const current = stateRef.current;
    const sourceRevision = current.sourceRevision + 1; const targetRevision = current.targetRevision + 1;
    const restoredDraft = document.draftEdits && (document.draftEdits.order.length > 0 || Object.keys(document.draftEdits.overrides).length > 0) ? structuredClone(document.draftEdits) : null;
    const staleDraft = restoredDraft !== null && restoredDraft.sourceText !== document.sourceText;
    pendingDraft.current = staleDraft ? null : restoredDraft;
    commit({ sourceLanguage: document.sourceLanguage, sourceText: document.sourceText,
      sourceRevision, targetRevision, editorEpoch: current.editorEpoch + 1, targetDocument: editor.getJSON(), targetText: editor.getText({ blockSeparator: '\n' }),
      result: null, draftEdits: restoredDraft ? { ...restoredDraft, ...(staleDraft ? {} : { sourceRevision }) } : emptyDraft(sourceRevision, document.sourceText), draftDirty: restoredDraft !== null, dirty: false,
      sourceSelection: { start: 0, end: 0 }, selectedSegmentId: null, error: null, needsRetranslation: true,
      ...(document.viewState ? { viewState: structuredClone(document.viewState) } : {}) });
    observeSource();
    if (native) enqueueNative(() => invokeCommand('observe_document_target', { documentId: stateRef.current.documentId, targetRevision }));
  }, [cancel, commit, native, enqueueNative, observeSource]);
  const markSaved = useCallback(() => commit({ dirty: false }), [commit]);
  const markDirty = useCallback(() => commit({ dirty: true }), [commit]);
  const getState = useCallback(() => stateRef.current, []);
  useEffect(() => { inputs.onState?.(state); }, [state, inputs.onState]);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; generation.current += 1; if (native) void invokeCommand('cancel_offline_translation').catch(() => {}); };
  }, [native]);
  useEffect(() => {
    if (!inputs.documentId || inputs.documentId === stateRef.current.documentId) return;
    const current = stateRef.current;
    const sourceRevision = Math.max(current.sourceRevision, inputs.sourceRevision ?? 0) + 1;
    const targetRevision = Math.max(current.targetRevision, inputs.targetRevision ?? 0) + 1;
    commit({ documentId: inputs.documentId, sourceRevision, targetRevision, draftEdits: emptyDraft(sourceRevision, current.sourceText) });
    observeSource();
    if (native) enqueueNative(() => invokeCommand('observe_document_target', { documentId: stateRef.current.documentId, targetRevision }));
  }, [inputs.documentId, inputs.sourceRevision, inputs.targetRevision, commit, observeSource, native, enqueueNative]);
  useEffect(() => {
    const revision = inputs.dictionaryRevision;
    if (revision !== undefined && revision > stateRef.current.dictionaryRevision) {
      cancel(); commit({ dictionaryRevision: revision, needsRetranslation: true });
    }
  }, [inputs.dictionaryRevision, cancel, commit]);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const subscription = listen<{ revision: number }>('dictionaries-changed', ({ payload }) => {
      if (disposed || payload.revision <= stateRef.current.dictionaryRevision) return;
      cancel(); commit({ dictionaryRevision: payload.revision, needsRetranslation: true,
        ...(stateRef.current.draftDirty ? {} : { result: null }) });
    });
    void subscription.catch((failure: unknown) => { if (!disposed) reportError(toAppError(failure)); });
    return () => { disposed = true; void subscription.then((stop) => stop()).catch(() => {}); };
  }, [native, cancel, commit, reportError]);
  useEffect(() => {
    if (!state.needsRetranslation || state.composing || (state.draftDirty && pendingDraft.current === null) || !inputs.documentId) return;
    const timer = window.setTimeout(() => { void translate(false); }, 300);
    return () => window.clearTimeout(timer);
  }, [state.needsRetranslation, state.sourceRevision, state.dictionaryRevision, state.options, state.composing, state.draftDirty, inputs.documentId, inputs.japaneseAvailable, translate]);
  const draft = useMemo(() => projectDraft(state.draftDirty ? state.draftEdits.sourceText : state.sourceText, state.result, state.draftEdits, state.options), [state.sourceText, state.result, state.draftDirty, state.draftEdits, state.options]);
  const selectedIds = useMemo(() => state.result?.sourceRevision === state.sourceRevision ? state.result.segments.filter((segment) => overlaps(segment.sourceRange, state.sourceSelection) || segment.id === state.selectedSegmentId).map((segment) => segment.id) : [], [state.result, state.sourceRevision, state.sourceSelection, state.selectedSegmentId]);
  return { state, targetEditor, sourceView, draft, selectedIds, setSourceText, setSourceLanguage, setSourceSelection, setComposing,
    selectSegment, chooseMeaning, moveToken, dropToken, translate, translateClipboard, cancel, setOptions, setViewState, loadPreferences, registerView, getSourceEditorState,
    syncScroll, captureTarget, insertTarget, applyTarget, getTargetJSON, serialize, restore, markSaved, markDirty, reportError, getState, flushObservations };
}
