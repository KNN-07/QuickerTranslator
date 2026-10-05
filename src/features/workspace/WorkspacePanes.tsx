import { useEffect, useMemo, useRef, useState } from 'react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { CodePane } from '../../components/CodePane';
import type { CodeContext, CodeHighlight } from '../../components/CodePane';
import { TargetPane } from '../../components/TargetPane';
import { invokeCommand, nativeAvailable, toAppError } from '../../lib/ipc';
import { paneTitle, translate } from '../../lib/i18n';
import type { DictionaryProvenance, PaneId, UiLocale } from '../../lib/types';
import type { AlignedPane, WorkspaceController } from './useWorkspaceController';
import { reorderable } from './state';
import type { DictionaryLookupEntry } from './state';
import { DictionaryOverride } from './DictionaryOverride';
import type { OverrideSubject } from './DictionaryOverride';
import { wt } from './i18n';
import { kindLabel } from '../dictionaries/i18n';

interface Props { pane: Exclude<PaneId, 'ai'>; controller: WorkspaceController; locale: UiLocale; onReveal: () => void }
function Provenance({ items, locale }: { items: DictionaryProvenance[]; locale: UiLocale }) {
  return <ul className="dictionary-provenance">{items.map((item, index) => <li key={`${item.dictionaryId}-${index}`}><strong>{item.dictionaryName}</strong> · {wt(locale, item.layer)}<span className="provenance-kind">{kindLabel(locale, item.kind)}</span>{item.sourceUrls.map((url) => <span className="provenance-url" key={url}>{url}</span>)}</li>)}</ul>;
}
function MeaningsPane({ controller, locale, onEdit, wrap }: { controller: WorkspaceController; locale: UiLocale; onEdit: (subject: OverrideSubject) => void; wrap: boolean }) {
  const { state } = controller;
  const segment = state.result?.sourceRevision === state.sourceRevision ? state.result.segments.find((item) => item.id === state.selectedSegmentId) : undefined;
  const [entries, setEntries] = useState<DictionaryLookupEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [lookupError, setLookupError] = useState<string | null>(null);
  useEffect(() => {
    setEntries([]); setLookupError(null);
    if (!segment || !nativeAvailable()) { setLoading(false); return; }
    let disposed = false;
    setLoading(true);
    const headwords = [...new Set([segment.surface, segment.lemma].filter((value): value is string => Boolean(value)))];
    void Promise.all(headwords.map((headword) => invokeCommand<DictionaryLookupEntry[]>('lookup_dictionary_entries', { headword, language: state.sourceLanguage }))).then((values) => {
      if (disposed) return;
      const seen = new Set<string>();
      setEntries(values.flat().filter((entry) => { const key = `${entry.kind}:${entry.headword}`; if (seen.has(key)) return false; seen.add(key); return true; }));
    }).catch((failure: unknown) => { if (!disposed) setLookupError(toAppError(failure).code); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [segment, state.sourceLanguage, state.dictionaryRevision]);
  if (!segment) return <p className="pane-empty-message">{wt(locale, 'chooseWord')}</p>;
  return <div className={`meanings-content${wrap ? '' : ' meanings-no-wrap'}`} style={{ fontSize: state.viewState.fonts.meanings }}>
    <h3>{segment.surface}</h3>
    {segment.lemma && <p><span>{wt(locale, 'lemma')}:</span> {segment.lemma}</p>}
    {segment.reading && <p><span>{wt(locale, 'reading')}:</span> <button type="button" className="meaning-reading" title={wt(locale, 'insertReading')} onClick={() => controller.insertTarget(segment.reading!)}>{segment.reading}</button></p>}
    {segment.partOfSpeech && <p><span>{wt(locale, 'partOfSpeech')}:</span> {segment.partOfSpeech}</p>}
    {segment.unknown && <p className="unknown-marker">{wt(locale, 'unknown')}</p>}
    <ol className="meaning-choices">{segment.meanings.map((meaning, index) => <li key={`${index}-${meaning}`}>
      <button type="button" className="meaning-choice" title={wt(locale, 'chooseMeaning')} onClick={() => controller.chooseMeaning(segment.id, meaning)}>{meaning}</button>
      <button type="button" title={wt(locale, 'insert')} onClick={() => controller.insertTarget(meaning)}>↳ {paneTitle(locale, state.sourceLanguage, 'target')}</button>
      {nativeAvailable() && <button type="button" title={wt(locale, 'saveOverride')} onClick={() => onEdit({ headword: segment.surface, segment, chosenMeaning: meaning })}>✎</button>}
    </li>)}</ol>
    <Provenance items={segment.provenance} locale={locale} />
    {nativeAvailable() && <button type="button" onClick={() => onEdit({ headword: segment.surface, segment })}>{wt(locale, 'editOverride')}</button>}
    {loading && <p role="status">{wt(locale, 'lookup')}</p>}
    {lookupError && <p className="workspace-error" role="alert">{wt(locale, 'error')} ({lookupError})</p>}
    {!loading && entries.length === 0 && <p className="pane-note">{wt(locale, 'noMeaning')}</p>}
    {entries.map((entry) => <section className="auxiliary-entry" key={`${entry.kind}:${entry.headword}`}>
      <h4>{entry.headword} · {kindLabel(locale, entry.kind)}</h4>
      {entry.reading && <p>{entry.reading}</p>}
      {entry.partOfSpeech && <p>{entry.partOfSpeech}</p>}
      {entry.meanings.map((meaning, index) => <p className="auxiliary-meaning" key={index}>{meaning}</p>)}
      <Provenance items={entry.provenance} locale={locale} />
      {['primaryNames', 'secondaryNames', 'vietPhrase', 'hanViet', 'japanese'].includes(entry.kind) && <button type="button" onClick={() => onEdit({ headword: entry.headword, lookup: entry, kind: entry.kind })}>{wt(locale, 'editOverride')}</button>}
    </section>)}
  </div>;
}

export function WorkspacePane({ pane, controller, locale, onReveal }: Props) {
  const { state, draft } = controller;
  const [context, setContext] = useState<CodeContext | null>(null);
  const [subject, setSubject] = useState<OverrideSubject | null>(null);
  const menu = useRef<HTMLDivElement>(null);
  const segment = state.result?.segments.find((item) => item.id === context?.segmentId);
  const highlights = useMemo<CodeHighlight[]>(() => {
    if (pane === 'target' || pane === 'meanings' || state.result?.sourceRevision !== state.sourceRevision || (pane === 'singleMeaning' && !draft.aligned)) return [];
    const selected = new Set(controller.selectedIds);
    return state.result.segments.map((item) => ({
      id: item.id, range: pane === 'source' ? item.sourceRange : pane === 'readings' ? item.readingsRange : pane === 'phrases' ? item.phrasesRange : draft.ranges.get(item.id) ?? item.singleMeaningRange,
      matched: item.meanings.length > 0 || item.provenance.length > 0,
      selected: selected.has(item.id), draggable: pane === 'singleMeaning' && reorderable(state.result!, item),
    }));
  }, [pane, state.result, state.sourceRevision, controller.selectedIds, draft.ranges, draft.aligned]);
  useEffect(() => {
    if (!context) return;
    menu.current?.querySelector<HTMLButtonElement>('button')?.focus();
    const close = (event: PointerEvent | KeyboardEvent) => {
      if (event instanceof KeyboardEvent) { if (event.key === 'Escape') setContext(null); return; }
      if (event.target instanceof Node && !menu.current?.contains(event.target)) setContext(null);
    };
    window.addEventListener('pointerdown', close); window.addEventListener('keydown', close);
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', close); };
  }, [context]);
  const select = (id: string) => { controller.selectSegment(id); onReveal(); };
  const openContext = (value: CodeContext) => { setContext(value); if (value.segmentId && (pane !== 'source' || !value.selection)) select(value.segmentId); };
  const action = (handler: () => void) => { handler(); setContext(null); };
  const copy = async (text: string) => {
    try { if (nativeAvailable()) await writeText(text); else await navigator.clipboard.writeText(text); }
    catch (failure: unknown) { controller.reportError(toAppError(failure)); }
  };
  let content;
  if (pane === 'target') content = <TargetPane editor={controller.targetEditor} fontSize={state.viewState.fonts.target} locale={locale} wrap={state.viewState.wraps?.target ?? state.viewState.wrap} />;
  else if (pane === 'meanings') content = <MeaningsPane controller={controller} locale={locale} onEdit={setSubject} wrap={state.viewState.wraps?.meanings ?? state.viewState.wrap} />;
  else {
    const kind: AlignedPane = pane;
    const text = kind === 'source' ? state.sourceText : kind === 'readings' ? state.result?.readings ?? '' : kind === 'phrases' ? state.result?.phrases ?? '' : draft.text;
    content = <>
      <CodePane text={text} locale={locale} label={paneTitle(locale, state.sourceLanguage, pane)} readOnly={kind !== 'source'} fontSize={state.viewState.fonts[kind]} wrap={state.viewState.wraps?.[kind] ?? state.viewState.wrap} resetKey={state.editorEpoch}
        initialLine={kind === 'readings' ? state.viewState.legacyScrollIndices?.[0] : kind === 'phrases' ? state.viewState.legacyScrollIndices?.[1] : kind === 'singleMeaning' ? state.viewState.legacyScrollIndices?.[2] : undefined}
        selection={kind === 'source' ? state.sourceSelection : undefined}
        getInitialState={kind === 'source' ? controller.getSourceEditorState : undefined}
        highlights={highlights} onView={(view) => controller.registerView(kind, view)}
        onChange={kind === 'source' ? (text) => controller.setSourceText(text, true) : undefined} onSelection={kind === 'source' ? controller.setSourceSelection : undefined}
        onComposition={kind === 'source' ? controller.setComposing : undefined} onSegmentClick={select} onContext={openContext}
        onMove={kind === 'singleMeaning' ? controller.moveToken : undefined} onDrop={kind === 'singleMeaning' ? controller.dropToken : undefined}
        onScroll={(offset) => controller.syncScroll(kind, offset)} />
      {kind !== 'source' && <div className="editor-pane-note">{kind === 'singleMeaning' && state.draftDirty ? wt(locale, 'dirty') + ' · ' : ''}{translate(locale, 'offlineGlossNotice')}{kind === 'singleMeaning' && <span title={wt(locale, 'reorderHint')}> · Alt+Shift+←/→</span>}</div>}
    </>;
  }
  const editSubject = pane === 'source' && context?.selection ? { headword: context.selection, ...(context.selection === segment?.surface ? { segment } : {}) }
    : segment ? { headword: segment.surface, segment } : context?.selection ? { headword: context.selection } : null;
  return <section className={`workspace-editor-pane workspace-editor-pane--${pane}`} aria-label={paneTitle(locale, state.sourceLanguage, pane)}>
    {content}
    {context && <div ref={menu} className="editor-context-menu" role="menu" style={{ left: Math.max(0, Math.min(context.x, window.innerWidth - 320)), top: Math.max(0, Math.min(context.y, window.innerHeight - 330)) }}>
      {context.selection && <><button type="button" role="menuitem" onClick={() => action(() => { void copy(context.selection); })}>{wt(locale, 'copy')}</button><button type="button" role="menuitem" onClick={() => action(() => { controller.insertTarget(context.selection); })}>{wt(locale, 'insertSelection')}</button></>}
      {segment?.reading && <button type="button" role="menuitem" onClick={() => action(() => { controller.insertTarget(segment.reading!); })}>{wt(locale, 'insertReading')}: {segment.reading}</button>}
      {segment?.meanings.map((meaning, index) => <div className="context-meaning-row" key={index}><button type="button" role="menuitem" title={wt(locale, 'chooseMeaning')} onClick={() => action(() => controller.chooseMeaning(segment.id, meaning))}>{index + 1}. {meaning}</button><button type="button" role="menuitem" title={wt(locale, 'insert')} onClick={() => action(() => { controller.insertTarget(meaning); })}>↳ {paneTitle(locale, state.sourceLanguage, 'target')}</button></div>)}
      {pane === 'singleMeaning' && segment && <><button type="button" role="menuitem" onClick={() => action(() => controller.moveToken(segment.id, -1))}>{wt(locale, 'moveLeft')}</button><button type="button" role="menuitem" onClick={() => action(() => controller.moveToken(segment.id, 1))}>{wt(locale, 'moveRight')}</button></>}
      {nativeAvailable() && editSubject && <button type="button" role="menuitem" onClick={() => action(() => setSubject(editSubject))}>{wt(locale, 'editOverride')}</button>}
      <button type="button" role="menuitem" onClick={() => setContext(null)}>{wt(locale, 'close')}</button>
    </div>}
    {subject && <DictionaryOverride subject={subject} language={state.sourceLanguage} locale={locale} onClose={() => setSubject(null)} onError={controller.reportError} />}
  </section>;
}
