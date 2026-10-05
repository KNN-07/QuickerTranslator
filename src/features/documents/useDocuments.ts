import { useCallback, useEffect, useRef, useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invokeCommand, nativeAvailable, newDocumentWindow, toAppError } from '../../lib/ipc';
import { translate } from '../../lib/i18n';
import type { AppError, UiLocale } from '../../lib/types';
import type { WorkspaceController } from '../workspace/useWorkspaceController';
import { documentStamp, guardUnsaved, unchangedDocument } from './guard';
import type { DocumentStamp } from './guard';
import { fileName, projectDocument, projectName, withExtension } from './types';
import type { DocumentImportPreview, ExportOptions, OpenDocumentResult, RecoveryDocument, RecoveryRecord, SaveResult, UnsavedDecision } from './types';
import { ft } from './i18n';

export interface DocumentController {
  path: string | null; name: string | null; busy: boolean; unsavedOpen: boolean;
  answerUnsaved: (answer: UnsavedDecision) => void;
  importPreview: DocumentImportPreview | null; importEncoding: string;
  setImportEncoding: (encoding: string) => void;
  previewEncoding: () => Promise<void>; confirmImport: () => Promise<void>; cancelImport: () => void;
  exportOpen: boolean; setExportOpen: (open: boolean) => void;
  exportDocument: (options: ExportOptions) => Promise<void>; legacyRtf: string | undefined;
  openFile: () => Promise<void>; saveDocument: (saveAs?: boolean) => Promise<void>;
  createWindow: () => Promise<void>; closeWindow: () => Promise<void>; navigate: (direction: -1 | 1) => Promise<void>;
  siblings: { previous: string | null; next: string | null };
  recoveries: RecoveryRecord[]; recoveryOpen: boolean; setRecoveryOpen: (open: boolean) => void;
  recover: (record: RecoveryRecord) => Promise<void>; discardRecovery: (record: RecoveryRecord) => Promise<void>;
  warnings: string[]; setWarnings: (warnings: string[]) => void;
  notice: 'saved' | 'exported' | 'syncWarning' | null; setNotice: (notice: 'saved' | 'exported' | 'syncWarning' | null) => void;
}
export function useDocuments({ workspace, locale, ready, onError, flushSettings }: {
  workspace: WorkspaceController; locale: UiLocale; ready: boolean; onError: (error: AppError) => void; flushSettings: () => Promise<void>;
}): DocumentController {
  const native = nativeAvailable();
  const [path, setPath] = useState<string | null>(null);
  const [name, setName] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [unsavedOpen, setUnsavedOpen] = useState(false);
  const [importPreview, setImportPreview] = useState<DocumentImportPreview | null>(null);
  const [importEncoding, setImportEncoding] = useState('');
  const [exportOpen, setExportOpen] = useState(false);
  const [recoveries, setRecoveries] = useState<RecoveryRecord[]>([]);
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [notice, setNotice] = useState<'saved' | 'exported' | 'syncWarning' | null>(null);
  const [siblings, setSiblings] = useState<{ previous: string | null; next: string | null }>({ previous: null, next: null });
  const [legacyRtf, setLegacyRtf] = useState<string>();
  const recordId = useRef<string>(crypto.randomUUID());
  const recordPath = useRef<string | null>(null);
  const recordName = useRef<string | null>(null);
  const rtf = useRef<string | undefined>(undefined);
  const locked = useRef(false);
  const closing = useRef(false);
  const decision = useRef<((answer: UnsavedDecision) => void) | null>(null);
  const recoveryQueue = useRef(Promise.resolve());
  const recoveryEnabled = useRef(false);
  const [recoveryReady, setRecoveryReady] = useState(false);
  const latest = useRef({ workspace, locale, onError, flushSettings }); latest.current = { workspace, locale, onError, flushSettings };
  const answerUnsaved = useCallback((answer: UnsavedDecision) => {
    const resolve = decision.current; decision.current = null; setUnsavedOpen(false); resolve?.(answer);
  }, []);
  const decide = useCallback(() => new Promise<UnsavedDecision>((resolve) => {
    decision.current?.('cancel'); decision.current = resolve; setUnsavedOpen(true);
  }), []);
  const saveCurrent = useCallback(async (saveAs = false): Promise<boolean> => {
    const { workspace: controller, locale: language } = latest.current;
    let destination = recordPath.current;
    if (saveAs || !destination || !destination.toLowerCase().endsWith('.qtp')) {
      destination = await save({ title: translate(language, 'saveAs'), defaultPath: destination ? projectName(destination) : `${translate(language, 'untitled')}.qtp`, filters: [{ name: ft(language, 'projectFilter'), extensions: ['qtp'] }] });
      if (!destination) return false;
      destination = withExtension(destination, 'qtp');
    }
    const stamp = documentStamp(controller.getState());
    const project = projectDocument(controller.serialize(), rtf.current);
    const result = await invokeCommand<SaveResult>('save_document', { request: { path: destination, project } });
    recordPath.current = result.path; recordName.current = fileName(result.path); setPath(result.path); setName(fileName(result.path));
    if (unchangedDocument(controller.getState(), stamp)) {
      controller.markSaved();
      await recoveryQueue.current;
      if (unchangedDocument(controller.getState(), stamp)) {
        try { await invokeCommand('recovery_discard', { documentId: recordId.current }); }
        catch (error: unknown) { latest.current.onError(toAppError(error)); }
      }
    }
    setNotice(result.directorySyncConfirmed ? 'saved' : 'syncWarning');
    return unchangedDocument(controller.getState(), stamp) && !controller.getState().dirty;
  }, []);
  const guarded = useCallback(() => guardUnsaved(latest.current.workspace.getState, decide, () => saveCurrent()), [decide, saveCurrent]);
  const run = useCallback(async (operation: () => Promise<void>) => {
    if (!native || !ready || locked.current) return;
    locked.current = true; setBusy(true);
    try { await operation(); }
    catch (error: unknown) { latest.current.onError(toAppError(error)); }
    finally { locked.current = false; setBusy(false); }
  }, [native, ready]);
  const clearOldRecovery = useCallback(async () => {
    await recoveryQueue.current;
    await invokeCommand('recovery_discard', { documentId: recordId.current });
  }, []);
  const install = useCallback(async (result: Extract<OpenDocumentResult, { status: 'ready' }>, recovery?: RecoveryDocument, approval?: DocumentStamp) => {
    const allowed = approval ? unchangedDocument(latest.current.workspace.getState(), approval) : await guarded() !== 'cancel';
    if (!allowed) return false;
    const controller = latest.current.workspace;
    // Schema validation precedes all active-document/recovery changes, including Discard.
    controller.targetEditor?.schema.nodeFromJSON(result.project.targetDocument).check();
    const previous = controller.serialize(); const previousDirty = controller.getState().dirty;
    try { controller.restore(result.project); }
    catch (error: unknown) {
      controller.restore(previous); if (previousDirty) controller.markDirty(); throw error;
    }
    recoveryEnabled.current = false;
    try { if (recordId.current !== recovery?.documentId) await clearOldRecovery(); }
    catch (error: unknown) { latest.current.onError(toAppError(error)); }
    recordId.current = recovery?.documentId ?? crypto.randomUUID();
    recordPath.current = recovery ? recovery.path : result.path;
    recordName.current = recovery?.name ?? fileName(result.path);
    rtf.current = result.project.legacyRtf;
    setPath(recordPath.current); setName(recordName.current); setLegacyRtf(rtf.current); setImportPreview(null); setWarnings(result.warnings);
    if (recovery || result.imported) controller.markDirty();
    recoveryEnabled.current = true;
    return true;
  }, [guarded, clearOldRecovery]);
  const openPath = useCallback(async (selectedPath: string, encoding?: string, confirmDetectedEncoding = false) => {
    const result = await invokeCommand<OpenDocumentResult>('open_document', { request: {
      path: selectedPath, ...(encoding ? { encoding } : {}), sourceLanguage: latest.current.workspace.getState().sourceLanguage, confirmDetectedEncoding,
    } });
    if (result.status === 'encodingConfirmationRequired') { setImportPreview(result.preview); setImportEncoding(''); return; }
    await install(result);
  }, [install]);
  const openFile = useCallback(() => run(async () => {
    const chosen = await open({ title: ft(latest.current.locale, 'importTitle'), multiple: false, directory: false, filters: [
      { name: ft(latest.current.locale, 'documentFilter'), extensions: ['qtp', 'qt', 'txt', 'html', 'htm'] }, { name: ft(latest.current.locale, 'allFiles'), extensions: ['*'] },
    ] });
    if (typeof chosen !== 'string') return;
    // Text imports expose an encoding override even when detection chose valid UTF-8.
    const preview = await invokeCommand<DocumentImportPreview>('preview_document_import', { request: { path: chosen, sourceLanguage: latest.current.workspace.getState().sourceLanguage } });
    if (preview.kind === 'project') await openPath(chosen);
    else { setImportPreview(preview); setImportEncoding(''); }
  }), [run, openPath]);
  const previewEncoding = useCallback(() => run(async () => {
    if (!importPreview) return;
    const result = await invokeCommand<DocumentImportPreview>('preview_document_import', { request: {
      path: importPreview.path, sourceLanguage: latest.current.workspace.getState().sourceLanguage, ...(importEncoding ? { encoding: importEncoding } : {}),
    } });
    setImportPreview(result);
  }), [run, importPreview, importEncoding]);
  const confirmImport = useCallback(() => run(async () => {
    if (importPreview) await openPath(importPreview.path, importEncoding || importPreview.encoding, true);
  }), [run, importPreview, importEncoding, openPath]);
  const navigate = useCallback((direction: -1 | 1) => run(async () => {
    const destination = direction < 0 ? siblings.previous : siblings.next;
    if (destination) await openPath(destination);
  }), [run, siblings, openPath]);
  const saveDocument = useCallback((saveAs = false) => run(async () => { await saveCurrent(saveAs); }), [run, saveCurrent]);
  const createWindow = useCallback(() => run(async () => {
    // The existing window stays open: Discard authorizes New but does not discard its work.
    if (await guarded() !== 'cancel') await newDocumentWindow();
  }), [run, guarded]);
  const closeWindow = useCallback(() => run(async () => {
    if (await guarded() === 'cancel') return;
    await latest.current.flushSettings();
    await clearOldRecovery(); latest.current.workspace.cancel(); closing.current = true;
    try { await getCurrentWindow().destroy(); }
    catch (error: unknown) { closing.current = false; throw error; }
  }), [run, guarded, clearOldRecovery]);
  const exportDocument = useCallback((options: ExportOptions) => run(async () => {
    const controller = latest.current.workspace;
    const extension = options.format;
    const destination = await save({ title: ft(latest.current.locale, 'exportTitle'), defaultPath: `${recordName.current?.replace(/\.[^.]+$/u, '') ?? 'Untitled'}.${extension}`, filters: [{ name: extension.toUpperCase(), extensions: [extension] }] });
    if (!destination) return;
    const result = await invokeCommand<SaveResult>('export_document', { request: {
      path: withExtension(destination, extension), format: options.format, project: projectDocument(controller.serialize(), rtf.current),
      columns: options.columns, blankLines: options.blankLines, readings: controller.getState().result?.readings ?? '',
      phrases: controller.getState().result?.phrases ?? '', singleMeaning: controller.draft.text,
    } });
    setNotice(result.directorySyncConfirmed ? 'exported' : 'syncWarning'); setExportOpen(false);
  }), [run]);
  const recover = useCallback((record: RecoveryRecord) => run(async () => {
    if (await guarded() === 'cancel') return;
    const approval = documentStamp(latest.current.workspace.getState());
    const recovered = await invokeCommand<RecoveryDocument>('recovery_read', { documentId: record.documentId });
    let installed = false;
    try {
      installed = await install({ status: 'ready', path: recovered.path ?? recovered.name, project: recovered.project, imported: true, warnings: [] }, recovered, approval);
      if (installed) { setRecoveries((current) => current.filter((item) => item.documentId !== record.documentId)); setRecoveryOpen(false); }
    } finally {
      if (!installed && recordId.current !== recovered.documentId) await invokeCommand('recovery_release', { documentId: recovered.documentId });
    }
  }), [run, guarded, install]);
  const discardRecovery = useCallback((record: RecoveryRecord) => run(async () => {
    await invokeCommand('recovery_discard', { documentId: record.documentId });
    setRecoveries((current) => current.filter((item) => item.documentId !== record.documentId));
  }), [run]);
  useEffect(() => {
    if (!native || !ready) return;
    let disposed = false;
    void invokeCommand<{ records: RecoveryRecord[]; warnings: string[] }>('recovery_list').then((result) => {
      if (disposed) return;
      setRecoveries(result.records); setRecoveryOpen(result.records.length > 0); setWarnings(result.warnings); recoveryEnabled.current = true; setRecoveryReady(true);
    }).catch((error: unknown) => { if (!disposed) { latest.current.onError(toAppError(error)); recoveryEnabled.current = true; setRecoveryReady(true); } });
    return () => { disposed = true; };
  }, [native, ready]);
  useEffect(() => {
    if (!native || !ready || !recoveryReady || !workspace.state.dirty) return;
    const timer = window.setTimeout(() => {
      if (!recoveryEnabled.current || locked.current) return;
      const request = { documentId: recordId.current, name: recordName.current ?? translate(latest.current.locale, 'untitled'), path: recordPath.current,
        project: projectDocument(latest.current.workspace.serialize(), rtf.current) };
      recoveryQueue.current = recoveryQueue.current.catch(() => {}).then(async () => { await invokeCommand('recovery_write', { request }); })
        .catch((error: unknown) => latest.current.onError(toAppError(error)));
    }, 800);
    return () => window.clearTimeout(timer);
  }, [native, ready, recoveryReady, workspace.state.sourceRevision, workspace.state.targetRevision, workspace.state.draftEdits, workspace.state.viewState, workspace.state.dirty, busy]);
  useEffect(() => {
    if (!native || !ready || !path) { setSiblings({ previous: null, next: null }); return; }
    let disposed = false;
    setSiblings({ previous: null, next: null });
    void invokeCommand<{ previous: string | null; next: string | null }>('document_siblings', { path }).then((result) => {
      if (!disposed) setSiblings(result);
    }).catch((error: unknown) => { if (!disposed) latest.current.onError(toAppError(error)); });
    return () => { disposed = true; };
  }, [native, ready, path]);
  useEffect(() => {
    const title = `${workspace.state.dirty ? '* ' : ''}${name ?? translate(locale, 'untitled')} — QuickTranslator`;
    document.title = title;
    if (native && ready) void getCurrentWindow().setTitle(title).catch((error: unknown) => latest.current.onError(toAppError(error)));
  }, [native, ready, name, locale, workspace.state.dirty]);
  useEffect(() => {
    if (!native || !ready) return;
    let disposed = false;
    const window = getCurrentWindow();
    const closeListener = window.onCloseRequested((event) => {
      if (closing.current) return;
      event.preventDefault(); if (!disposed) void closeWindow();
    });
    const dropListener = window.onDragDropEvent((event) => {
      if (disposed || event.payload.type !== 'drop') return;
      void run(async () => {
        if (event.payload.type !== 'drop') return;
        if (event.payload.paths.length !== 1) { latest.current.onError({ code: 'multipleDocumentDrops', message: '' }); return; }
        const selectedPath = event.payload.paths[0]!;
        const preview = await invokeCommand<DocumentImportPreview>('preview_document_import', { request: { path: selectedPath, sourceLanguage: latest.current.workspace.getState().sourceLanguage } });
        if (preview.kind === 'project') await openPath(selectedPath);
        else { setImportPreview(preview); setImportEncoding(''); }
      });
    });
    void Promise.all([closeListener, dropListener]).catch((error: unknown) => latest.current.onError(toAppError(error)));
    return () => { disposed = true; void closeListener.then((unlisten) => unlisten()).catch(() => {}); void dropListener.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native, ready, closeWindow, run, openPath]);
  useEffect(() => () => { decision.current?.('cancel'); decision.current = null; }, []);
  return { path, name, busy, unsavedOpen, answerUnsaved, importPreview, importEncoding, setImportEncoding, previewEncoding, confirmImport,
    cancelImport: () => setImportPreview(null), exportOpen, setExportOpen, exportDocument, legacyRtf, openFile, saveDocument, createWindow, closeWindow,
    navigate, siblings, recoveries, recoveryOpen, setRecoveryOpen, recover, discardRecovery, warnings, setWarnings, notice, setNotice };
}
