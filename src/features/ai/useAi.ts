import { useCallback, useEffect, useRef, useState } from 'react';
import { Channel } from '@tauri-apps/api/core';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { invokeCommand, nativeAvailable, toAppError } from '../../lib/ipc';
import type { AiEvent, AiProfile, AiTranslationRequest, AppError } from '../../lib/types';
import { validateRange } from '../../lib/offsets';
import type { TargetCapture, WorkspaceController } from '../workspace/useWorkspaceController';
import { canApplyTarget } from '../workspace/state';
import { beginAiPreview, canApplyAi, emptyAiPreview, previewText, reduceAiEvent, sourceStillCurrent } from './state';
import type { AiPreviewState } from './state';
import type { AiPayloadPreview } from './types';

interface Inputs {
  workspace: WorkspaceController;
  confirmReplaceTarget: () => Promise<boolean>;
  onError: (error: AppError) => void;
}
export interface AiController {
  state: AiPreviewState;
  payload: AiPayloadPreview | null;
  preparing: boolean;
  inFlight: boolean;
  canApply: boolean;
  canRetry: boolean;
  prepare: (profile: AiProfile, mode: AiTranslationRequest['mode'], scope: AiTranslationRequest['scope'], instructions: string) => Promise<void>;
  send: () => Promise<void>;
  cancel: () => void;
  retry: () => Promise<void>;
  apply: (wholeDocument?: boolean) => Promise<void>;
  copy: (text?: string) => Promise<void>;
}
export function useAi(inputs: Inputs): AiController {
  const latest = useRef(inputs); latest.current = inputs;
  const [state, setState] = useState(emptyAiPreview);
  const stateRef = useRef(state);
  const [payload, setPayload] = useState<AiPayloadPreview | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [inFlight, setInFlight] = useState(false);
  const capture = useRef<TargetCapture | null>(null);
  const invokedJob = useRef<string | null>(null);
  const generation = useRef(0);
  const mounted = useRef(true);
  const commit = useCallback((next: AiPreviewState) => {
    stateRef.current = next;
    if (mounted.current) setState(next);
  }, []);
  const cancel = useCallback(() => {
    generation.current += 1;
    const current = stateRef.current;
    if (current.status === 'running' || current.status === 'review') {
      commit({ ...current, status: 'cancelled' });
      if (nativeAvailable() && current.request && invokedJob.current === current.request.jobId) void invokeCommand('cancel_ai_translation', { jobId: current.request.jobId }).catch((error: unknown) => {
        if (mounted.current) latest.current.onError(toAppError(error));
      });
    }
    if (mounted.current) setPreparing(false);
  }, [commit]);
  const prepareRequest = useCallback(async (request: AiTranslationRequest, targetCapture: TargetCapture) => {
    cancel();
    setPayload(null);
    const ticket = generation.current;
    capture.current = targetCapture;
    commit({ ...emptyAiPreview(), status: 'review', request });
    setPreparing(true);
    try {
      await latest.current.workspace.flushObservations();
      if (!mounted.current || ticket !== generation.current || !sourceStillCurrent(latest.current.workspace.getState(), request)) return;
      const result = await invokeCommand<AiPayloadPreview>('preview_ai_translation', { request });
      if (!mounted.current || ticket !== generation.current) return;
      if (!sourceStillCurrent(latest.current.workspace.getState(), request)) { cancel(); return; }
      setPayload(result);
      commit({ ...stateRef.current, chunkCount: result.chunkCount });
    } catch (failure: unknown) {
      if (mounted.current && ticket === generation.current) commit({ ...stateRef.current, status: 'error', error: toAppError(failure) });
    } finally {
      if (mounted.current && ticket === generation.current) setPreparing(false);
    }
  }, [cancel, commit]);
  const prepare = useCallback(async (profile: AiProfile, mode: AiTranslationRequest['mode'], scope: AiTranslationRequest['scope'], instructions: string) => {
    if (stateRef.current.status === 'running') return;
    const workspace = latest.current.workspace;
    const snapshot = workspace.getState();
    const targetCapture = workspace.captureTarget();
    const sourceRange = mode === 'translate' && scope === 'document' ? { start: 0, end: snapshot.sourceText.length } : { ...snapshot.sourceSelection };
    const sourceText = snapshot.sourceText.slice(sourceRange.start, sourceRange.end);
    let error: AppError | null = null;
    if (!nativeAvailable()) error = { code: 'native_unavailable', message: 'AI is available only in the native desktop application.' };
    else if (!profile.model.trim()) error = { code: 'ai_model_required', message: 'Enter and save a model before sending.' };
    else if (!targetCapture) error = { code: 'ai_target_unavailable', message: 'The Vietnamese editor is not ready.' };
    else if (snapshot.composing) error = { code: 'ai_composing', message: 'Finish text composition before sending.' };
    else if (!validateRange(snapshot.sourceText, sourceRange)) error = { code: 'invalid_range', message: 'Select a valid source span.' };
    else if (mode === 'translate' && !sourceText.trim()) error = { code: 'ai_empty_source', message: 'Select source text or enter a document to translate.' };
    else if (mode === 'improve' && !targetCapture.text.trim()) error = { code: 'ai_empty_target', message: 'Select Vietnamese text to improve.' };
    if (error || !targetCapture) {
      cancel(); commit({ ...emptyAiPreview(), status: 'error', error }); return;
    }
    await prepareRequest({
      jobId: crypto.randomUUID(), documentId: snapshot.documentId, sourceRevision: snapshot.sourceRevision,
      targetRevision: snapshot.targetRevision, profileId: profile.id, mode, sourceLanguage: snapshot.sourceLanguage,
      scope: mode === 'improve' ? 'selection' : scope, sourceRange, sourceText,
      targetText: mode === 'improve' ? targetCapture.text : null, instructions,
    }, targetCapture);
  }, [cancel, commit, prepareRequest]);
  const send = useCallback(async () => {
    const current = stateRef.current;
    if (current.status !== 'review' || !current.request || !capture.current) return;
    if (!canApplyTarget(latest.current.workspace.getState(), current.request) || !sourceStillCurrent(latest.current.workspace.getState(), current.request)) { cancel(); return; }
    const request = current.request;
    const ticket = generation.current;
    // Register the channel callback before the native command can emit its first event.
    const onEvent = new Channel<AiEvent>();
    onEvent.onmessage = (event) => {
      if (!mounted.current || ticket !== generation.current) return;
      if (!sourceStillCurrent(latest.current.workspace.getState(), request)) { cancel(); return; }
      commit(reduceAiEvent(stateRef.current, event));
    };
    commit(beginAiPreview(request, current.chunkCount));
    try {
      await latest.current.workspace.flushObservations();
      if (!mounted.current || ticket !== generation.current || stateRef.current.status !== 'running') return;
      invokedJob.current = request.jobId;
      setInFlight(true);
      await invokeCommand('start_ai_translation', { request, onEvent });
    } catch (failure: unknown) {
      if (mounted.current && ticket === generation.current && stateRef.current.status === 'running') commit({ ...stateRef.current, status: 'error', error: toAppError(failure) });
    } finally {
      if (invokedJob.current === request.jobId) {
        invokedJob.current = null;
        if (mounted.current) setInFlight(false);
      }
    }
  }, [cancel, commit]);
  const retry = useCallback(async () => {
    const current = stateRef.current;
    if (!current.request || !capture.current || !['error', 'cancelled', 'completed'].includes(current.status)) return;
    const snapshot = latest.current.workspace.getState();
    if (!canApplyTarget(snapshot, current.request) || !sourceStillCurrent(snapshot, current.request)) return;
    await prepareRequest({ ...current.request, jobId: crypto.randomUUID() }, capture.current);
  }, [prepareRequest]);
  const apply = useCallback(async (wholeDocument = false) => {
    const current = stateRef.current;
    const targetCapture = capture.current;
    if (!targetCapture || !canApplyAi(current, latest.current.workspace.getState())) return;
    if (wholeDocument && !await latest.current.confirmReplaceTarget()) return;
    // Confirmation is asynchronous; check both the job and live editor revisions again.
    if (stateRef.current !== current || !canApplyAi(current, latest.current.workspace.getState())) return;
    latest.current.workspace.applyTarget(current.text, targetCapture, wholeDocument);
  }, []);
  const copy = useCallback(async (text?: string) => {
    try {
      const value = text ?? previewText(stateRef.current);
      if (nativeAvailable()) await writeText(value); else await navigator.clipboard.writeText(value);
    } catch (failure: unknown) { latest.current.onError(toAppError(failure)); }
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => { cancel(); mounted.current = false; };
  }, [cancel]);
  useEffect(() => {
    const request = stateRef.current.request;
    if (request && (stateRef.current.status === 'running' || stateRef.current.status === 'review') && !sourceStillCurrent(inputs.workspace.state, request)) cancel();
  }, [inputs.workspace.state.documentId, inputs.workspace.state.sourceRevision, inputs.workspace.state.sourceLanguage, cancel]);
  const canApply = canApplyAi(state, inputs.workspace.state);
  const canRetry = state.request !== null && ['error', 'cancelled', 'completed'].includes(state.status)
    && sourceStillCurrent(inputs.workspace.state, state.request) && canApplyTarget(inputs.workspace.state, state.request);
  return { state, payload, preparing, inFlight, canApply, canRetry, prepare, send, cancel, retry, apply, copy };
}
