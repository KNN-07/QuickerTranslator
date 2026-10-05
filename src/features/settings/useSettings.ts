import { useCallback, useEffect, useRef, useState } from 'react';
import { defaultPreferences } from '../../app/preferences';
import type { InterfacePreferences } from '../../app/preferences';
import { invokeCommand, nativeAvailable, toAppError } from '../../lib/ipc';
import type { AiProfile, AppError } from '../../lib/types';
import type { AiProfileRecord } from '../ai/profiles';
import type { WorkspaceController } from '../workspace/useWorkspaceController';

export function useSettings(workspace: WorkspaceController, onError: (error: AppError) => void) {
  const native = nativeAvailable();
  const [preferences, setPreferences] = useState(defaultPreferences);
  const [ready, setReady] = useState(!native);
  const [preservedInvalidFile, setPreservedInvalidFile] = useState(false);
  const applied = useRef(false);
  const baseline = useRef('');
  const saveQueue = useRef(Promise.resolve());
  const latest = useRef({ workspace, onError, preferences }); latest.current = { workspace, onError, preferences };
  const nativeProfiles = useRef<AiProfile[] | null>(null);
  const syncProfiles = useCallback((profiles: AiProfile[]) => {
    nativeProfiles.current = profiles;
    setPreferences((current) => ({ ...current, profiles }));
    if (baseline.current) {
      const saved = JSON.parse(baseline.current) as InterfacePreferences;
      baseline.current = JSON.stringify({ ...saved, profiles });
    }
  }, []);
  const currentProfileSettings = useCallback(async (settings: InterfacePreferences) => {
    const records = await invokeCommand<AiProfileRecord[]>('list_ai_profiles');
    const profiles = records.map((record) => record.profile);
    syncProfiles(profiles);
    return { ...settings, profiles };
  }, [syncProfiles]);
  useEffect(() => {
    if (!native) { applied.current = true; return; }
    let disposed = false;
    void invokeCommand<{ settings: InterfacePreferences; preservedInvalidFile: boolean; warning: string | null }>('load_settings').then((result) => {
      if (disposed) return;
      latest.current.workspace.loadPreferences(result.settings.editorOptions, result.settings.translationOptions);
      const loaded = nativeProfiles.current ? { ...result.settings, profiles: nativeProfiles.current } : result.settings;
      baseline.current = JSON.stringify(loaded);
      applied.current = true;
      setPreferences(loaded); setPreservedInvalidFile(result.preservedInvalidFile); setReady(true);
    }).catch((error: unknown) => {
      if (disposed) return;
      // Loading failure must not silently overwrite a file whose state is unknown.
      setPreservedInvalidFile(true); setReady(true); applied.current = true;
      latest.current.onError(toAppError(error));
    });
    return () => { disposed = true; };
  }, [native]);
  useEffect(() => {
    if (!ready || !applied.current) return;
    const editorOptions = { ...workspace.state.viewState };
    delete editorOptions.legacyScrollIndices;
    setPreferences((current) => ({
      ...current, editorOptions, translationOptions: workspace.state.options,
    }));
  }, [ready, workspace.state.viewState, workspace.state.options]);
  useEffect(() => {
    if (!native || !ready || preservedInvalidFile) return;
    const serialized = JSON.stringify(preferences);
    if (serialized === baseline.current) return;
    const timer = window.setTimeout(() => {
      saveQueue.current = saveQueue.current.catch(() => {}).then(async () => {
        const settings = await currentProfileSettings(latest.current.preferences);
        await invokeCommand('save_settings', { settings });
        baseline.current = JSON.stringify(settings);
      }).catch((error: unknown) => latest.current.onError(toAppError(error)));
    }, 450);
    return () => window.clearTimeout(timer);
  }, [native, ready, preferences, preservedInvalidFile, currentProfileSettings]);
  const replaceInvalid = useCallback(async () => {
    try {
      await saveQueue.current;
      const settings = await currentProfileSettings(latest.current.preferences);
      await invokeCommand('save_settings', { settings, replaceInvalid: true });
      baseline.current = JSON.stringify(settings); setPreservedInvalidFile(false);
    } catch (error: unknown) { latest.current.onError(toAppError(error)); }
  }, [currentProfileSettings]);
  const flush = useCallback(async () => {
    if (!native || !ready || preservedInvalidFile) return;
    await saveQueue.current;
    if (JSON.stringify(latest.current.preferences) !== baseline.current) {
      const settings = await currentProfileSettings(latest.current.preferences);
      await invokeCommand('save_settings', { settings }); baseline.current = JSON.stringify(settings);
    }
  }, [native, ready, preservedInvalidFile, currentProfileSettings]);
  const preserveInvalid = useCallback(() => setPreservedInvalidFile(true), []);
  return { preferences, setPreferences, syncProfiles, ready, preservedInvalidFile, preserveInvalid, replaceInvalid, flush };
}
