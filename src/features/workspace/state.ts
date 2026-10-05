import type { JSONContent } from '@tiptap/core';
import type { AppError, DictionaryKind, DictionaryProvenance, SourceLanguage, TextRange, TranslationOptions, TranslationResult, TranslationSegment } from '../../lib/types';
import { validateRange } from '../../lib/offsets';

export interface DictionaryLookupEntry {
  headword: string;
  kind: DictionaryKind;
  meanings: string[];
  reading: string | null;
  partOfSpeech: string | null;
  provenance: DictionaryProvenance[];
}
export interface DraftEdits {
  sourceRevision: number;
  /** Exact source snapshot; stale drafts never attach to a different document text. */
  sourceText: string;
  overrides: Record<string, string>;
  order: string[];
  /** Saved rendered draft for copying when its source snapshot has become stale. */
  previewText?: string;
}
export interface WorkspaceViewState {
  autoScroll: boolean;
  wrap: boolean;
  fonts: { source: number; readings: number; phrases: number; singleMeaning: number; meanings: number; target: number };
  wraps?: { source: boolean; readings: boolean; phrases: boolean; singleMeaning: boolean; meanings: boolean; target: boolean };
  legacyScrollIndices?: [number, number, number];
}
export const DEFAULT_VIEW_STATE: WorkspaceViewState = {
  autoScroll: false, wrap: true,
  fonts: { source: 18, readings: 16, phrases: 16, singleMeaning: 16, meanings: 14, target: 16 },
};
export interface WorkspaceSnapshot {
  documentId: string;
  sourceLanguage: SourceLanguage;
  sourceText: string;
  sourceRevision: number;
  targetRevision: number;
  editorEpoch: number;
  targetDocument: JSONContent;
  targetText: string;
  sourceSelection: TextRange;
  selectedSegmentId: string | null;
  result: TranslationResult | null;
  draftEdits: DraftEdits;
  draftDirty: boolean;
  dirty: boolean;
  translating: boolean;
  composing: boolean;
  dictionaryRevision: number;
  options: TranslationOptions;
  viewState: WorkspaceViewState;
  error: AppError | null;
  needsRetranslation: boolean;
}
export interface WorkspaceDocument {
  sourceLanguage: SourceLanguage;
  sourceText: string;
  targetDocument: JSONContent;
  draftEdits?: DraftEdits;
  viewState?: WorkspaceViewState;
}
export interface TranslationTicket {
  generation: number;
  documentId: string;
  sourceRevision: number;
  sourceLanguage: SourceLanguage;
  dictionaryRevision: number;
}
export interface TargetRevisions {
  documentId: string;
  sourceRevision: number;
  targetRevision: number;
}
export function canApplyTarget(current: TargetRevisions, captured: TargetRevisions): boolean {
  return current.documentId === captured.documentId && current.sourceRevision === captured.sourceRevision && current.targetRevision === captured.targetRevision;
}
export function acceptsTranslation(state: WorkspaceSnapshot, ticket: TranslationTicket, generation: number, result: TranslationResult): boolean {
  return ticket.generation === generation && ticket.documentId === state.documentId
    && result.documentId === state.documentId && ticket.sourceRevision === state.sourceRevision
    && result.sourceRevision === state.sourceRevision && ticket.sourceLanguage === state.sourceLanguage
    && result.dictionaryRevision >= state.dictionaryRevision && result.dictionaryRevision >= ticket.dictionaryRevision;
}
export function emptyDraft(sourceRevision: number, sourceText: string): DraftEdits {
  return { sourceRevision, sourceText, overrides: {}, order: [] };
}
export function paragraphAt(source: string, segment: TranslationSegment): number {
  let paragraph = 0;
  for (let index = 0; index < segment.sourceRange.start; index += 1) if (source[index] === '\n') paragraph += 1;
  return paragraph;
}
export function reorderable(result: TranslationResult, segment: TranslationSegment): boolean {
  const value = result.singleMeaning.slice(segment.singleMeaningRange.start, segment.singleMeaningRange.end);
  return value.length > 0 && !/^[\p{P}\p{S}\p{M}\p{Cf}\s]+$/u.test(value) && !/[\r\n]/u.test(segment.surface);
}
export function orderedTokens(result: TranslationResult, draft: DraftEdits): TranslationSegment[] {
  const tokens = result.segments.filter((segment) => reorderable(result, segment));
  const byId = new Map(tokens.map((token) => [token.id, token]));
  const seen = new Set<string>();
  const ordered: TranslationSegment[] = [];
  for (const id of draft.order) {
    const token = byId.get(id);
    if (token && !seen.has(id)) { ordered.push(token); seen.add(id); }
  }
  for (const token of tokens) if (!seen.has(token.id)) ordered.push(token);
  return ordered;
}
/** Moves only dictionary draft tokens in the same source paragraph. IDs travel with text. */
export function moveDraftToken(source: string, result: TranslationResult, draft: DraftEdits, id: string, targetId: string): DraftEdits {
  if (draft.sourceRevision !== result.sourceRevision || draft.sourceText !== source) return draft;
  const tokens = orderedTokens(result, draft);
  const from = tokens.findIndex((token) => token.id === id);
  const to = tokens.findIndex((token) => token.id === targetId);
  if (from < 0 || to < 0 || from === to || paragraphAt(source, tokens[from]!) !== paragraphAt(source, tokens[to]!)) return draft;
  const order = tokens.map((token) => token.id);
  order.splice(to, 0, order.splice(from, 1)[0]!);
  return { ...draft, order };
}
export interface DraftProjection { text: string; ranges: Map<string, TextRange>; aligned: boolean }
/** Keep original separators and punctuation slots, replacing only reorderable token slots. */
export function projectDraft(source: string, result: TranslationResult | null, draft: DraftEdits, options: TranslationOptions): DraftProjection {
  if (!result) return { text: draft.previewText ?? '', ranges: new Map(), aligned: false };
  if (draft.sourceRevision !== result.sourceRevision || draft.sourceText !== source || (draft.order.length === 0 && Object.keys(draft.overrides).length === 0)) {
    return { text: result.singleMeaning, ranges: new Map(result.segments.map((segment) => [segment.id, segment.singleMeaningRange])), aligned: true };
  }
  const ids = new Set(result.segments.map((segment) => segment.id));
  if (draft.previewText !== undefined && [...draft.order, ...Object.keys(draft.overrides)].some((id) => !ids.has(id))) {
    return { text: draft.previewText, ranges: new Map(), aligned: false };
  }
  const groups = new Map<number, TranslationSegment[]>();
  for (const token of orderedTokens(result, draft)) {
    const paragraph = paragraphAt(source, token);
    const group = groups.get(paragraph) ?? [];
    group.push(token); groups.set(paragraph, group);
  }
  const cursors = new Map<number, number>();
  const ranges = new Map<string, TextRange>();
  let text = ''; let cursor = 0;
  for (const slot of result.segments) {
    const range = slot.singleMeaningRange;
    if (!validateRange(result.singleMeaning, range) || range.start < cursor) continue;
    text += result.singleMeaning.slice(cursor, range.start);
    let token = slot;
    if (reorderable(result, slot)) {
      const paragraph = paragraphAt(source, slot);
      const index = cursors.get(paragraph) ?? 0;
      token = groups.get(paragraph)?.[index] ?? slot;
      cursors.set(paragraph, index + 1);
    }
    let value = result.singleMeaning.slice(token.singleMeaningRange.start, token.singleMeaningRange.end);
    const override = draft.overrides[token.id];
    if (override !== undefined) value = options.singleWrap === 'all' ? `[${override}]` : override;
    const start = text.length;
    text += value;
    ranges.set(token.id, { start, end: text.length });
    cursor = range.end;
  }
  text += result.singleMeaning.slice(cursor);
  return { text, ranges, aligned: true };
}
