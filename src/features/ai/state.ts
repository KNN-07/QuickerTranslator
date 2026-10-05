import type { AiEvent, AiTranslationRequest, AiUsage, AppError, TextRange } from '../../lib/types';
import { canApplyTarget } from '../workspace/state';
import type { TargetRevisions } from '../workspace/state';

export type AiStatus = 'idle' | 'review' | 'running' | 'completed' | 'cancelled' | 'error';
export interface AiChunk {
  index: number;
  sourceRange: TextRange;
  text: string;
  completed: boolean;
  usage: AiUsage | null;
}
export interface AiPreviewState {
  status: AiStatus;
  request: AiTranslationRequest | null;
  chunkCount: number;
  chunks: AiChunk[];
  text: string;
  usage: AiUsage | null;
  error: AppError | null;
  errorChunkIndex: number | null;
}
export function emptyAiPreview(): AiPreviewState {
  return { status: 'idle', request: null, chunkCount: 0, chunks: [], text: '', usage: null, error: null, errorChunkIndex: null };
}
export function beginAiPreview(request: AiTranslationRequest, chunkCount: number): AiPreviewState {
  return { ...emptyAiPreview(), status: 'running', request, chunkCount };
}
/** Terminal states are absorbing: a late native delta can never revive a cancelled job. */
export function reduceAiEvent(state: AiPreviewState, event: AiEvent): AiPreviewState {
  if (state.status !== 'running' || state.request?.jobId !== event.jobId) return state;
  switch (event.type) {
    case 'started': return { ...state, chunkCount: event.chunkCount };
    case 'chunkStarted': {
      if (state.chunks.some((chunk) => chunk.index === event.chunkIndex)) return state;
      return { ...state, chunks: [...state.chunks, { index: event.chunkIndex, sourceRange: event.sourceRange, text: '', completed: false, usage: null }] };
    }
    case 'delta': {
      const chunk = state.chunks.find((item) => item.index === event.chunkIndex);
      if (!chunk || chunk.completed) return state;
      return { ...state, chunks: state.chunks.map((item) => item === chunk ? { ...item, text: item.text + event.text } : item) };
    }
    case 'chunkCompleted': {
      const chunk = state.chunks.find((item) => item.index === event.chunkIndex);
      if (!chunk || chunk.completed) return state;
      return { ...state, chunks: state.chunks.map((item) => item === chunk ? { ...item, text: event.text, completed: true, usage: event.usage } : item) };
    }
    case 'completed': return { ...state, status: 'completed', text: event.text, usage: event.usage };
    case 'cancelled': return { ...state, status: 'cancelled' };
    case 'error': return { ...state, status: 'error', error: { code: event.code, message: event.message }, errorChunkIndex: event.chunkIndex };
  }
}
/** No invented AI word alignment: only immutable per-request/chunk source spans. */
export function previewText(state: AiPreviewState): string {
  if (state.status === 'completed') return state.text;
  const request = state.request;
  if (!request) return '';
  let text = '';
  let previousEnd: number | null = null;
  for (const chunk of [...state.chunks].sort((a, b) => a.index - b.index)) {
    if (previousEnd !== null) text += request.sourceText.slice(previousEnd - request.sourceRange.start, chunk.sourceRange.start - request.sourceRange.start);
    text += chunk.text;
    previousEnd = chunk.sourceRange.end;
  }
  return text;
}
export function sourceStillCurrent(current: TargetRevisions & { sourceLanguage: string }, request: AiTranslationRequest): boolean {
  return current.documentId === request.documentId && current.sourceRevision === request.sourceRevision && current.sourceLanguage === request.sourceLanguage;
}
export function canApplyAi(state: AiPreviewState, current: TargetRevisions & { sourceLanguage: string }): boolean {
  return state.status === 'completed' && state.request !== null && state.text.length > 0
    && sourceStillCurrent(current, state.request) && canApplyTarget(current, state.request);
}
