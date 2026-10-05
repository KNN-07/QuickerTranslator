import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { DockviewReact } from 'dockview-react';
import type { DockviewApi, DockviewReadyEvent, DockviewTheme, IDockviewPanelProps, SerializedDockview } from 'dockview-react';
import { paneTitle, translate } from '../lib/i18n';
import type { TranslationKey } from '../lib/i18n';
import type { PaneId, SourceLanguage, ThemePreference, UiLocale } from '../lib/types';

export const PANE_IDS: readonly PaneId[] = [
  'source', 'readings', 'phrases', 'singleMeaning', 'meanings', 'target', 'ai',
];

export type WorkspacePaneRenderer = (pane: PaneId) => ReactNode;

export interface DockWorkspaceProps {
  locale: UiLocale;
  sourceLanguage: SourceLanguage;
  theme: ThemePreference;
  initialLayout?: SerializedDockview | null;
  onLayoutChange?: (layout: SerializedDockview) => void;
  renderPane?: WorkspacePaneRenderer;
  onReady?: (api: DockviewApi) => void;
  onVisiblePanesChange?: (panes: PaneId[]) => void;
  onNotice?: (key: TranslationKey) => void;
}

interface PaneContextValue {
  locale: UiLocale;
  sourceLanguage: SourceLanguage;
  renderPane?: WorkspacePaneRenderer;
}

const PaneContext = createContext<PaneContextValue>({ locale: 'vi', sourceLanguage: 'zh' });
const hintKeys: Record<PaneId, TranslationKey> = {
  source: 'sourceHint', readings: 'readingsHint', phrases: 'phrasesHint',
  singleMeaning: 'singleHint', meanings: 'meaningsHint', target: 'targetHint', ai: 'aiHint',
};

function WorkspacePane({ params }: IDockviewPanelProps<{ paneId: PaneId }>) {
  const { locale, sourceLanguage, renderPane } = useContext(PaneContext);
  const pane = params.paneId;
  if (renderPane) return <>{renderPane(pane)}</>;

  const note: TranslationKey | null = pane === 'target'
    ? 'manualTargetNotice'
    : pane === 'ai'
      ? 'aiPrivacyNotice'
      : pane === 'readings' || pane === 'phrases' || pane === 'singleMeaning'
        ? 'offlineGlossNotice'
        : null;
  return (
    <section
      className={`workspace-pane workspace-pane--${pane}`}
      aria-label={paneTitle(locale, sourceLanguage, pane)}
      tabIndex={0}
    >
      <p className="pane-empty-message">{translate(locale, hintKeys[pane])}</p>
      {note && <p className="pane-note">{translate(locale, note)}</p>}
    </section>
  );
}

function WorkspaceWatermark() {
  const { locale } = useContext(PaneContext);
  return <p className="workspace-watermark">{translate(locale, 'reopenPane')}</p>;
}

const components = { workspace: WorkspacePane };
const lightTheme: DockviewTheme = {
  name: 'quicktranslator-light', className: 'quicktranslator-dock', colorScheme: 'light',
  gap: 3, dndOverlayMounting: 'absolute', dndPanelOverlay: 'group', dndTabIndicator: 'line',
};
const darkTheme: DockviewTheme = { ...lightTheme, name: 'quicktranslator-dark', colorScheme: 'dark' };

/** Relative splits create two columns before independently splitting each column. */
export function createDefaultLayout(api: DockviewApi, locale: UiLocale, language: SourceLanguage): void {
  api.clear();
  api.addPanel({ id: 'readings', component: 'workspace', params: { paneId: 'readings' }, title: paneTitle(locale, language, 'readings') });
  api.addPanel({ id: 'source', component: 'workspace', params: { paneId: 'source' }, title: paneTitle(locale, language, 'source'), position: { referencePanel: 'readings', direction: 'within' } });
  api.addPanel({ id: 'singleMeaning', component: 'workspace', params: { paneId: 'singleMeaning' }, title: paneTitle(locale, language, 'singleMeaning'), position: { referencePanel: 'readings', direction: 'right' }, initialWidth: api.width * 0.7 });
  api.addPanel({ id: 'phrases', component: 'workspace', params: { paneId: 'phrases' }, title: paneTitle(locale, language, 'phrases'), position: { referencePanel: 'singleMeaning', direction: 'within' }, inactive: true });
  api.addPanel({ id: 'ai', component: 'workspace', params: { paneId: 'ai' }, title: paneTitle(locale, language, 'ai'), position: { referencePanel: 'singleMeaning', direction: 'within' }, inactive: true });
  api.addPanel({ id: 'meanings', component: 'workspace', params: { paneId: 'meanings' }, title: paneTitle(locale, language, 'meanings'), position: { referencePanel: 'readings', direction: 'below' }, initialHeight: api.height * 0.42 });
  api.addPanel({ id: 'target', component: 'workspace', params: { paneId: 'target' }, title: paneTitle(locale, language, 'target'), position: { referencePanel: 'singleMeaning', direction: 'below' }, initialHeight: api.height * 0.4 });
  api.getPanel('source')?.group.api.setSize({ width: api.width * 0.3, height: api.height * 0.58 });
  api.getPanel('singleMeaning')?.group.api.setSize({ width: api.width * 0.7, height: api.height * 0.6 });
  api.getPanel('source')?.api.setActive();
  api.getPanel('singleMeaning')?.api.setActive();
}

/** Reopen closed outputs without disturbing the other groups or the manual target. */
export function showPane(api: DockviewApi, pane: PaneId, locale: UiLocale, language: SourceLanguage): void {
  const existing = api.getPanel(pane);
  if (existing) {
    existing.api.setActive();
    return;
  }
  const preferred: Record<PaneId, readonly PaneId[]> = {
    source: ['readings', 'meanings'], readings: ['source', 'meanings'],
    phrases: ['singleMeaning', 'ai', 'target'], singleMeaning: ['phrases', 'ai', 'target'],
    ai: ['singleMeaning', 'phrases', 'target'], meanings: ['source', 'readings'],
    target: ['singleMeaning', 'phrases', 'ai'],
  };
  const reference = preferred[pane].find((candidate) => api.getPanel(candidate));
  api.addPanel({
    id: pane, component: 'workspace', params: { paneId: pane }, title: paneTitle(locale, language, pane),
    ...(reference ? { position: { referencePanel: reference, direction: pane === 'meanings' || pane === 'target' ? 'below' as const : 'within' as const } } : {}),
  });
}

function validateLayout(layout: unknown): SerializedDockview {
  if (typeof layout !== 'object' || layout === null || !('grid' in layout) || !('panels' in layout)) {
    throw new Error('Invalid layout');
  }
  if (typeof layout.panels !== 'object' || layout.panels === null) throw new Error('Invalid panels');
  if ('popoutGroups' in layout && Array.isArray(layout.popoutGroups) && layout.popoutGroups.length > 0) {
    throw new Error('Browser popouts are not document windows');
  }
  for (const [id, panel] of Object.entries(layout.panels)) {
    if (!PANE_IDS.includes(id as PaneId) || typeof panel !== 'object' || panel === null) throw new Error('Unknown pane');
    if (panel.contentComponent !== 'workspace' || panel.params?.paneId !== id) throw new Error('Unknown component');
  }
  return layout as SerializedDockview;
}

export function DockWorkspace({ locale, sourceLanguage, theme, initialLayout, onLayoutChange, renderPane, onReady, onVisiblePanesChange, onNotice }: DockWorkspaceProps) {
  const [api, setApi] = useState<DockviewApi>();
  const context = useMemo(() => ({ locale, sourceLanguage, renderPane }), [locale, sourceLanguage, renderPane]);

  const ready = useCallback((event: DockviewReadyEvent) => {
    let restored = false;
    try {
      const layout = initialLayout ? validateLayout(initialLayout) : null;
      if (layout) {
        event.api.fromJSON(layout);
        restored = true;
      }
    } catch {
      onNotice?.('layoutRestoreError');
    }
    if (!restored) createDefaultLayout(event.api, locale, sourceLanguage);
    setApi(event.api);
    onReady?.(event.api);
  }, [locale, sourceLanguage, initialLayout, onReady, onNotice]);

  useEffect(() => {
    if (!api) return;
    for (const pane of PANE_IDS) api.getPanel(pane)?.api.setTitle(paneTitle(locale, sourceLanguage, pane));
    api.updateOptions({
      messages: {
        closeTab: (title) => `${translate(locale, 'close')}: ${title}`,
        closeTabPlain: () => translate(locale, 'close'),
      },
    });
  }, [api, locale, sourceLanguage]);

  useEffect(() => {
    if (!api) return;
    const update = () => {
      onVisiblePanesChange?.(PANE_IDS.filter((pane) => api.getPanel(pane)));
      onLayoutChange?.(api.toJSON());
    };
    update();
    const subscription = api.onDidLayoutChange(update);
    const activeSubscription = api.onDidActivePanelChange(update);
    return () => { subscription.dispose(); activeSubscription.dispose(); };
  }, [api, onVisiblePanesChange, onLayoutChange]);

  return (
    <PaneContext.Provider value={context}>
      <DockviewReact
        className="dock-workspace"
        components={components}
        watermarkComponent={WorkspaceWatermark}
        theme={theme === 'dark' ? darkTheme : lightTheme}
        disableFloatingGroups
        dndStrategy="pointer"
        onReady={ready}
        messages={{ closeTab: (title) => `${translate(locale, 'close')}: ${title}`, closeTabPlain: () => translate(locale, 'close') }}
      />
    </PaneContext.Provider>
  );
}
