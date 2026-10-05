import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DockviewApi } from 'dockview-react';
import { DockWorkspace, PANE_IDS, createDefaultLayout, showPane } from './DockWorkspace';
import { Toolbar } from './Toolbar';
import { InterfaceSettings } from './InterfaceSettings';
import { DictionaryDialog } from '../features/dictionaries/DictionaryDialog';
import type { DictionaryTab } from '../features/dictionaries/DictionaryDialog';
import { translate } from '../lib/i18n';
import type { TranslationKey } from '../lib/i18n';
import { getFoundationHealth, nativeAvailable, toAppError } from '../lib/ipc';
import type { AppError, FoundationHealth, PaneId, UiLocale } from '../lib/types';
import { useWorkspaceController } from '../features/workspace/useWorkspaceController';
import { WorkspacePane } from '../features/workspace/WorkspacePanes';
import { wt } from '../features/workspace/i18n';
import '../styles/editors.css';
import { openSearchPanel } from '@codemirror/search';
import type { SerializedDockview } from 'dockview-react';
import { useSettings } from '../features/settings/useSettings';
import { useShortcuts } from '../features/settings/useShortcuts';
import { useDocuments } from '../features/documents/useDocuments';
import { DocumentDialogs } from '../features/documents/DocumentDialogs';
import { useKeyboard } from '../features/documents/useKeyboard';
import { ft, localizedDocumentError, localizedDocumentWarning } from '../features/documents/i18n';
import '../styles/documents.css';
import { useAi } from '../features/ai/useAi';
import { useAiProfiles } from '../features/ai/useAiProfiles';
import { AiPane } from '../features/ai/AiPane';
import { at } from '../features/ai/i18n';
import '../styles/ai.css';

export function App() {
  const [locale, setLocale] = useState<UiLocale>('vi');
  const [dockApi, setDockApi] = useState<DockviewApi>();
  const [visiblePanes, setVisiblePanes] = useState<PaneId[]>([...PANE_IDS]);
  const [configOpen, setConfigOpen] = useState(false);
  const [focusAiProfiles, setFocusAiProfiles] = useState(false);
  const [aiReplaceOpen, setAiReplaceOpen] = useState(false);
  const aiReplaceDecision = useRef<((answer: boolean) => void) | null>(null);
  const invalidateAi = useRef<(() => void) | null>(null);
  const confirmReplaceTarget = useCallback(() => new Promise<boolean>((resolve) => {
    aiReplaceDecision.current?.(false);
    aiReplaceDecision.current = resolve;
    setAiReplaceOpen(true);
  }), []);
  const answerReplaceTarget = useCallback((answer: boolean) => {
    const resolve = aiReplaceDecision.current;
    aiReplaceDecision.current = null;
    setAiReplaceOpen(false);
    resolve?.(answer);
  }, []);
  useEffect(() => () => { aiReplaceDecision.current?.(false); aiReplaceDecision.current = null; }, []);
  const [dictionaryTab, setDictionaryTab] = useState<DictionaryTab | null>(null);
  const [health, setHealth] = useState<FoundationHealth | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [findRequested, setFindRequested] = useState(false);
  const [notice, setNotice] = useState<TranslationKey | null>(null);
  const [draftDiscardOpen, setDraftDiscardOpen] = useState(false);
  const draftDecision = useRef<((answer: boolean) => void) | null>(null);
  const confirmDiscardDraft = useCallback(() => new Promise<boolean>((resolve) => {
    draftDecision.current?.(false);
    draftDecision.current = resolve;
    setDraftDiscardOpen(true);
  }), []);
  const answerDraftDiscard = useCallback((answer: boolean) => {
    const resolve = draftDecision.current;
    draftDecision.current = null;
    setDraftDiscardOpen(false);
    resolve?.(answer);
  }, []);
  useEffect(() => () => { draftDecision.current?.(false); draftDecision.current = null; }, []);
  const native = nativeAvailable();
  const shortcuts = useShortcuts(setError);
  const workspace = useWorkspaceController({
    locale,
    shortcuts,
    documentId: health?.documentWindow.documentId,
    sourceRevision: health?.sourceRevision,
    targetRevision: health?.targetRevision,
    dictionaryRevision: health?.dictionaryRevision ?? undefined,
    japaneseAvailable: health?.tokenizerReady ?? false,
    confirmDiscardDraft,
    onInvalidate: () => invalidateAi.current?.(),
  });
  const { sourceLanguage } = workspace.state;
  const settings = useSettings(workspace, setError);
  const { preferences, setPreferences } = settings;
  const aiProfiles = useAiProfiles(settings.syncProfiles);
  const ai = useAi({ workspace, confirmReplaceTarget, onError: setError });
  invalidateAi.current = () => { ai.cancel(); aiProfiles.cancelTest(); };
  const openAiProfiles = useCallback(() => { ai.cancel(); setFocusAiProfiles(true); setConfigOpen(true); }, [ai.cancel]);
  const theme = preferences.theme;
  const documents = useDocuments({ workspace, locale, ready: settings.ready && (!native || !!health) && !!workspace.targetEditor, onError: setError, flushSettings: settings.flush });
  const layoutChanged = useCallback((layout: SerializedDockview) => {
    setPreferences((current) => ({ ...current, dockLayout: layout }));
  }, [setPreferences]);
  const layoutNotice = useCallback((key: TranslationKey) => {
    if (key === 'layoutRestoreError') settings.preserveInvalid();
    setNotice(key);
  }, [settings.preserveInvalid]);
  const showSource = useCallback(() => { if (dockApi) showPane(dockApi, 'source', locale, sourceLanguage); }, [dockApi, locale, sourceLanguage]);
  const findSource = useCallback(() => { showSource(); setFindRequested(true); }, [showSource]);
  const showTarget = useCallback(() => { if (dockApi) showPane(dockApi, 'target', locale, sourceLanguage); }, [dockApi, locale, sourceLanguage]);
  useKeyboard({ controller: workspace, documents, preferences, showSource, findSource, showTarget });
  useEffect(() => { setLocale(preferences.locale); }, [preferences.locale]);
  const sourceCount = useMemo(() => Array.from(workspace.state.sourceText).length, [workspace.state.sourceText]);
  const sourcePrefix = workspace.state.sourceText.slice(0, workspace.state.sourceSelection.start);
  const cursorLine = sourcePrefix.split('\n').length;
  const cursorColumn = sourcePrefix.length - sourcePrefix.lastIndexOf('\n');
  useEffect(() => {
    if (findRequested && workspace.sourceView) {
      openSearchPanel(workspace.sourceView);
      setFindRequested(false);
    }
  }, [findRequested, workspace.sourceView]);

  useEffect(() => {
    document.documentElement.lang = locale;
    document.documentElement.dataset.theme = theme;
  }, [locale, theme]);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    getFoundationHealth().then((result) => {
      if (!disposed) setHealth(result);
    }).catch((failure: unknown) => {
      if (!disposed) setError(toAppError(failure));
    });
    return () => { disposed = true; };
  }, [native]);


  const changePaneVisibility = (pane: PaneId, visible: boolean) => {
    if (!dockApi) return;
    if (visible) showPane(dockApi, pane, locale, sourceLanguage);
    else dockApi.getPanel(pane)?.api.close();
  };

  const connectionKey: TranslationKey = !native ? 'browserStatus' : health ? 'nativeConnected' : error ? 'nativeDisconnected' : 'nativeConnecting';

  return (
    <div className="app-shell">
      <Toolbar
        locale={locale}
        sourceLanguage={sourceLanguage}
        native={native}
        creatingWindow={documents.busy || !settings.ready || (native && !health)}
        documentBusy={documents.busy || !settings.ready || (native && !health)}
        onOpen={() => { void documents.openFile(); }}
        onSave={(saveAs) => { void documents.saveDocument(saveAs); }}
        onExport={() => documents.setExportOpen(true)}
        onCloseDocument={() => { void documents.closeWindow(); }}
        onNavigate={(direction) => { void documents.navigate(direction); }}
        canBack={!!documents.siblings.previous}
        canNext={!!documents.siblings.next}
        onOriginalRtf={documents.legacyRtf !== undefined ? () => { void documents.exportDocument({ format: 'rtf', columns: ['target'], blankLines: 0 }); } : undefined}
        onRecover={documents.recoveries.length ? () => documents.setRecoveryOpen(true) : undefined}
        visiblePanes={visiblePanes}
        onSourceLanguageChange={workspace.setSourceLanguage}
        onNewWindow={() => { void documents.createWindow(); }}
        onResetLayout={() => { if (dockApi) createDefaultLayout(dockApi, locale, sourceLanguage); }}
        onPaneVisibilityChange={changePaneVisibility}
        onConfig={() => { setFocusAiProfiles(false); setConfigOpen(true); }}
        onReloadDictionaries={() => setDictionaryTab('imports')}
        onRetranslate={() => { void workspace.translate(); }}
        onTranslateClipboard={() => { void workspace.translateClipboard(); }}
        onFindSource={findSource}
        onAiTranslate={() => { if (dockApi) showPane(dockApi, 'ai', locale, sourceLanguage); }}
        translating={workspace.state.translating}
      />
      {!native && <div className="runtime-banner runtime-banner--browser" role="status">{translate(locale, 'browserNotice')}</div>}
      {draftDiscardOpen && <div className="runtime-banner runtime-banner--notice" role="alertdialog" aria-label={wt(locale, 'discardDraft')} onKeyDown={(event) => {
        if (event.key === 'Escape') { event.preventDefault(); answerDraftDiscard(false); }
      }}>
        <span>{wt(locale, 'discardDraft')}</span>
        <button type="button" onClick={() => answerDraftDiscard(true)}>{wt(locale, 'discard')}</button>
        <button type="button" autoFocus onClick={() => answerDraftDiscard(false)}>{wt(locale, 'cancel')}</button>
      </div>}
      {aiReplaceOpen && <div className="runtime-banner runtime-banner--notice" role="alertdialog" aria-label={at(locale, 'replaceQuestion')} onKeyDown={(event) => {
        if (event.key === 'Escape') { event.preventDefault(); answerReplaceTarget(false); }
      }}>
        <span>{at(locale, 'replaceQuestion')}</span>
        <button type="button" onClick={() => answerReplaceTarget(true)}>{at(locale, 'replaceConfirm')}</button>
        <button type="button" autoFocus onClick={() => answerReplaceTarget(false)}>{at(locale, 'cancel')}</button>
      </div>}
      {sourceLanguage === 'ja' && !health?.tokenizerReady && <div className="runtime-banner" role="status">{wt(locale, 'japanesePending')}</div>}
      {workspace.state.draftDirty && workspace.state.needsRetranslation && <div className="runtime-banner runtime-banner--notice" role="status">{wt(locale, 'staleDraft')}</div>}
      {workspace.state.error && <div className="runtime-banner runtime-banner--error" role="alert">{wt(locale, 'error')} <span className="error-code">({workspace.state.error.code})</span><button type="button" aria-label={translate(locale, 'close')} onClick={() => workspace.reportError(null)}>×</button></div>}
      {error && <div className="runtime-banner runtime-banner--error" role="alert">{localizedDocumentError(locale, error)} <span className="error-code">({error.code})</span><button type="button" aria-label={translate(locale, 'close')} onClick={() => setError(null)}>×</button></div>}
      {notice && <div className="runtime-banner runtime-banner--notice" role="status">{translate(locale, notice)} <button type="button" aria-label={translate(locale, 'close')} onClick={() => setNotice(null)}>×</button></div>}
      {settings.preservedInvalidFile && <div className="runtime-banner runtime-banner--notice" role="alert"><span>{ft(locale, 'settingsInvalid')}</span>{native && <button type="button" onClick={() => { void settings.replaceInvalid(); }}>{ft(locale, 'replaceSettings')}</button>}</div>}
      {documents.notice && <div className="runtime-banner runtime-banner--notice" role="status">{ft(locale, documents.notice)}<button type="button" aria-label={translate(locale, 'close')} onClick={() => documents.setNotice(null)}>×</button></div>}
      {documents.warnings.length > 0 && <div className="runtime-banner runtime-banner--notice document-warnings" role="status"><details><summary>{ft(locale, 'warnings')} ({documents.warnings.length})</summary><ul>{documents.warnings.map((warning, index) => <li key={index}>{localizedDocumentWarning(locale, warning)}</li>)}</ul></details><button type="button" aria-label={translate(locale, 'close')} onClick={() => documents.setWarnings([])}>×</button></div>}
      <main className="workspace-host" aria-label={translate(locale, 'workspace')} inert={documents.busy}>
        {settings.ready && <DockWorkspace
          locale={locale}
          sourceLanguage={sourceLanguage}
          theme={theme}
          initialLayout={preferences.dockLayout}
          onLayoutChange={layoutChanged}
          renderPane={(pane) => pane === 'ai'
            ? <section className="workspace-editor-pane workspace-editor-pane--ai" aria-label="AI"><AiPane ai={ai} profiles={aiProfiles} workspace={workspace} locale={locale} onManageProfiles={openAiProfiles} /></section>
            : <WorkspacePane pane={pane} controller={workspace} locale={locale} onReveal={() => { if (dockApi) { showPane(dockApi, 'source', locale, sourceLanguage); showPane(dockApi, 'meanings', locale, sourceLanguage); } }} />}
          onReady={setDockApi}
          onVisiblePanesChange={setVisiblePanes}
          onNotice={layoutNotice}
        />}
      </main>
      <footer className="status-bar" aria-label={translate(locale, 'workspace')}>
        <span>{documents.name ?? translate(locale, 'untitled')} · {wt(locale, workspace.state.dirty ? 'dirty' : 'saved')}</span>
        <span>{sourceCount} {wt(locale, 'sourceCount')}</span>
        <span title={ft(locale, 'cursor')}>{cursorLine}:{cursorColumn} · {workspace.state.sourceSelection.start}–{workspace.state.sourceSelection.end}</span>
        {documents.busy && <span role="status">{ft(locale, 'busy')}</span>}
        {workspace.state.translating && <span role="status">{wt(locale, 'translating')}</span>}
        {ai.state.status === 'running' && <span role="status">AI · {at(locale, 'running')} {ai.state.chunks.filter((chunk) => chunk.completed).length}/{ai.state.chunkCount}</span>}
        {workspace.state.result && <span>{workspace.state.result.segments.filter((segment) => !segment.unknown && segment.meanings.length > 0).length}/{workspace.state.result.segments.filter((segment) => segment.unknown || segment.meanings.length > 0).length} · {translate(locale, 'dictionaryReady')} r{workspace.state.result.dictionaryRevision}</span>}
        <span className="status-spacer" />
        {health && <>
          <span>{translate(locale, health.dictionaryReady ? 'dictionaryReady' : 'dictionaryNotReady')}</span>
          <span>{translate(locale, health.tokenizerReady ? 'tokenizerReady' : 'tokenizerNotReady')}</span>
        </>}
        <span className={error ? 'status-error' : undefined} role="status">{translate(locale, connectionKey)}{health ? ` · ${health.version}` : ''}</span>
      </footer>
      <InterfaceSettings
        open={configOpen}
        locale={locale}
        theme={theme}
        workspace={workspace}
        aiProfiles={aiProfiles}
        aiBusy={ai.state.status === 'running' || ai.preparing || ai.inFlight || ai.state.status === 'review'}
        focusAiProfiles={focusAiProfiles}
        preferences={preferences}
        onPreferencesChange={setPreferences}
        onLocaleChange={(value) => setPreferences((current) => ({ ...current, locale: value }))}
        onThemeChange={(value) => setPreferences((current) => ({ ...current, theme: value }))}
        onClose={() => setConfigOpen(false)}
        onDictionaries={(tab) => { setConfigOpen(false); setDictionaryTab(tab); }}
      />
      {dictionaryTab && <DictionaryDialog locale={locale} initialTab={dictionaryTab} onClose={() => setDictionaryTab(null)} />}
      <DocumentDialogs controller={documents} locale={locale} sourceLanguage={sourceLanguage} />
    </div>
  );
}
