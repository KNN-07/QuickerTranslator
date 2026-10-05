import { useEffect, useRef } from 'react';
import { paneTitle, translate } from '../lib/i18n';
import { PANE_IDS } from './DockWorkspace';
import type { PaneId, SourceLanguage, UiLocale } from '../lib/types';
import { ft } from '../features/documents/i18n';

export interface ToolbarProps {
  locale: UiLocale;
  sourceLanguage: SourceLanguage;
  native: boolean;
  creatingWindow: boolean;
  documentBusy: boolean;
  onOpen: () => void;
  onSave: (saveAs?: boolean) => void;
  onExport: () => void;
  onCloseDocument: () => void;
  onNavigate: (direction: -1 | 1) => void;
  canBack: boolean;
  canNext: boolean;
  onOriginalRtf?: () => void;
  onRecover?: () => void;
  visiblePanes: readonly PaneId[];
  onSourceLanguageChange: (language: SourceLanguage) => void;
  onNewWindow: () => void;
  onResetLayout: () => void;
  onPaneVisibilityChange: (pane: PaneId, visible: boolean) => void;
  onConfig: () => void;
  onReloadDictionaries: () => void;
  onRetranslate: () => void;
  onTranslateClipboard: () => void;
  onFindSource: () => void;
  onAiTranslate: () => void;
  translating: boolean;
}

export function Toolbar({ locale, sourceLanguage, native, creatingWindow, documentBusy, onOpen, onSave, onExport, onCloseDocument, onNavigate, canBack, canNext, onOriginalRtf, onRecover, visiblePanes, onSourceLanguageChange, onNewWindow, onResetLayout, onPaneVisibilityChange, onConfig, onReloadDictionaries, onRetranslate, onTranslateClipboard, onFindSource, onAiTranslate, translating }: ToolbarProps) {
  const toolbarRef = useRef<HTMLElement>(null);
  const unavailable = translate(locale, 'nativeUnavailable');

  useEffect(() => {
    const closeMenus = (event: PointerEvent | KeyboardEvent) => {
      if (event instanceof KeyboardEvent && event.key !== 'Escape') return;
      if (event instanceof PointerEvent && event.target instanceof Node && toolbarRef.current?.contains(event.target)) return;
      toolbarRef.current?.querySelectorAll('details[open]').forEach((element) => element.removeAttribute('open'));
    };
    document.addEventListener('pointerdown', closeMenus);
    document.addEventListener('keydown', closeMenus);
    return () => {
      document.removeEventListener('pointerdown', closeMenus);
      document.removeEventListener('keydown', closeMenus);
    };
  }, []);

  return (
    <nav className="toolbar" aria-label={translate(locale, 'workspace')} ref={toolbarRef} inert={documentBusy}>
      <details className="toolbar-menu">
        <summary>{translate(locale, 'file')}</summary>
        <div className="menu-popover">
          <button type="button" disabled={!native || creatingWindow} title={native ? 'Ctrl/Cmd+Shift+N' : unavailable} onClick={onNewWindow}>{translate(locale, 'newWindow')}<kbd>⇧⌘/Ctrl+N</kbd></button>
          <hr />
          <button type="button" disabled={!native || documentBusy} title={native ? 'Ctrl/Cmd+O' : unavailable} onClick={onOpen}>{translate(locale, 'open')}<kbd>Ctrl/Cmd+O</kbd></button>
          <button type="button" disabled={!native || documentBusy} title={native ? 'Ctrl/Cmd+S' : unavailable} onClick={() => onSave()}>{translate(locale, 'save')}<kbd>Ctrl/Cmd+S</kbd></button>
          <button type="button" disabled={!native || documentBusy} title={native ? 'Ctrl/Cmd+Shift+S' : unavailable} onClick={() => onSave(true)}>{translate(locale, 'saveAs')}<kbd>⇧⌘/Ctrl+S</kbd></button>
          <button type="button" disabled={!native || documentBusy} title={native ? 'Ctrl/Cmd+E' : unavailable} onClick={onExport}>{translate(locale, 'export')}<kbd>Ctrl/Cmd+E</kbd></button>
          {onOriginalRtf && <button type="button" disabled={!native || documentBusy} onClick={onOriginalRtf}>{ft(locale, 'originalRtf')}</button>}
          {onRecover && <button type="button" disabled={!native || documentBusy} onClick={onRecover}>{ft(locale, 'recoveryTitle')}</button>}
          <button type="button" disabled={!native || documentBusy} title={native ? undefined : unavailable} onClick={onCloseDocument}>{ft(locale, 'closeDocument')}</button>
          <hr />
          <button type="button" onClick={onFindSource} title="Ctrl/Cmd+F">{translate(locale, 'find')}<kbd>Ctrl/Cmd+F</kbd></button>
        </div>
      </details>
      <details className="toolbar-menu">
        <summary>{translate(locale, 'layout')}</summary>
        <div className="menu-popover layout-menu">
          <button type="button" onClick={onResetLayout}>{translate(locale, 'resetLayout')}</button>
          <hr />
          <p className="menu-label">{translate(locale, 'showPanels')}</p>
          {PANE_IDS.map((pane) => (
            <label key={pane}>
              <input type="checkbox" checked={visiblePanes.includes(pane)} onChange={(event) => onPaneVisibilityChange(pane, event.target.checked)} />
              {paneTitle(locale, sourceLanguage, pane)}
            </label>
          ))}
        </div>
      </details>
      <button type="button" disabled={!native} title={native ? undefined : unavailable} onClick={onTranslateClipboard}>{translate(locale, 'translateClipboard')}</button>
      <button type="button" disabled={!native} aria-busy={translating} title={native ? undefined : unavailable} onClick={onRetranslate}>{translate(locale, 'retranslate')}</button>
      <button type="button" onClick={onReloadDictionaries}>{translate(locale, 'reloadDictionaries')}</button>
      <button type="button" onClick={onConfig}>{translate(locale, 'config')}</button>
      <span className="toolbar-separator" aria-hidden="true" />
      <label className="language-selector">
        <span className="sr-only">{translate(locale, 'sourceLanguage')}</span>
        <select value={sourceLanguage} disabled={documentBusy} aria-label={translate(locale, 'sourceLanguage')} onChange={(event) => onSourceLanguageChange(event.target.value === 'ja' ? 'ja' : 'zh')}>
          <option value="zh">{translate(locale, 'chinese')}</option>
          <option value="ja">{translate(locale, 'japanese')}</option>
        </select>
      </label>
      <button type="button" disabled={!native} title={native ? undefined : unavailable} onClick={onAiTranslate}>{translate(locale, 'aiTranslate')}</button>
      <span className="toolbar-spacer" />
      <button type="button" disabled={!native || documentBusy || !canBack} title="Alt+←" onClick={() => onNavigate(-1)}>← {translate(locale, 'back')}</button>
      <button type="button" disabled={!native || documentBusy || !canNext} title="Alt+→" onClick={() => onNavigate(1)}>{translate(locale, 'next')} →</button>
    </nav>
  );
}
