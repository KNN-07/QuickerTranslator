import { describe, expect, it } from 'vitest';
import { guardUnsaved, documentStamp, unchangedDocument } from './guard';
import { DEFAULT_VIEW_STATE, emptyDraft } from '../workspace/state';
import type { WorkspaceSnapshot } from '../workspace/state';
import { DEFAULT_TRANSLATION_OPTIONS } from '../../lib/types';

function documentState(): WorkspaceSnapshot {
  return { documentId: 'document', sourceLanguage: 'zh', sourceText: '你好🙂', sourceRevision: 4, targetRevision: 9,
    editorEpoch: 0, targetDocument: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Bản dịch riêng' }] }] },
    targetText: 'Bản dịch riêng', sourceSelection: { start: 0, end: 0 }, selectedSegmentId: null, result: null,
    draftEdits: emptyDraft(4, '你好🙂'), draftDirty: false, dirty: true, translating: false, composing: false, dictionaryRevision: 2,
    options: { ...DEFAULT_TRANSLATION_OPTIONS }, viewState: structuredClone(DEFAULT_VIEW_STATE), error: null, needsRetranslation: false };
}
describe('shared unsaved-document guard', () => {
  it('rejects Open / New / Close when Save As is cancelled without changing the old state', async () => {
    const state = documentState(); const before = structuredClone(state);
    expect(await guardUnsaved(() => state, async () => 'save', async () => false)).toBe('cancel');
    expect(state).toEqual(before);
  });
  it('does not treat a failed save as permission to discard work', async () => {
    const state = documentState(); const before = structuredClone(state);
    await expect(guardUnsaved(() => state, async () => 'save', async () => { throw new Error('atomic write failed'); })).rejects.toThrow('atomic write failed');
    expect(state).toEqual(before);
  });
  it('approves Discard without actually mutating the document before a successful import', async () => {
    const state = documentState(); const before = structuredClone(state);
    expect(await guardUnsaved(() => state, async () => 'discard', async () => false)).toBe('discard');
    expect(state).toEqual(before);
  });
  it('rejects a stale discard choice and a save that did not clear the current dirty revision', async () => {
    const state = documentState();
    expect(await guardUnsaved(() => state, async () => { state.targetRevision += 1; return 'discard'; }, async () => false)).toBe('cancel');
    expect(await guardUnsaved(() => state, async () => 'save', async () => true)).toBe('cancel');
  });
  it('marks only the exact saved source, target, document and draft snapshot as unchanged', () => {
    const state = documentState(); const stamp = documentStamp(state);
    expect(unchangedDocument(state, stamp)).toBe(true);
    expect(unchangedDocument({ ...state, sourceRevision: 5 }, stamp)).toBe(false);
    expect(unchangedDocument({ ...state, targetRevision: 10 }, stamp)).toBe(false);
    expect(unchangedDocument({ ...state, documentId: 'other-window' }, stamp)).toBe(false);
    expect(unchangedDocument({ ...state, draftDirty: true, draftEdits: { ...state.draftEdits, overrides: { token: 'nghĩa mới' } } }, stamp)).toBe(false);
  });
});
