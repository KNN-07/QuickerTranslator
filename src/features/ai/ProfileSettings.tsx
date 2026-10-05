import { useEffect, useRef, useState } from 'react';
import { nativeAvailable, toAppError } from '../../lib/ipc';
import type { AiProfile, AiProtocol, AppError, UiLocale } from '../../lib/types';
import type { AiProfilesController } from './useAiProfiles';
import type { AiConnectionTest, AiProfilePreview, CredentialStorage } from './profiles';
import { aiError, at } from './i18n';

const protocolNames: Record<AiProtocol, string> = {
  'openai-responses': 'OpenAI Responses', 'openai-chat': 'OpenAI-compatible Chat', gemini: 'Gemini', anthropic: 'Anthropic Messages',
};
interface Props { profiles: AiProfilesController; locale: UiLocale; active: boolean; aiBusy?: boolean }
export function ProfileSettings({ profiles, locale, active, aiBusy = false }: Props) {
  const [selectedId, setSelectedId] = useState('');
  const [draft, setDraft] = useState<AiProfile | null>(null);
  const [urlPreview, setUrlPreview] = useState<AiProfilePreview | null>(null);
  const [previewDraft, setPreviewDraft] = useState<AiProfile | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [key, setKey] = useState('');
  const [storage, setStorage] = useState<CredentialStorage>('keychain');
  const [deleting, setDeleting] = useState(false);
  const [testReview, setTestReview] = useState(false);
  const [testResult, setTestResult] = useState<AiConnectionTest | null>(null);
  const urlGeneration = useRef(0);
  const selected = profiles.records.find((item) => item.profile.id === selectedId);
  const savedDraft = !!draft && !!selected && JSON.stringify(draft) === JSON.stringify(selected.profile);
  const disabled = !nativeAvailable() || profiles.busy || aiBusy;
  const validModel = !!draft?.model.trim();
  const currentUrl = previewDraft === draft ? urlPreview : null;
  useEffect(() => {
    if (!active) { setKey(''); setTestReview(false); setDeleting(false); profiles.cancelTest(); }
  }, [active, profiles.cancelTest]);
  useEffect(() => {
    const first = profiles.records[0];
    if (!draft && first) {
      setSelectedId(first.profile.id);
      setDraft(structuredClone(first.profile));
    }
  }, [draft, profiles.records]);
  useEffect(() => {
    setUrlPreview(null);
    if (!active || !draft || !nativeAvailable()) return;
    const ticket = ++urlGeneration.current;
    const timer = window.setTimeout(() => {
      void profiles.preview(draft).then((value) => {
        if (ticket !== urlGeneration.current) return;
        setUrlPreview(value); setPreviewDraft(draft); setError(null);
      }).catch((failure: unknown) => { if (ticket === urlGeneration.current) setError(toAppError(failure)); });
    }, 220);
    return () => { urlGeneration.current += 1; window.clearTimeout(timer); };
  }, [active, draft, profiles.preview]);
  const select = (id: string) => {
    setKey(''); setStorage('keychain'); setError(null); setTestResult(null); setTestReview(false); setDeleting(false); setSelectedId(id);
    setDraft(structuredClone(profiles.records.find((item) => item.profile.id === id)?.profile ?? null));
  };
  const create = (preset?: AiProfile) => {
    setKey(''); setStorage('keychain'); setError(null); setTestResult(null); setTestReview(false); setDeleting(false); setSelectedId('');
    setDraft(preset ? { ...preset, id: crypto.randomUUID(), model: '' } : {
      id: crypto.randomUUID(), name: at(locale, 'newProfile'), protocol: 'openai-chat', baseUrl: 'https://api.openai.com/v1',
      model: '', stream: true, maxOutputTokens: 4096, authMode: 'apiKey', allowInsecureHttp: false, tokenLimitField: 'max_tokens',
    });
  };
  const save = async () => {
    if (!draft || !currentUrl || !validModel || disabled) return;
    setError(null); setTestReview(false); setTestResult(null);
    try {
      const record = await profiles.save(draft);
      setSelectedId(record.profile.id); setDraft(record.profile);
    } catch (failure: unknown) { setError(toAppError(failure)); }
  };
  const saveKey = async () => {
    if (!draft || !savedDraft || !currentUrl || !key || disabled) return;
    setError(null);
    try { await profiles.setCredential(draft.id, currentUrl.endpointIdentity, key, storage); }
    catch (failure: unknown) { setError(toAppError(failure)); }
    finally { setKey(''); }
  };
  const runTest = async () => {
    if (!draft || !savedDraft || disabled) return;
    setError(null); setTestReview(false); setTestResult(null);
    try { setTestResult(await profiles.test(draft.id)); }
    catch (failure: unknown) { setError(toAppError(failure)); }
  };
  const remove = async () => {
    if (!selected || disabled) return;
    setKey('');
    try { await profiles.remove(selected.profile.id); setSelectedId(''); setDraft(null); setDeleting(false); setTestResult(null); }
    catch (failure: unknown) { setError(toAppError(failure)); }
  };
  return <section className="ai-profile-settings" aria-label={at(locale, 'profiles')}>
    <h3>{at(locale, 'profiles')}</h3>
    {!nativeAvailable() && <p className="pane-note">{at(locale, 'nativeOnly')}</p>}
    <div className="ai-controls">
      <label><span>{at(locale, 'profile')}</span><select value={selectedId} disabled={disabled} onChange={(event) => select(event.target.value)}>
        <option value="">{at(locale, 'newProfile')}</option>{profiles.records.map((item) => <option key={item.profile.id} value={item.profile.id}>{item.profile.name}</option>)}
      </select></label>
      <button type="button" disabled={disabled} onClick={() => create()}>{at(locale, 'newProfile')}</button>
      <label><span>{at(locale, 'preset')}</span><select value="" disabled={disabled} onChange={(event) => {
        const preset = profiles.presets.find((item) => item.id === event.target.value); if (preset) create(preset);
      }}><option value="">—</option>{profiles.presets.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
    </div>
    {!draft && <p className="pane-note">{at(locale, 'noProfiles')}</p>}
    {draft && <>
      <fieldset className="ai-profile-fields" disabled={disabled}>
        <label><span>{at(locale, 'name')}</span><input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></label>
        <label><span>{at(locale, 'protocol')}</span><select value={draft.protocol} onChange={(event) => {
          const protocol = event.target.value as AiProtocol;
          if (protocol === 'openai-chat') setDraft({ ...draft, protocol, tokenLimitField: 'max_tokens' });
          else {
            const { tokenLimitField, ...fields } = draft;
            void tokenLimitField;
            setDraft({ ...fields, protocol });
          }
          setKey(''); setTestReview(false);
        }}>{Object.entries(protocolNames).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
        <label className="ai-field-wide"><span>{at(locale, 'baseUrl')}</span><input type="url" value={draft.baseUrl} spellCheck={false} onChange={(event) => { setDraft({ ...draft, baseUrl: event.target.value }); setKey(''); setTestReview(false); }} /></label>
        <label><span>{at(locale, 'model')}</span><input value={draft.model} required placeholder={at(locale, 'modelRequired')} spellCheck={false} onChange={(event) => setDraft({ ...draft, model: event.target.value })} /></label>
        <label><span>{at(locale, 'maxTokens')}</span><input type="number" min={1} max={2147483647} step={1} value={draft.maxOutputTokens} onChange={(event) => setDraft({ ...draft, maxOutputTokens: Number(event.target.value) })} /></label>
        <label><span>{at(locale, 'auth')}</span><select value={draft.authMode} onChange={(event) => { setDraft({ ...draft, authMode: event.target.value === 'none' ? 'none' : 'apiKey' }); setKey(''); }}><option value="apiKey">{at(locale, 'apiKey')}</option><option value="none">{at(locale, 'noAuth')}</option></select></label>
        <label className="ai-checkbox"><input type="checkbox" checked={draft.stream} onChange={(event) => setDraft({ ...draft, stream: event.target.checked })} />{at(locale, 'stream')}</label>
        {draft.protocol === 'openai-chat' && <label><span>{at(locale, 'tokenField')}</span><select value={draft.tokenLimitField} onChange={(event) => {
          const tokenLimitField = event.target.value === 'max_completion_tokens' ? 'max_completion_tokens' : event.target.value === 'omit' ? 'omit' : 'max_tokens';
          setDraft({ ...draft, protocol: 'openai-chat', tokenLimitField });
        }}><option value="max_tokens">max_tokens</option><option value="max_completion_tokens">max_completion_tokens</option><option value="omit">omit</option></select></label>}
        <label className="ai-checkbox ai-field-wide"><input type="checkbox" checked={draft.allowInsecureHttp} onChange={(event) => setDraft({ ...draft, allowInsecureHttp: event.target.checked })} />{at(locale, 'insecure')}</label>
      </fieldset>
      {draft.allowInsecureHttp && <p className="ai-warning">{at(locale, 'insecureWarning')}</p>}
      <p className="pane-note">{at(locale, 'endpointWarning')}</p>
      <p className="ai-endpoint"><strong>{at(locale, 'requestUrl')}:</strong> <code>{currentUrl?.requestUrl ?? '—'}</code></p>
      <div className="ai-controls">
        <button type="button" disabled={disabled || !currentUrl || !validModel || !draft.name.trim()} onClick={() => { void save(); }}>{at(locale, 'save')}</button>
        <button type="button" disabled={disabled || !selected} onClick={() => setDeleting(true)}>{at(locale, 'delete')}</button>
        <button type="button" disabled={disabled || !savedDraft || !validModel || !currentUrl} onClick={() => setTestReview(true)}>{at(locale, 'test')}</button>
      </div>
      {!savedDraft && <p className="pane-note">{at(locale, 'savedRequired')}</p>}
      {deleting && <div className="ai-confirm" role="alertdialog" aria-label={at(locale, 'deleteQuestion')}><span>{at(locale, 'deleteQuestion')}</span><button type="button" disabled={disabled} onClick={() => { void remove(); }}>{at(locale, 'confirmDelete')}</button><button type="button" onClick={() => setDeleting(false)}>{at(locale, 'cancel')}</button></div>}
      {draft.authMode === 'apiKey' && <fieldset className="ai-credentials" disabled={disabled || !savedDraft || !currentUrl}>
        <legend>{at(locale, 'apiKey')}</legend>
        <p role="status">{at(locale, selected?.profile.baseUrl === draft.baseUrl ? selected?.credentialStatus.status ?? 'missing' : 'missing')}</p>
        <div className="ai-controls"><label className="ai-checkbox"><input type="radio" name="ai-key-storage" value="keychain" checked={storage === 'keychain'} onChange={() => setStorage('keychain')} />{at(locale, 'keychain')}</label><label className="ai-checkbox"><input type="radio" name="ai-key-storage" value="session" checked={storage === 'session'} onChange={() => setStorage('session')} />{at(locale, 'sessionStorage')}</label></div>
        <label><span>{at(locale, 'key')}</span><input type="password" autoComplete="new-password" value={key} spellCheck={false} onChange={(event) => setKey(event.target.value)} /></label>
        <div className="ai-controls"><button type="button" disabled={!key} onClick={() => { void saveKey(); }}>{at(locale, 'saveKey')}</button><button type="button" onClick={() => {
          setKey(''); void profiles.removeCredential(draft.id).catch((failure: unknown) => setError(toAppError(failure)));
        }}>{at(locale, 'removeKey')}</button></div>
        <p className="pane-note">{at(locale, 'keyPrivacy')}</p>
        {(selected?.credentialStatus.status === 'locked' || selected?.credentialStatus.status === 'unavailable') && <p className="ai-warning">{at(locale, 'keychainChoice')}</p>}
      </fieldset>}
      {draft.authMode === 'none' && <p className="pane-note">{at(locale, 'notRequired')}</p>}
      {testReview && <div className="ai-confirm" role="alertdialog" aria-label={at(locale, 'test')}><p>{at(locale, 'testWarning')}</p><p><code>{selected?.requestUrl}</code> · {draft.model} · 你好。 → vi</p><button type="button" disabled={disabled || !savedDraft} onClick={() => { void runTest(); }}>{at(locale, 'runTest')}</button><button type="button" onClick={() => setTestReview(false)}>{at(locale, 'cancel')}</button></div>}
      {testResult && <div className="ai-test-result" role="status"><strong>{at(locale, 'testSuccess')}:</strong> {testResult.host} · {testResult.model}<pre>{testResult.text}</pre></div>}
    </>}
    {profiles.busy && <p role="status">{at(locale, 'busy')}</p>}
    {profiles.testing && <button type="button" onClick={profiles.cancelTest}>{at(locale, 'cancel')}</button>}
    {(error ?? profiles.error) && <p className="workspace-error" role="alert">{aiError(locale, (error ?? profiles.error)!)} <span className="error-code">({(error ?? profiles.error)!.code})</span></p>}
  </section>;
}
