import { useEffect, useRef, useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { invokeCommand, toAppError } from '../../lib/ipc';
import type { DictionaryCatalog, DictionaryMetadata, EntryHistory, EntryKey, EntryRecord, EntryView, SearchResult } from '../../lib/dictionaryTypes';
import type { AppError, DictionaryKind, SourceLanguage, UiLocale } from '../../lib/types';
import { DICTIONARY_KINDS, dt, kindLabel, languageLabel, layerLabel } from './i18n';
import { EntryHistoryRecord } from './EntryHistoryRecord';

interface EntryDraft {
  dictionaryId: string; name: string; language: SourceLanguage; kind: DictionaryKind;
  headword: string; meanings: string; reading: string; pos: string; aliases: string; sourceUrls: string; payload: string;
}

export function DictionaryEntries({ locale, catalog, onError, onNotice, onChanged, onDirtyChange, onBusyChange }: {
  locale: UiLocale; catalog: DictionaryCatalog; onError: (error: AppError) => void;
  onNotice: (notice: 'saved' | 'exported') => void; onChanged: () => Promise<void>; onDirtyChange: (dirty: boolean) => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [query, setQuery] = useState('');
  const [search, setSearch] = useState({ query: '', language: 'zh' as SourceLanguage, kind: '' as DictionaryKind | '', dictionaryId: '', offset: 0 });
  const [results, setResults] = useState<SearchResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<EntryView | null>(null);
  const [draft, setDraft] = useState<EntryDraft | null>(null);
  const [baseline, setBaseline] = useState('');
  const [history, setHistory] = useState<EntryHistory[] | null>(null);
  const [historyKey, setHistoryKey] = useState<EntryKey | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [deleted, setDeleted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [pendingSelection, setPendingSelection] = useState<{ entry: EntryView | null } | null>(null);
  const historyGeneration = useRef(0);
  const dirty = draft !== null && JSON.stringify(draft) !== baseline;

  useEffect(() => { onDirtyChange(dirty); }, [dirty, onDirtyChange]);
  useEffect(() => { onBusyChange(busy); }, [busy, onBusyChange]);
  useEffect(() => () => { historyGeneration.current++; }, []);

  useEffect(() => {
    let disposed = false;
    setLoading(true);
    invokeCommand<SearchResult>('search_dictionary_entries', { request: {
      ...search, kind: search.kind || null, dictionaryId: search.dictionaryId || null, limit: 60,
    } }).then((value) => { if (!disposed) setResults(value); }).catch((error: unknown) => {
      if (!disposed) { setResults(null); onError(toAppError(error)); }
    }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [search, catalog.revision, onError]);

  async function loadHistory(entry: EntryKey) {
    const generation = ++historyGeneration.current;
    setHistory(null);
    setHistoryKey(entry);
    setHistoryLoading(true);
    try {
      const value = await invokeCommand<EntryHistory[]>('dictionary_entry_history', { request: entry });
      if (generation === historyGeneration.current) setHistory(value);
    } catch (error: unknown) {
      if (generation === historyGeneration.current) onError(toAppError(error));
    } finally {
      if (generation === historyGeneration.current) setHistoryLoading(false);
    }
  }

  function selectEntry(entry: EntryView | null) {
    setSelected(entry);
    setDeleted(false);
    setConfirmDelete(false);
    setPendingSelection(null);
    const metadata = catalog.dictionaries.find((item) => item.id === (entry?.dictionaryId ?? search.dictionaryId));
    const next: EntryDraft = {
      dictionaryId: entry?.dictionaryId ?? metadata?.id ?? '', name: metadata?.name ?? '',
      language: entry?.language ?? metadata?.language ?? search.language, kind: entry?.kind ?? metadata?.kind ?? (search.kind || (search.language === 'ja' ? 'japanese' : 'vietPhrase')),
      headword: entry?.entry.headword ?? query, meanings: entry?.entry.meanings.join('\n') ?? '', reading: entry?.entry.reading ?? '', pos: entry?.entry.pos ?? '',
      aliases: entry?.entry.aliases.join('\n') ?? '', sourceUrls: entry?.entry.sourceUrls.join('\n') ?? '', payload: entry?.entry.payload ?? '',
    };
    setDraft(next);
    setBaseline(JSON.stringify(next));
    if (next.dictionaryId && next.headword) void loadHistory({ dictionaryId: next.dictionaryId, headword: next.headword });
    else { historyGeneration.current++; setHistory(null); setHistoryLoading(false); }
  }

  function requestSelection(entry: EntryView | null) {
    if (dirty) setPendingSelection({ entry });
    else selectEntry(entry);
  }

  async function saveEntry() {
    if (!draft || busy) return;
    const meanings = draft.meanings.split(/\r?\n/).filter((value) => value.trim().length > 0);
    if (!draft.headword.trim() || (draft.kind !== 'ignored' && meanings.length === 0) || (!draft.dictionaryId && !draft.name.trim())) {
      onError({ code: 'invalidDictionaryEntry', message: '' }); return;
    }
    setBusy(true);
    try {
      let dictionaryId = draft.dictionaryId;
      if (!dictionaryId) {
        const metadata = await invokeCommand<DictionaryMetadata>('save_dictionary_metadata', { request: { id: null, name: draft.name, language: draft.language, kind: draft.kind } });
        dictionaryId = metadata.id;
        setDraft((current) => current ? { ...current, dictionaryId } : current);
      }
      const entry: EntryRecord = {
        headword: draft.headword, meanings, reading: draft.reading || null, pos: draft.pos || null,
        aliases: draft.aliases.split(/\r?\n/).filter((value) => value.trim().length > 0),
        sourceUrls: draft.sourceUrls.split(/\r?\n/).filter((value) => value.trim().length > 0), payload: draft.payload || null,
      };
      await invokeCommand<number>('save_dictionary_entry', { request: { dictionaryId, entry } });
      let next = { ...draft, dictionaryId };
      const refreshed = await invokeCommand<SearchResult>('search_dictionary_entries', { request: { query: entry.headword, dictionaryId, language: draft.language, kind: draft.kind, offset: 0, limit: 100 } });
      const view = refreshed.entries.find((item) => item.entry.headword === entry.headword);
      if (view) {
        setSelected(view);
        next = { ...next, meanings: view.entry.meanings.join('\n'), reading: view.entry.reading ?? '', pos: view.entry.pos ?? '', aliases: view.entry.aliases.join('\n'), sourceUrls: view.entry.sourceUrls.join('\n'), payload: view.entry.payload ?? '' };
      }
      setDraft(next);
      setBaseline(JSON.stringify(next));
      await loadHistory({ dictionaryId, headword: entry.headword });
      setDeleted(false);
      await onChanged();
      onNotice('saved');
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  async function deleteEntry() {
    if (!selected || busy) return;
    setBusy(true);
    try {
      await invokeCommand<number>('delete_dictionary_entry', { request: { dictionaryId: selected.dictionaryId, headword: selected.entry.headword } });
      setDeleted(true);
      setConfirmDelete(false);
      if (draft) setBaseline(JSON.stringify(draft));
      await Promise.all([onChanged(), loadHistory({ dictionaryId: selected.dictionaryId, headword: selected.entry.headword })]);
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  async function exportDictionary() {
    const dictionaryId = draft?.dictionaryId || search.dictionaryId;
    if (!dictionaryId || busy) return;
    setBusy(true);
    try {
      const metadata = catalog.dictionaries.find((item) => item.id === dictionaryId);
      const destination = await save({ title: dt(locale, 'export'), defaultPath: `${metadata?.name.replace(/[\\/:*?"<>|]/g, '_') || 'dictionary'}.txt`, filters: [{ name: dt(locale, 'dictionaryFiles'), extensions: ['txt'] }] });
      if (!destination) return;
      await invokeCommand('export_dictionary', { request: { dictionaryId, destination } });
      onNotice('exported');
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  return <section className="dictionary-entries">
    <form className="dictionary-filter" onSubmit={(event) => { event.preventDefault(); setSearch((current) => ({ ...current, query, offset: 0 })); }}>
      <label>{dt(locale, 'language')}<select value={search.language} disabled={busy} onChange={(event) => setSearch((current) => ({ ...current, language: event.target.value as SourceLanguage, dictionaryId: '', offset: 0 }))}>
        <option value="zh">{languageLabel(locale, 'zh')}</option><option value="ja">{languageLabel(locale, 'ja')}</option>
      </select></label>
      <label>{dt(locale, 'kind')}<select value={search.kind} disabled={busy} onChange={(event) => setSearch((current) => ({ ...current, kind: event.target.value as DictionaryKind | '', dictionaryId: '', offset: 0 }))}>
        <option value="">{dt(locale, 'allKinds')}</option>{DICTIONARY_KINDS.map((kind) => <option key={kind} value={kind}>{kindLabel(locale, kind)}</option>)}
      </select></label>
      <label>{dt(locale, 'dictionary')}<select value={search.dictionaryId} disabled={busy} onChange={(event) => setSearch((current) => ({ ...current, dictionaryId: event.target.value, offset: 0 }))}>
        <option value="">{dt(locale, 'allDictionaries')}</option>{catalog.dictionaries.filter((item) => item.language === search.language && (!search.kind || item.kind === search.kind)).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select></label>
      <label className="dictionary-query">{dt(locale, 'searchLabel')}<input type="search" value={query} placeholder={dt(locale, 'searchHint')} onChange={(event) => setQuery(event.target.value)} /></label>
      <button type="submit" disabled={loading || busy}>{dt(locale, 'search')}</button>
      <button type="button" disabled={busy} onClick={() => requestSelection(null)}>{dt(locale, 'add')}</button>
      <button type="button" disabled={busy || !(draft?.dictionaryId || search.dictionaryId)} onClick={() => void exportDictionary()}>{dt(locale, 'export')}</button>
    </form>
    <div className="dictionary-entry-columns">
      <div className="dictionary-results" aria-busy={loading}>
        <p role="status">{loading ? dt(locale, 'loading') : results ? `${dt(locale, 'results')}: ${results.total.toLocaleString(locale)}` : ''}</p>
        {results?.total === 0 && <p>{dt(locale, 'noResults')}</p>}
        <ul>{results?.entries.map((view) => <li key={`${view.dictionaryId}:${view.entry.headword}`}>
          <button type="button" disabled={busy} aria-pressed={selected?.dictionaryId === view.dictionaryId && selected.entry.headword === view.entry.headword} onClick={() => requestSelection(view)}>
            <strong>{view.entry.headword}</strong><span>{view.entry.meanings.join(' / ') || view.entry.reading}</span>
            <small>{kindLabel(locale, view.kind)} · {layerLabel(locale, view.layer)}</small>
          </button>
        </li>)}</ul>
        <div className="dictionary-actions">
          <button type="button" disabled={loading || search.offset === 0} onClick={() => setSearch((current) => ({ ...current, offset: Math.max(0, current.offset - 60) }))}>{dt(locale, 'previous')}</button>
          <button type="button" disabled={loading || !results || search.offset + 60 >= results.total} onClick={() => setSearch((current) => ({ ...current, offset: current.offset + 60 }))}>{dt(locale, 'more')}</button>
        </div>
      </div>
      <div className="dictionary-editor">
        {pendingSelection && <div className="dictionary-warning" role="alert"><p>{dt(locale, 'discardEdits')}</p><div className="dictionary-actions"><button type="button" onClick={() => selectEntry(pendingSelection.entry)}>{dt(locale, 'discard')}</button><button type="button" onClick={() => setPendingSelection(null)}>{dt(locale, 'cancel')}</button></div></div>}
        {!draft ? <p>{dt(locale, 'noSelection')}</p> : <>
          <form onSubmit={(event) => { event.preventDefault(); void saveEntry(); }}>
            <fieldset disabled={busy}>
              <legend>{selected ? `${dt(locale, 'edit')}: ${selected.entry.headword}` : dt(locale, 'add')}</legend>
              {deleted && <p className="dictionary-warning">{dt(locale, 'deleted')}</p>}
              {!selected && <>
                <label>{dt(locale, 'chooseDictionary')}<select value={draft.dictionaryId} onChange={(event) => {
                  const metadata = catalog.dictionaries.find((item) => item.id === event.target.value);
                  setDraft({ ...draft, dictionaryId: event.target.value, ...(metadata ? { language: metadata.language, kind: metadata.kind, name: metadata.name } : {}) });
                }}><option value="">{dt(locale, 'newDictionary')}</option>{catalog.dictionaries.map((item) => <option key={item.id} value={item.id}>{item.name} — {languageLabel(locale, item.language)} · {kindLabel(locale, item.kind)}</option>)}</select></label>
                {!draft.dictionaryId && <div className="dictionary-form-row">
                  <label>{dt(locale, 'dictionaryName')}<input required value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></label>
                  <label>{dt(locale, 'language')}<select value={draft.language} onChange={(event) => setDraft({ ...draft, language: event.target.value as SourceLanguage, kind: event.target.value === 'ja' ? 'japanese' : 'vietPhrase' })}><option value="zh">{languageLabel(locale, 'zh')}</option><option value="ja">{languageLabel(locale, 'ja')}</option></select></label>
                  <label>{dt(locale, 'kind')}<select value={draft.kind} onChange={(event) => setDraft({ ...draft, kind: event.target.value as DictionaryKind })}>{DICTIONARY_KINDS.map((kind) => <option key={kind} value={kind}>{kindLabel(locale, kind)}</option>)}</select></label>
                </div>}
              </>}
              <label>{dt(locale, 'headword')}<input required value={draft.headword} readOnly={selected !== null} onChange={(event) => setDraft({ ...draft, headword: event.target.value })} /></label>
              {draft.kind === 'ignored' ? <p>{dt(locale, 'ignoredHint')}</p> : <label>{dt(locale, draft.kind === 'hanViet' ? 'hanMeanings' : ['cedict', 'babylon', 'lacViet', 'thieuChuu', 'auxiliary'].includes(draft.kind) ? 'meaningsText' : 'meanings')}<textarea required rows={4} value={draft.meanings} onChange={(event) => setDraft({ ...draft, meanings: event.target.value })} /></label>}
              {draft.kind === 'hanViet' && <p>{dt(locale, 'hanHint')}</p>}{draft.kind === 'rules' && <p>{dt(locale, 'ruleHint')}</p>}
              {['cedict', 'babylon', 'lacViet', 'thieuChuu', 'auxiliary'].includes(draft.kind) && <p>{dt(locale, 'lookupOnlyNotice')}</p>}
              <div className="dictionary-form-row">{draft.kind !== 'hanViet' && <label>{dt(locale, 'reading')}<input value={draft.reading} onChange={(event) => setDraft({ ...draft, reading: event.target.value })} /></label>}<label>{dt(locale, 'pos')}<input value={draft.pos} onChange={(event) => setDraft({ ...draft, pos: event.target.value })} /></label></div>
              <details><summary>{dt(locale, 'aliases')} / {dt(locale, 'source')}</summary>
                <label>{dt(locale, 'aliases')}<textarea rows={2} value={draft.aliases} onChange={(event) => setDraft({ ...draft, aliases: event.target.value })} /></label>
                <label>{dt(locale, 'sourceUrls')}<textarea rows={2} value={draft.sourceUrls} onChange={(event) => setDraft({ ...draft, sourceUrls: event.target.value })} /></label>
                <label>{dt(locale, 'payload')}<textarea rows={2} value={draft.payload} onChange={(event) => setDraft({ ...draft, payload: event.target.value })} /></label>
              </details>
              <p className="dictionary-muted">{dt(locale, 'overridesNotice')}</p>
              <div className="dictionary-actions"><button type="submit" disabled={selected !== null && !dirty && !deleted}>{busy ? dt(locale, 'working') : dt(locale, 'save')}</button><button type="button" disabled={!selected || deleted} onClick={() => setConfirmDelete(true)}>{dt(locale, 'delete')}</button></div>
            </fieldset>
          </form>
          {confirmDelete && <div className="dictionary-warning" role="alert"><p>{dt(locale, 'confirmDelete')}</p><p>{dt(locale, 'deleteNotice')}</p><div className="dictionary-actions"><button type="button" disabled={busy} onClick={() => void deleteEntry()}>{dt(locale, 'delete')}</button><button type="button" disabled={busy} onClick={() => setConfirmDelete(false)}>{dt(locale, 'cancel')}</button></div></div>}
          {selected && <details open><summary>{dt(locale, 'provenance')}</summary><ul>{selected.provenance.map((item, index) => <li key={index}>{item.dictionaryName} · {kindLabel(locale, item.kind)} · {layerLabel(locale, item.layer)}{item.sourceUrls.map((url) => <div key={url} className="dictionary-source-url">{url}</div>)}</li>)}</ul></details>}
          {draft.dictionaryId && draft.headword && <>
            <button type="button" disabled={busy || historyLoading} onClick={() => void loadHistory({ dictionaryId: draft.dictionaryId, headword: draft.headword })}>{dt(locale, 'history')}</button>
            {historyKey && historyKey.dictionaryId === draft.dictionaryId && historyKey.headword === draft.headword && <details open><summary>{dt(locale, 'history')}</summary>{historyLoading ? <p>{dt(locale, 'loading')}</p> : history?.length === 0 ? <p>{dt(locale, 'noHistory')}</p> : history?.map((item) => <details key={item.id}><summary>{item.timestamp} · {dt(locale, item.action === 'delete' ? 'deleteAction' : item.action === 'import' ? 'importAction' : item.action === 'add' ? 'createAction' : 'updateAction')}</summary>
              <div className="dictionary-history"><div><h4>{dt(locale, 'before')}</h4><EntryHistoryRecord locale={locale} entry={item.before} /></div><div><h4>{dt(locale, 'after')}</h4><EntryHistoryRecord locale={locale} entry={item.after} /></div></div>
            </details>)}</details>}
          </>}
        </>}
      </div>
    </div>
    <p className="dictionary-muted">{dt(locale, 'exportNotice')}</p>
  </section>;
}
