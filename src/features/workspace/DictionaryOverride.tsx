import { useEffect, useRef, useState } from 'react';
import { invokeCommand, toAppError } from '../../lib/ipc';
import type { DictionaryCatalog, DictionaryMetadata, EntryRecord, EntryView, SearchResult } from '../../lib/dictionaryTypes';
import type { AppError, DictionaryKind, SourceLanguage, TranslationSegment, UiLocale } from '../../lib/types';
import { wt } from './i18n';
import type { DictionaryLookupEntry } from './state';

export interface OverrideSubject { headword: string; segment?: TranslationSegment; entry?: EntryView; lookup?: DictionaryLookupEntry; chosenMeaning?: string; kind?: DictionaryKind }
interface Props { subject: OverrideSubject; language: SourceLanguage; locale: UiLocale; onClose: () => void; onError: (error: AppError) => void }
export function DictionaryOverride({ subject, language, locale, onClose, onError }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const initialKind = subject.kind ?? (language === 'ja' ? 'japanese' : subject.segment?.provenance.find((item) => ['primaryNames', 'secondaryNames', 'vietPhrase', 'hanViet'].includes(item.kind))?.kind ?? 'vietPhrase');
  const [kind, setKind] = useState<DictionaryKind>(initialKind);
  const [headword, setHeadword] = useState(subject.headword);
  const original = subject.entry?.entry;
  const initialMeanings = original?.meanings ?? subject.lookup?.meanings ?? subject.segment?.meanings ?? [];
  const orderedMeanings = subject.chosenMeaning ? [subject.chosenMeaning, ...initialMeanings.filter((meaning) => meaning !== subject.chosenMeaning)] : initialMeanings;
  const [meanings, setMeanings] = useState(orderedMeanings.join('/'));
  const [reading, setReading] = useState(original?.reading ?? subject.lookup?.reading ?? subject.segment?.reading ?? '');
  const [catalog, setCatalog] = useState<DictionaryCatalog | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  useEffect(() => {
    dialog.current?.showModal(); let disposed = false;
    void invokeCommand<DictionaryCatalog>('dictionary_catalog').then((value) => { if (!disposed) setCatalog(value); }).catch((failure: unknown) => { if (!disposed) setError(toAppError(failure)); });
    return () => { disposed = true; };
  }, []);
  const save = async () => {
    const values = meanings.split(/[\/|]/u).filter((value) => value.trim().length > 0);
    if (busy || !catalog || !headword.trim() || values.length === 0) return;
    setBusy(true); setError(null);
    try {
      const originalId = subject.entry?.dictionaryId ?? (subject.lookup?.provenance ?? subject.segment?.provenance)?.find((item) => item.kind === kind && item.layer !== 'bundled')?.dictionaryId;
      let metadata = catalog.dictionaries.find((item) => item.id === originalId && !item.bundled && item.language === language && item.kind === kind)
        ?? catalog.dictionaries.find((item) => !item.bundled && item.language === language && item.kind === kind);
      if (!metadata) metadata = await invokeCommand<DictionaryMetadata>('save_dictionary_metadata', { request: {
        id: null, name: `QuickTranslator ${language} ${kind}`, language, kind,
      } });
      const existing = await invokeCommand<SearchResult>('search_dictionary_entries', { request: {
        query: headword, dictionaryId: metadata.id, language, kind, offset: 0, limit: 1,
      } });
      const base = existing.entries.find((item) => item.entry.headword === headword)?.entry;
      const entry: EntryRecord = {
        headword, meanings: values, reading: reading || (kind === 'hanViet' ? values[0] ?? null : null), pos: base?.pos ?? subject.lookup?.partOfSpeech ?? subject.segment?.partOfSpeech ?? null,
        aliases: base?.aliases ?? [], sourceUrls: base?.sourceUrls ?? [], payload: base?.payload ?? null,
      };
      await invokeCommand<number>('save_dictionary_entry', { request: { dictionaryId: metadata.id, entry } });
      onClose();
    } catch (failure: unknown) { const next = toAppError(failure); setError(next); onError(next); }
    finally { setBusy(false); }
  };
  return <dialog ref={dialog} className="settings-dialog override-dialog" onClose={onClose} onCancel={(event) => { if (busy) event.preventDefault(); }} aria-labelledby="override-title">
    <h2 id="override-title">{wt(locale, 'overrideTitle')}</h2>
    <label><span>{wt(locale, 'kind')}</span><select value={kind} onChange={(event) => setKind(event.target.value as DictionaryKind)}>
      {(language === 'ja' ? ['japanese'] as const : ['primaryNames', 'secondaryNames', 'vietPhrase', 'hanViet'] as const).map((value) => <option key={value} value={value}>{wt(locale, value)}</option>)}
    </select></label>
    <label><span>{wt(locale, 'headword')}</span><input value={headword} onChange={(event) => setHeadword(event.target.value)} autoFocus /></label>
    <label><span>{wt(locale, 'meanings')}</span><textarea value={meanings} onChange={(event) => setMeanings(event.target.value)} rows={3} /></label>
    <label><span>{wt(locale, 'reading')}</span><input value={reading} onChange={(event) => setReading(event.target.value)} /></label>
    {error && <p role="alert" className="workspace-error">{wt(locale, 'error')} ({error.code})</p>}
    <div className="dialog-actions"><button type="button" disabled={busy} onClick={() => dialog.current?.close()}>{wt(locale, 'cancel')}</button><button type="button" disabled={busy || !catalog || !headword.trim() || !meanings.trim()} onClick={() => { void save(); }}>{wt(locale, 'saveOverride')}</button></div>
  </dialog>;
}
