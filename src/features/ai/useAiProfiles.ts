import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invokeCommand, nativeAvailable, toAppError } from '../../lib/ipc';
import type { AiProfile, AppError } from '../../lib/types';
import type { AiConnectionTest, AiProfilePreview, AiProfileRecord, CredentialStatus, CredentialStorage } from './profiles';

export interface AiProfilesController {
  records: AiProfileRecord[];
  presets: AiProfile[];
  ready: boolean;
  busy: boolean;
  testing: boolean;
  error: AppError | null;
  refresh: () => Promise<void>;
  preview: (profile: AiProfile) => Promise<AiProfilePreview>;
  save: (profile: AiProfile) => Promise<AiProfileRecord>;
  remove: (profileId: string) => Promise<void>;
  setCredential: (profileId: string, expectedEndpoint: string, key: string, storage: CredentialStorage) => Promise<CredentialStatus>;
  removeCredential: (profileId: string) => Promise<void>;
  test: (profileId: string) => Promise<AiConnectionTest>;
  cancelTest: () => void;
}
export function useAiProfiles(syncProfiles: (profiles: AiProfile[]) => void): AiProfilesController {
  const [records, setRecords] = useState<AiProfileRecord[]>([]);
  const [presets, setPresets] = useState<AiProfile[]>([]);
  const [ready, setReady] = useState(!nativeAvailable());
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const testJob = useRef<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const sync = useRef(syncProfiles); sync.current = syncProfiles;
  const mounted = useRef(true);
  const loadGeneration = useRef(0);
  const refresh = useCallback(async () => {
    if (!nativeAvailable()) return;
    const ticket = ++loadGeneration.current;
    try {
      const values = await invokeCommand<AiProfileRecord[]>('list_ai_profiles');
      if (!mounted.current || ticket !== loadGeneration.current) return;
      sync.current(values.map((item) => item.profile));
      setRecords(values); setReady(true); setError(null);
    } catch (failure: unknown) {
      if (mounted.current && ticket === loadGeneration.current) { setError(toAppError(failure)); setReady(true); }
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    if (!nativeAvailable()) return () => { mounted.current = false; };
    let disposed = false;
    const subscription = listen<AiProfile[]>('profiles-changed', ({ payload }) => {
      if (disposed) return;
      sync.current(payload);
      void refresh();
    });
    void subscription.then(() => { if (!disposed) void refresh(); }).catch((failure: unknown) => {
      if (!disposed) { setError(toAppError(failure)); setReady(true); }
    });
    void invokeCommand<AiProfile[]>('ai_profile_presets').then((values) => { if (!disposed) setPresets(values); }).catch((failure: unknown) => { if (!disposed) setError(toAppError(failure)); });
    return () => { disposed = true; mounted.current = false; void subscription.then((stop) => stop()).catch(() => {}); };
  }, [refresh]);
  const preview = useCallback((profile: AiProfile) => invokeCommand<AiProfilePreview>('preview_ai_profile', { profile }), []);
  const save = useCallback(async (profile: AiProfile) => {
    setBusy(true); setError(null);
    try {
      const record = await invokeCommand<AiProfileRecord>('save_ai_profile', { profile });
      await refresh();
      return record;
    } catch (failure: unknown) { setError(toAppError(failure)); throw failure; }
    finally { if (mounted.current) setBusy(false); }
  }, [refresh]);
  const remove = useCallback(async (profileId: string) => {
    setBusy(true); setError(null);
    try { await invokeCommand('delete_ai_profile', { profileId }); await refresh(); }
    catch (failure: unknown) { setError(toAppError(failure)); throw failure; }
    finally { if (mounted.current) setBusy(false); }
  }, [refresh]);
  const setCredential = useCallback(async (profileId: string, expectedEndpoint: string, key: string, storage: CredentialStorage) => {
    setBusy(true); setError(null);
    try {
      const status = await invokeCommand<CredentialStatus>('set_ai_credential', { profileId, expectedEndpoint, key, storage });
      await refresh(); return status;
    } catch (failure: unknown) { setError(toAppError(failure)); throw failure; }
    finally { if (mounted.current) setBusy(false); }
  }, [refresh]);
  const removeCredential = useCallback(async (profileId: string) => {
    setBusy(true); setError(null);
    try { await invokeCommand('delete_ai_credential', { profileId }); await refresh(); }
    catch (failure: unknown) { setError(toAppError(failure)); throw failure; }
    finally { if (mounted.current) setBusy(false); }
  }, [refresh]);
  const cancelTest = useCallback(() => {
    if (!testJob.current || !nativeAvailable()) return;
    void invokeCommand('cancel_ai_translation', { jobId: testJob.current }).catch((failure: unknown) => {
      if (mounted.current) setError(toAppError(failure));
    });
  }, []);
  const test = useCallback(async (profileId: string) => {
    const jobId = crypto.randomUUID();
    testJob.current = jobId;
    setBusy(true); setTesting(true); setError(null);
    try { return await invokeCommand<AiConnectionTest>('test_ai_profile', { profileId, jobId }); }
    catch (failure: unknown) { if (mounted.current) setError(toAppError(failure)); throw failure; }
    finally {
      if (testJob.current === jobId) testJob.current = null;
      if (mounted.current) { setBusy(false); setTesting(false); }
    }
  }, []);
  useEffect(() => () => cancelTest(), [cancelTest]);
  return { records, presets, ready, busy, testing, error, refresh, preview, save, remove, setCredential, removeCredential, test, cancelTest };
}
