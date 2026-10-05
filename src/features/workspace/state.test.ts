import { describe, expect, it } from 'vitest';
import { DEFAULT_TRANSLATION_OPTIONS } from '../../lib/types';
import type { TranslationResult, TranslationSegment } from '../../lib/types';
import { acceptsTranslation, canApplyTarget, DEFAULT_VIEW_STATE, emptyDraft, moveDraftToken, projectDraft } from './state';
import type { TranslationTicket, WorkspaceSnapshot } from './state';

const source = '甲乙。\n丙丁。🙂';
const output = 'Một hai.\nBa bốn.🙂';
function segment(id: string, start: number, end: number, outputStart: number, outputEnd: number, meanings: string[] = []): TranslationSegment {
  return { id, sourceRange: { start, end }, readingsRange: { start: outputStart, end: outputEnd }, phrasesRange: { start: outputStart, end: outputEnd },
    singleMeaningRange: { start: outputStart, end: outputEnd }, surface: source.slice(start, end), lemma: null, reading: null, partOfSpeech: null,
    meanings, provenance: [], unknown: meanings.length === 0 && !/^[\p{P}\s]+$/u.test(source.slice(start, end)) };
}
const result: TranslationResult = {
  documentId: 'document', sourceRevision: 4, dictionaryRevision: 8, readings: output, phrases: output, singleMeaning: output,
  segments: [segment('a', 0, 1, 0, 3, ['Một']), segment('b', 1, 2, 4, 7, ['hai', 'thứ hai']), segment('period1', 2, 3, 7, 8),
    segment('newline', 3, 4, 8, 9), segment('c', 4, 5, 9, 11, ['Ba']), segment('d', 5, 6, 12, 15, ['bốn']),
    segment('period2', 6, 7, 15, 16), segment('emoji', 7, 9, 16, 18)],
};
function snapshot(): WorkspaceSnapshot {
  return { documentId: 'document', sourceLanguage: 'zh', sourceText: source, sourceRevision: 4, targetRevision: 10, editorEpoch: 0,
    targetDocument: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Bản dịch tự viết' }] }] },
    targetText: 'Bản dịch tự viết', sourceSelection: { start: 0, end: 0 }, selectedSegmentId: null, result: null,
    draftEdits: emptyDraft(4, source), draftDirty: false, dirty: false, translating: true, composing: false,
    dictionaryRevision: 8, options: { ...DEFAULT_TRANSLATION_OPTIONS }, viewState: structuredClone(DEFAULT_VIEW_STATE), error: null, needsRetranslation: false };
}
const ticket: TranslationTicket = { generation: 3, documentId: 'document', sourceRevision: 4, sourceLanguage: 'zh', dictionaryRevision: 8 };

describe('immutable offline result guards', () => {
  it('accepts only the current source, language, job, document and dictionary snapshot', () => {
    const state = snapshot();
    expect(acceptsTranslation(state, ticket, 3, result)).toBe(true);
    expect(acceptsTranslation(state, ticket, 4, result)).toBe(false);
    expect(acceptsTranslation({ ...state, sourceRevision: 5 }, ticket, 3, result)).toBe(false);
    expect(acceptsTranslation({ ...state, sourceLanguage: 'ja' }, ticket, 3, result)).toBe(false);
    expect(acceptsTranslation({ ...state, dictionaryRevision: 9 }, ticket, 3, result)).toBe(false);
    expect(acceptsTranslation({ ...state, documentId: 'other-window' }, ticket, 3, result)).toBe(false);
    expect(acceptsTranslation(state, ticket, 3, { ...result, sourceRevision: 3 })).toBe(false);
    expect(acceptsTranslation(state, ticket, 3, { ...result, dictionaryRevision: 7 })).toBe(false);
    expect(state.targetText).toBe('Bản dịch tự viết');
  });
  it('rejects target application after either editor changes or another window captures it', () => {
    const current = snapshot();
    const captured = { documentId: current.documentId, sourceRevision: current.sourceRevision, targetRevision: current.targetRevision };
    expect(canApplyTarget(current, captured)).toBe(true);
    expect(canApplyTarget({ ...current, sourceRevision: 5 }, captured)).toBe(false);
    expect(canApplyTarget({ ...current, targetRevision: 11 }, captured)).toBe(false);
    expect(canApplyTarget({ ...current, documentId: 'other' }, captured)).toBe(false);
  });
});
describe('first-meaning draft identity and paragraph boundaries', () => {
  it('moves text and attached source IDs together, preserving punctuation, paragraphs and emoji offsets', () => {
    const draft = moveDraftToken(source, result, emptyDraft(4, source), 'b', 'a');
    const projection = projectDraft(source, result, draft, DEFAULT_TRANSLATION_OPTIONS);
    expect(projection.text).toBe('hai Một.\nBa bốn.🙂');
    expect(projection.ranges.get('b')).toEqual({ start: 0, end: 3 });
    expect(projection.ranges.get('a')).toEqual({ start: 4, end: 7 });
    expect(projection.ranges.get('emoji')).toEqual({ start: 16, end: 18 });
    expect(result.segments[1]?.sourceRange).toEqual({ start: 1, end: 2 });
    expect(result.singleMeaning).toBe(output);
  });
  it('refuses cross-paragraph movement and movement of non-token punctuation', () => {
    const draft = emptyDraft(4, source);
    expect(moveDraftToken(source, result, draft, 'b', 'c')).toBe(draft);
    expect(moveDraftToken(source, result, draft, 'period1', 'a')).toBe(draft);
    expect(moveDraftToken(source, result, draft, 'emoji', 'd')).toBe(draft);
  });
  it('chooses a meaning independently of the generated baseline and remaps following UTF-16 ranges', () => {
    const draft = { ...emptyDraft(4, source), overrides: { b: 'thứ hai' } };
    const projection = projectDraft(source, result, draft, { ...DEFAULT_TRANSLATION_OPTIONS, singleWrap: 'all' });
    expect(projection.text).toBe('Một [thứ hai].\nBa bốn.🙂');
    expect(projection.ranges.get('b')).toEqual({ start: 4, end: 13 });
    expect(projection.ranges.get('emoji')).toEqual({ start: 22, end: 24 });
    expect(result.singleMeaning).toBe(output);
  });
  it('does not attach edits from a different source revision to a new result', () => {
    const oldDraft = { ...emptyDraft(3, '旧'), overrides: { b: 'không được áp dụng' } };
    expect(projectDraft(source, result, oldDraft, DEFAULT_TRANSLATION_OPTIONS).text).toBe(output);
    const wrongSource = { ...oldDraft, sourceRevision: result.sourceRevision };
    expect(projectDraft(source, result, wrongSource, DEFAULT_TRANSLATION_OPTIONS).text).toBe(output);
    expect(moveDraftToken(source, result, wrongSource, 'b', 'a')).toBe(wrongSource);
  });
  it('retains a saved stale draft for copying without inventing alignment to edited source', () => {
    const stale = { ...emptyDraft(3, '旧'), overrides: { old: 'bản nháp' }, previewText: 'Bản nháp đã lưu.🙂' };
    const projection = projectDraft(source, null, stale, DEFAULT_TRANSLATION_OPTIONS);
    expect(projection.text).toBe('Bản nháp đã lưu.🙂');
    expect(projection.ranges.size).toBe(0);
    expect(projection.aligned).toBe(false);
  });
  it('preserves an actual saved preview if dictionary changes removed the saved token IDs', () => {
    const saved = { ...emptyDraft(result.sourceRevision, source), overrides: { removed: 'nghĩa đã chọn' }, previewText: 'Bản nháp từ từ điển cũ.' };
    const projection = projectDraft(source, result, saved, DEFAULT_TRANSLATION_OPTIONS);
    expect(projection.text).toBe(saved.previewText);
    expect(projection.aligned).toBe(false);
    expect(projection.ranges.size).toBe(0);
  });
});
