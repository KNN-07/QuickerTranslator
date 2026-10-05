import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { save } from '@tauri-apps/plugin-dialog';
import { invokeCommand, nativeAvailable, toAppError } from '../../lib/ipc';
import type { DictionaryCatalog, DictionaryStorageStatus } from '../../lib/dictionaryTypes';
import type { AppError, UiLocale } from '../../lib/types';
import { DictionaryEntries } from './DictionaryEntries';
import { DictionaryImports } from './DictionaryImports';
import { DictionaryShortcuts } from './DictionaryShortcuts';
import { dictionaryError, dt, kindLabel, languageLabel } from './i18n';
import type { DictionaryText } from './i18n';
import '../../styles/dictionaries.css';

export type DictionaryTab = 'entries' | 'imports' | 'shortcuts' | 'attribution';
const TABS: DictionaryTab[] = ['entries', 'imports', 'shortcuts', 'attribution'];

export function DictionaryDialog({ locale, initialTab = 'entries', onClose }: {
  locale: UiLocale; initialTab?: DictionaryTab; onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const mounted = useRef(true);
  const loadGeneration = useRef(0);
  const [tab, setTab] = useState<DictionaryTab>(initialTab);
  const [catalog, setCatalog] = useState<DictionaryCatalog | null>(null);
  const [storage, setStorage] = useState<DictionaryStorageStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<DictionaryText | null>(null);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<'close' | DictionaryTab | null>(null);
  const [confirmRepair, setConfirmRepair] = useState(false);
  const native = nativeAvailable();
  const reportError = useCallback((failure: AppError) => { setError(failure); setNotice(null); }, []);
  const reportNotice = useCallback((message: DictionaryText) => { setNotice(message); setError(null); }, []);
  const reportDirty = useCallback((value: boolean) => { setDirty(value); }, []);
  const reportBusy = useCallback((value: boolean) => { setBusy(value); }, []);

  const refresh = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setLoading(true);
    try {
      const [nextCatalog, nextStorage] = await Promise.all([
        invokeCommand<DictionaryCatalog>('dictionary_catalog'),
        invokeCommand<DictionaryStorageStatus>('dictionary_storage_status'),
      ]);
      if (mounted.current && generation === loadGeneration.current) {
        setCatalog((current) => !current || nextCatalog.revision >= current.revision ? nextCatalog : current);
        setStorage(nextStorage);
      }
    } catch (failure: unknown) {
      if (mounted.current && generation === loadGeneration.current) reportError(toAppError(failure));
    } finally {
      if (mounted.current && generation === loadGeneration.current) setLoading(false);
    }
  }, [reportError]);

  useEffect(() => {
    mounted.current = true;
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const unlisten = listen<{ revision: number }>('dictionaries-changed', () => { if (!disposed) void refresh(); });
    unlisten.then(() => { if (!disposed) void refresh(); }).catch((failure: unknown) => {
      if (!disposed) { reportError(toAppError(failure)); void refresh(); }
    });
    return () => { disposed = true; void unlisten.then((stop) => stop()).catch(() => {}); };
  }, [native, refresh, reportError]);

  function requestAction(action: 'close' | DictionaryTab) {
    if (busy || action === tab) return;
    if (dirty) { setPending(action); return; }
    setPending(null); setDirty(false); setError(null); setNotice(null);
    if (action === 'close') dialogRef.current?.close();
    else setTab(action);
  }

  async function repairDatabase() {
    if (busy) return;
    setBusy(true);
    try {
      const destination = await save({ title: dt(locale, 'chooseBackup'), defaultPath: 'user-data-backup.sqlite3', filters: [{ name: 'SQLite', extensions: ['sqlite3', 'db'] }] });
      if (!destination) return;
      await invokeCommand<number>('repair_dictionary_database', { request: { backupDestination: destination } });
      setConfirmRepair(false); await refresh(); reportNotice('repairSuccess');
    } catch (failure: unknown) { reportError(toAppError(failure)); }
    finally { setBusy(false); }
  }

  return <dialog className="dictionary-dialog" ref={dialogRef} aria-labelledby="dictionary-dialog-title" onClose={onClose} onCancel={(event) => { event.preventDefault(); requestAction('close'); }}>
    <header className="dictionary-dialog-header"><h2 id="dictionary-dialog-title">{dt(locale, 'title')}</h2><button type="button" disabled={busy} onClick={() => requestAction('close')} aria-label={dt(locale, 'close')}>×</button></header>
    <div className="dictionary-tabs" role="tablist" aria-label={dt(locale, 'title')}>
      {TABS.map((item, index) => <button key={item} ref={(element) => { tabRefs.current[index] = element; }} type="button" role="tab" id={`dictionary-tab-${item}`} aria-controls={`dictionary-panel-${item}`} aria-selected={tab === item} tabIndex={tab === item ? 0 : -1} disabled={busy} onClick={() => requestAction(item)} onKeyDown={(event) => {
        const next = event.key === 'ArrowRight' ? (index + 1) % TABS.length : event.key === 'ArrowLeft' ? (index + TABS.length - 1) % TABS.length : event.key === 'Home' ? 0 : event.key === 'End' ? TABS.length - 1 : -1;
        const nextTab = TABS[next];
        if (next >= 0 && nextTab) { event.preventDefault(); tabRefs.current[next]?.focus(); requestAction(nextTab); }
      }}>{dt(locale, item)}</button>)}
    </div>
    <div className="dictionary-dialog-body">
      {!native ? TABS.map((item) => <div key={item} role="tabpanel" id={`dictionary-panel-${item}`} aria-labelledby={`dictionary-tab-${item}`} hidden={tab !== item}><p className="dictionary-warning" role="status">{dt(locale, 'nativeUnavailable')}</p></div>) : <>
        {error && <p className="dictionary-error" role="alert">{dictionaryError(locale, error)}</p>}
        {notice && <p className="dictionary-notice" role="status">{dt(locale, notice)}</p>}
        {pending && <div className="dictionary-warning" role="alert"><p>{dt(locale, 'discardEdits')}</p><div className="dictionary-actions"><button type="button" disabled={busy} onClick={() => {
          const action = pending; setPending(null); setDirty(false);
          if (action === 'close') dialogRef.current?.close(); else setTab(action);
        }}>{dt(locale, 'discard')}</button><button type="button" onClick={() => setPending(null)}>{dt(locale, 'cancel')}</button></div></div>}
        {storage && !storage.ready && <section className="dictionary-warning" aria-labelledby="dictionary-repair-title"><h3 id="dictionary-repair-title">{dt(locale, 'repairTitle')}</h3>
          {storage.error && <p>{dictionaryError(locale, storage.error)}</p>}<p className="dictionary-path">{storage.path}</p><p>{dt(locale, 'repairNotice')}</p>
          {!confirmRepair ? <button type="button" disabled={busy} onClick={() => setConfirmRepair(true)}>{dt(locale, 'repair')}</button> : <div className="dictionary-actions"><button type="button" disabled={busy} onClick={() => void repairDatabase()}>{dt(locale, 'confirmRepair')}</button><button type="button" disabled={busy} onClick={() => setConfirmRepair(false)}>{dt(locale, 'cancel')}</button></div>}
        </section>}
        {catalog ? <>
          {TABS.map((item) => <div key={item} role="tabpanel" id={`dictionary-panel-${item}`} aria-labelledby={`dictionary-tab-${item}`} hidden={tab !== item}>
            {tab === item && (item === 'entries' ? <DictionaryEntries locale={locale} catalog={catalog} onError={reportError} onNotice={reportNotice} onChanged={refresh} onDirtyChange={reportDirty} onBusyChange={reportBusy} />
              : item === 'imports' ? <DictionaryImports locale={locale} catalog={catalog} onError={reportError} onNotice={reportNotice} onChanged={refresh} onBusyChange={reportBusy} />
                : item === 'shortcuts' ? <DictionaryShortcuts locale={locale} revision={catalog.revision} onError={reportError} onNotice={reportNotice} onChanged={refresh} onDirtyChange={reportDirty} onBusyChange={reportBusy} />
                  : <section className="dictionary-attribution"><p>{dt(locale, 'dataNotice')}</p><h3>{dt(locale, 'attribution')}</h3><pre>{catalog.attribution}</pre>
                    {Object.entries(catalog.licenses).sort(([a], [b]) => a.localeCompare(b)).map(([name, text]) => <details key={name}><summary>{dt(locale, 'license')}: {name}</summary><pre>{text}</pre></details>)}
                    <details><summary>{dt(locale, 'manifest')}</summary><pre>{JSON.stringify(catalog.manifest, null, 2)}</pre></details>
                  </section>)}
          </div>)}
          <details className="dictionary-coverage"><summary>{dt(locale, 'coverage')} · {dt(locale, 'revision')}: {catalog.revision.toLocaleString(locale)}</summary>
            <div className="dictionary-table-scroll"><table><thead><tr><th>{dt(locale, 'dictionary')}</th><th>{dt(locale, 'language')}</th><th>{dt(locale, 'kind')}</th><th>{dt(locale, 'activeEntries')}</th><th>{dt(locale, 'aliasesCount')}</th><th>{dt(locale, 'imported')}</th><th>{dt(locale, 'editsCount')}</th><th>{dt(locale, 'tombstonesCount')}</th></tr></thead><tbody>
              {catalog.dictionaries.map((dictionary) => <tr key={dictionary.id}><td>{dictionary.name}{dictionary.bundled && <small>{dt(locale, 'bundled')}</small>}</td><td>{languageLabel(locale, dictionary.language)}</td><td>{kindLabel(locale, dictionary.kind)}</td><td>{dictionary.entryCount.toLocaleString(locale)}</td><td>{dictionary.aliasCount.toLocaleString(locale)}</td><td>{dictionary.importedCount.toLocaleString(locale)}</td><td>{dictionary.editedCount.toLocaleString(locale)}</td><td>{dictionary.tombstoneCount.toLocaleString(locale)}</td></tr>)}
            </tbody></table></div>
          </details>
        </> : loading && <p role="status">{dt(locale, 'loading')}</p>}
      </>}
    </div>
    <footer className="dictionary-dialog-footer"><span role="status">{busy ? dt(locale, 'working') : loading ? dt(locale, 'loading') : ''}</span><div className="dictionary-actions">{native && <button type="button" disabled={busy || loading} onClick={() => void refresh()}>{dt(locale, 'refresh')}</button>}<button type="button" disabled={busy} onClick={() => requestAction('close')}>{dt(locale, 'close')}</button></div></footer>
  </dialog>;
}
