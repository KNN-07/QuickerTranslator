import { useEffect, useState } from 'react';
import { nativeAvailable } from '../../lib/ipc';
import type { AiProfile, UiLocale } from '../../lib/types';
import type { WorkspaceController } from '../workspace/useWorkspaceController';
import { canApplyTarget } from '../workspace/state';
import type { AiController } from './useAi';
import type { AiProfilesController } from './useAiProfiles';
import { aiError, at } from './i18n';
import { previewText } from './state';

interface Props {
  ai: AiController;
  profiles: AiProfilesController;
  workspace: WorkspaceController;
  locale: UiLocale;
  onManageProfiles: () => void;
}
export function AiPane({ ai, profiles, workspace, locale, onManageProfiles }: Props) {
  const [profileId, setProfileId] = useState('');
  const [model, setModel] = useState('');
  const [instructions, setInstructions] = useState('');
  const [modelError, setModelError] = useState(false);
  const selected = profiles.records.find((item) => item.profile.id === profileId) ?? profiles.records[0];
  const profile = selected?.profile;
  const running = ai.state.status === 'running';
  const native = nativeAvailable();
  const busy = running || ai.preparing || ai.inFlight || profiles.busy;
  const modelSaved = !!profile && model === profile.model;
  const canPrepare = native && profiles.ready && !busy && !!profile?.model.trim() && modelSaved && !workspace.state.composing;
  const stale = !!ai.state.request && !canApplyTarget(workspace.state, ai.state.request);
  const partial = previewText(ai.state);
  const completedChunks = ai.state.chunks.filter((chunk) => chunk.completed);
  useEffect(() => {
    if (!profile) { setModel(''); return; }
    setProfileId(profile.id); setModel(profile.model); setModelError(false);
  }, [profile?.id, profile?.model]);
  const prepare = (mode: 'translate' | 'improve', scope: 'selection' | 'document') => {
    if (profile && canPrepare) void ai.prepare(profile, mode, scope, instructions);
  };
  const saveModel = async () => {
    if (!profile || !model.trim() || busy || !native) return;
    const next: AiProfile = { ...profile, model };
    try { await profiles.save(next); setModelError(false); }
    catch { setModelError(true); }
  };
  return <div className="ai-pane">
    <div className="ai-controls ai-primary-controls">
      <label><span>{at(locale, 'profile')}</span><select value={profile?.id ?? ''} disabled={!native || busy} onChange={(event) => { ai.cancel(); setProfileId(event.target.value); }}>
        {!profiles.records.length && <option value="">—</option>}{profiles.records.map((record) => <option key={record.profile.id} value={record.profile.id}>{record.profile.name}</option>)}
      </select></label>
      <label className="ai-model"><span>{at(locale, 'model')}</span><input value={model} required spellCheck={false} disabled={!native || busy || !profile} placeholder={at(locale, 'modelRequired')} onChange={(event) => { ai.cancel(); setModel(event.target.value); }} /></label>
      {!modelSaved && profile && <button type="button" disabled={!native || busy || !model.trim()} onClick={() => { void saveModel(); }}>{at(locale, 'save')}</button>}
      <button type="button" onClick={onManageProfiles}>{at(locale, 'manage')}</button>
    </div>
    {profile && selected && <div className="ai-profile-summary"><code>{selected.requestUrl}</code><span>{profile.protocol} · {at(locale, profile.stream ? 'stream' : 'nonstream')} · {profile.maxOutputTokens}</span><span>{at(locale, selected.credentialStatus.status)}</span></div>}
    {!native && <p className="pane-note">{at(locale, 'nativeOnly')}</p>}
    {!profile && <p className="pane-note">{at(locale, 'noProfiles')}</p>}
    {!modelSaved && profile && <p className="pane-note">{at(locale, 'savedRequired')}</p>}
    {(profiles.error || modelError) && <p className="workspace-error" role="alert">{profiles.error ? aiError(locale, profiles.error) : at(locale, 'genericError')}</p>}
    <div className="ai-controls">
      <button type="button" disabled={!canPrepare} onClick={() => prepare('translate', 'selection')}>{at(locale, 'translateSelection')}</button>
      <button type="button" disabled={!canPrepare} onClick={() => prepare('translate', 'document')}>{at(locale, 'translateDocument')}</button>
      <button type="button" disabled={!canPrepare} title={at(locale, 'improveHint')} onClick={() => prepare('improve', 'selection')}>{at(locale, 'improve')}</button>
      <button type="button" disabled={ai.state.status !== 'running' && ai.state.status !== 'review'} onClick={ai.cancel}>{at(locale, 'cancel')}</button>
      <button type="button" disabled={!ai.canRetry || !native || busy} title={at(locale, 'retryNotice')} onClick={() => { void ai.retry(); }}>{at(locale, 'retry')}</button>
    </div>
    <label className="ai-instructions"><span>{at(locale, 'instructions')}</span><input value={instructions} disabled={busy} placeholder={at(locale, 'instructionsHint')} onChange={(event) => { ai.cancel(); setInstructions(event.target.value); }} /></label>
    <p className="pane-note ai-privacy">{at(locale, 'privacy')}</p>
    {ai.preparing && <p role="status">{at(locale, 'preparing')}</p>}
    {ai.payload && ai.state.status === 'review' && <section className="ai-payload-review" aria-label={at(locale, 'review')}>
      <h3>{at(locale, 'review')}</h3>
      <p className="ai-endpoint"><strong>{at(locale, 'requestUrl')}:</strong> <code>{ai.payload.requestUrl}</code></p>
      <p>{at(locale, 'host')}: {ai.payload.host} · {at(locale, 'model')}: {ai.payload.model} · {at(locale, 'destination')}</p>
      <p>{at(locale, 'sourceScope')}: {at(locale, ai.payload.scope)} · {ai.payload.sourceLanguage} · UTF-16 {ai.payload.sourceRange.start}–{ai.payload.sourceRange.end} · {at(locale, 'chunks')}: {ai.payload.chunkCount}</p>
      <details open><summary>{at(locale, 'source')}</summary><pre>{ai.payload.sourceText}</pre></details>
      {ai.payload.targetText !== null && <details open><summary>{at(locale, 'selectedTarget')}</summary><pre>{ai.payload.targetText}</pre></details>}
      <details><summary>{at(locale, 'actualPayload')}</summary>{ai.payload.chunks.map((chunk) => <section key={chunk.chunkIndex}><h4>{at(locale, 'chunks')} {chunk.chunkIndex + 1} · UTF-16 {chunk.sourceRange.start}–{chunk.sourceRange.end}</h4><pre>{chunk.system}</pre><pre>{chunk.user}</pre></section>)}</details>
      <div className="ai-controls"><button type="button" disabled={!native || profiles.busy || ai.preparing || stale} onClick={() => { void ai.send(); }}>{at(locale, 'send')}</button><button type="button" onClick={ai.cancel}>{at(locale, 'cancel')}</button></div>
    </section>}
    <div className="ai-progress" role="status">
      <span>{at(locale, ai.state.status === 'review' ? 'review' : ai.state.status)}</span>
      {ai.state.chunkCount > 0 && <span>{completedChunks.length}/{ai.state.chunkCount} {at(locale, 'chunks')}{running && ai.state.chunks.length > completedChunks.length ? ` · ${ai.state.chunks.length}` : ''}</span>}
      {ai.state.usage && <span>{at(locale, 'inputTokens')}: {ai.state.usage.inputTokens ?? '—'} · {at(locale, 'outputTokens')}: {ai.state.usage.outputTokens ?? '—'}</span>}
    </div>
    {ai.payload && ai.state.request && ai.state.status !== 'review' && <p className="ai-profile-summary"><code>{ai.payload.requestUrl}</code><span>{ai.payload.model} · {at(locale, ai.payload.scope)} · UTF-16 {ai.payload.sourceRange.start}–{ai.payload.sourceRange.end}</span></p>}
    {ai.state.error && <p className="workspace-error" role="alert">{aiError(locale, ai.state.error)} <span className="error-code">({ai.state.error.code})</span>{ai.state.errorChunkIndex !== null && ` · ${at(locale, 'chunks')} ${ai.state.errorChunkIndex + 1}`}</p>}
    {stale && <p className="ai-warning">{at(locale, 'stale')}</p>}
    <div className="ai-preview" role="region" aria-label={at(locale, 'preview')} tabIndex={0} style={{ fontSize: workspace.state.viewState.fonts.target }}><pre>{partial || at(locale, 'noPreview')}</pre></div>
    <div className="ai-controls ai-apply-controls">
      <button type="button" onClick={() => { void ai.copy(); }}>{at(locale, 'copy')}</button>
      <button type="button" disabled={!ai.canApply} onClick={() => { void ai.apply(); }}>{at(locale, 'apply')}</button>
      <button type="button" disabled={!ai.canApply} onClick={() => { void ai.apply(true); }}>{at(locale, 'applyAll')}</button>
    </div>
    <p className="pane-note">{at(locale, 'incomplete')}</p>
    {completedChunks.length > 0 && <details className="ai-chunks"><summary>{at(locale, 'chunks')} ({completedChunks.length}/{ai.state.chunkCount})</summary>{completedChunks.map((chunk) => <section key={chunk.index}>
      <div className="ai-controls"><span>{at(locale, 'chunks')} {chunk.index + 1} · UTF-16 {chunk.sourceRange.start}–{chunk.sourceRange.end}</span><button type="button" onClick={() => { void ai.copy(chunk.text); }}>{at(locale, 'copyChunk')}</button></div><pre>{chunk.text}</pre>
    </section>)}</details>}
  </div>;
}
