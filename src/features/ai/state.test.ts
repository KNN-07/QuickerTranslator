import { describe, expect, it } from 'vitest';
import type { AiTranslationRequest } from '../../lib/types';
import { beginAiPreview, canApplyAi, previewText, reduceAiEvent, sourceStillCurrent } from './state';

const request: AiTranslationRequest = {
  jobId: 'job-1', documentId: 'document-1', sourceRevision: 7, targetRevision: 9, profileId: 'profile-1',
  mode: 'translate', scope: 'document', sourceLanguage: 'zh', sourceRange: { start: 0, end: 10 },
  sourceText: '你好。\n\n世界。🙂', targetText: null, instructions: '',
};
const current = { documentId: request.documentId, sourceRevision: request.sourceRevision, targetRevision: request.targetRevision, sourceLanguage: request.sourceLanguage };

describe('AI preview event lifecycle', () => {
  it('isolates job IDs and makes success, error and cancellation absorbing terminals', () => {
    const started = beginAiPreview(request, 1);
    expect(reduceAiEvent(started, { type: 'delta', jobId: 'another-window-job', chunkIndex: 0, text: 'late' })).toBe(started);
    for (const terminal of [
      { type: 'completed', jobId: request.jobId, text: 'Xin chào.', usage: null } as const,
      { type: 'cancelled', jobId: request.jobId } as const,
      { type: 'error', jobId: request.jobId, code: 'ai_interrupted', message: 'Incomplete.', chunkIndex: 0 } as const,
    ]) {
      const stopped = reduceAiEvent(started, terminal);
      expect(reduceAiEvent(stopped, { type: 'chunkStarted', jobId: request.jobId, chunkIndex: 0, sourceRange: request.sourceRange })).toBe(stopped);
      expect(reduceAiEvent(stopped, { type: 'delta', jobId: request.jobId, chunkIndex: 0, text: 'late' })).toBe(stopped);
      expect(reduceAiEvent(stopped, { type: 'completed', jobId: request.jobId, text: 'late success', usage: null })).toBe(stopped);
    }
  });
  it('keeps incomplete text and completed chunks copyable without enabling apply', () => {
    let state = beginAiPreview(request, 2);
    state = reduceAiEvent(state, { type: 'chunkStarted', jobId: request.jobId, chunkIndex: 0, sourceRange: { start: 0, end: 3 } });
    state = reduceAiEvent(state, { type: 'delta', jobId: request.jobId, chunkIndex: 0, text: 'Xin ' });
    state = reduceAiEvent(state, { type: 'chunkCompleted', jobId: request.jobId, chunkIndex: 0, text: 'Xin chào.', usage: { inputTokens: 5, outputTokens: null } });
    state = reduceAiEvent(state, { type: 'chunkStarted', jobId: request.jobId, chunkIndex: 1, sourceRange: { start: 5, end: 10 } });
    state = reduceAiEvent(state, { type: 'delta', jobId: request.jobId, chunkIndex: 1, text: 'Thế ' });
    state = reduceAiEvent(state, { type: 'error', jobId: request.jobId, code: 'ai_interrupted', message: 'Incomplete.', chunkIndex: 1 });
    expect(previewText(state)).toBe('Xin chào.\n\nThế ');
    expect(state.chunks[0]?.text).toBe('Xin chào.');
    expect(state.chunks[0]?.completed).toBe(true);
    expect(canApplyAi(state, current)).toBe(false);
  });
  it('accepts authoritative completed text including preserved delimiters only for current source and target', () => {
    const completed = reduceAiEvent(beginAiPreview(request, 2), { type: 'completed', jobId: request.jobId, text: 'Xin chào.\n\nThế giới.🙂', usage: { inputTokens: null, outputTokens: 14 } });
    expect(previewText(completed)).toBe('Xin chào.\n\nThế giới.🙂');
    expect(canApplyAi(completed, current)).toBe(true);
    expect(canApplyAi(completed, { ...current, targetRevision: current.targetRevision + 1 })).toBe(false);
    expect(canApplyAi(completed, { ...current, sourceRevision: current.sourceRevision + 1 })).toBe(false);
    expect(canApplyAi(completed, { ...current, sourceLanguage: 'ja' })).toBe(false);
    expect(canApplyAi(completed, { ...current, documentId: 'document-2' })).toBe(false);
    expect(sourceStillCurrent({ ...current, targetRevision: current.targetRevision + 1 }, request)).toBe(true);
  });
  it('does not append duplicate completion / late deltas to completed chunks', () => {
    let state = beginAiPreview(request, 1);
    state = reduceAiEvent(state, { type: 'chunkStarted', jobId: request.jobId, chunkIndex: 0, sourceRange: request.sourceRange });
    state = reduceAiEvent(state, { type: 'chunkCompleted', jobId: request.jobId, chunkIndex: 0, text: 'Xin chào.', usage: null });
    expect(reduceAiEvent(state, { type: 'delta', jobId: request.jobId, chunkIndex: 0, text: 'late' })).toBe(state);
    expect(reduceAiEvent(state, { type: 'chunkCompleted', jobId: request.jobId, chunkIndex: 0, text: 'duplicated', usage: null })).toBe(state);
  });
});
