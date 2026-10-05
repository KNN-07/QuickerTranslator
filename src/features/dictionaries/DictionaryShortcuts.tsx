import { useEffect, useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { invokeCommand, toAppError } from '../../lib/ipc';
import type { ShortcutPreview, ShortcutRecord } from '../../lib/dictionaryTypes';
import type { AppError, UiLocale } from '../../lib/types';
import { dt } from './i18n';
import { EncodingSelect, ImportReport } from './ImportReport';

export function DictionaryShortcuts({ locale, revision, onError, onNotice, onChanged, onDirtyChange, onBusyChange }: {
  locale: UiLocale; revision: number; onError: (error: AppError) => void;
  onNotice: (notice: 'shortcutSaved' | 'shortcutDeleted' | 'importedNotice' | 'exported') => void;
  onChanged: () => Promise<void>; onDirtyChange: (dirty: boolean) => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [records, setRecords] = useState<ShortcutRecord[] | null>(null);
  const [key, setKey] = useState('');
  const [value, setValue] = useState('');
  const [baseline, setBaseline] = useState({ key: '', value: '' });
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [pending, setPending] = useState<{ record: ShortcutRecord | null } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  const [path, setPath] = useState('');
  const [encoding, setEncoding] = useState('');
  const [preview, setPreview] = useState<ShortcutPreview | null>(null);
  const dirty = key !== baseline.key || value !== baseline.value;

  useEffect(() => { onDirtyChange(dirty); }, [dirty, onDirtyChange]);
  useEffect(() => { onBusyChange(busy); }, [busy, onBusyChange]);
  useEffect(() => {
    let disposed = false;
    invokeCommand<ShortcutRecord[]>('list_shortcuts').then((result) => { if (!disposed) setRecords(result); }).catch((error: unknown) => { if (!disposed) onError(toAppError(error)); });
    return () => { disposed = true; };
  }, [revision, onError]);

  function selectRecord(record: ShortcutRecord | null) {
    setKey(record?.key ?? ''); setValue(record?.value ?? ''); setSelectedKey(record?.key ?? null);
    setBaseline(record ?? { key: '', value: '' }); setPending(null); setConfirmDelete(false);
  }

  async function saveRecord() {
    if (busy) return;
    if (!key.trim() || !value.trim()) { onError({ code: 'invalidShortcut', message: '' }); return; }
    setBusy(true);
    try {
      const record = { key: key.toLowerCase(), value };
      await invokeCommand<number>('save_shortcut', { request: record });
      selectRecord(record);
      await onChanged(); onNotice('shortcutSaved');
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  async function deleteRecord() {
    if (busy || !selectedKey) return;
    setBusy(true);
    try {
      await invokeCommand<number>('delete_shortcut', { key: selectedKey });
      selectRecord(null);
      await onChanged(); onNotice('shortcutDeleted');
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  async function chooseImport() {
    if (busy) return;
    setBusy(true);
    try {
      const chosen = await open({ title: dt(locale, 'importShortcuts'), multiple: false, directory: false, filters: [{ name: 'Shortcuts.txt', extensions: ['txt'] }, { name: dt(locale, 'allFiles'), extensions: ['*'] }] });
      if (typeof chosen !== 'string') return;
      setPath(chosen); setPreview(null);
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  async function previewImport() {
    if (busy || !path) return;
    setBusy(true); setPreview(null);
    try { setPreview(await invokeCommand<ShortcutPreview>('preview_shortcuts_import', { request: { path, encoding: encoding || null } })); }
    catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  async function commitImport() {
    if (busy || !preview) return;
    setBusy(true);
    try {
      await invokeCommand<number>('commit_shortcuts_import', { previewId: preview.previewId });
      setPreview(null);
      await onChanged(); onNotice('importedNotice');
    } catch (error: unknown) { setPreview(null); onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  async function exportRecords() {
    if (busy) return;
    setBusy(true);
    try {
      const destination = await save({ title: dt(locale, 'exportShortcuts'), defaultPath: 'Shortcuts.txt', filters: [{ name: 'Shortcuts.txt', extensions: ['txt'] }] });
      if (!destination) return;
      await invokeCommand('export_shortcuts', { destination }); onNotice('exported');
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); }
  }

  return <section className="dictionary-shortcuts">
    <p>{dt(locale, 'shortcutNotice')}</p>
    <div className="dictionary-actions"><button type="button" disabled={busy} onClick={() => dirty ? setPending({ record: null }) : selectRecord(null)}>{dt(locale, 'add')}</button><button type="button" disabled={busy} onClick={() => void chooseImport()}>{dt(locale, 'importShortcuts')}</button><button type="button" disabled={busy} onClick={() => void exportRecords()}>{dt(locale, 'exportShortcuts')}</button></div>
    <div className="dictionary-entry-columns">
      <div className="dictionary-results"><ul>{records?.map((record) => <li key={record.key}><button type="button" disabled={busy} aria-pressed={selectedKey === record.key} onClick={() => dirty ? setPending({ record }) : selectRecord(record)}><strong>{record.key}</strong><span>{record.value}</span></button></li>)}</ul>{records === null ? <p>{dt(locale, 'loading')}</p> : records.length === 0 && <p>{dt(locale, 'noShortcuts')}</p>}</div>
      <div className="dictionary-editor">
        {pending && <div className="dictionary-warning" role="alert"><p>{dt(locale, 'discardEdits')}</p><div className="dictionary-actions"><button type="button" onClick={() => selectRecord(pending.record)}>{dt(locale, 'discard')}</button><button type="button" onClick={() => setPending(null)}>{dt(locale, 'cancel')}</button></div></div>}
        <form onSubmit={(event) => { event.preventDefault(); void saveRecord(); }}><fieldset disabled={busy}><legend>{dt(locale, 'shortcuts')}</legend>
          <label>{dt(locale, 'shortcutKey')}<input required value={key} readOnly={selectedKey !== null} onChange={(event) => setKey(event.target.value)} /></label>
          <label>{dt(locale, 'shortcutValue')}<textarea required rows={4} value={value} onChange={(event) => setValue(event.target.value)} /></label>
          <div className="dictionary-actions"><button type="submit" disabled={!dirty}>{busy ? dt(locale, 'working') : dt(locale, 'save')}</button><button type="button" disabled={selectedKey === null} onClick={() => setConfirmDelete(true)}>{dt(locale, 'delete')}</button></div>
        </fieldset></form>
        {confirmDelete && <div className="dictionary-warning" role="alert"><p>{dt(locale, 'confirmDelete')}</p><div className="dictionary-actions"><button type="button" disabled={busy} onClick={() => void deleteRecord()}>{dt(locale, 'delete')}</button><button type="button" disabled={busy} onClick={() => setConfirmDelete(false)}>{dt(locale, 'cancel')}</button></div></div>}
      </div>
    </div>
    {path && <fieldset disabled={busy}><legend>{dt(locale, 'importShortcuts')}</legend><p className="dictionary-path">{path}</p>
      <div className="dictionary-form-row"><EncodingSelect locale={locale} value={encoding} onChange={(next) => { setEncoding(next); setPreview(null); }} /><button type="button" onClick={() => void previewImport()}>{dt(locale, 'preview')}</button></div>
      {preview && <><ImportReport locale={locale} preview={preview} /><div className="dictionary-table-scroll"><table><thead><tr><th>{dt(locale, 'shortcutKey')}</th><th>{dt(locale, 'shortcutValue')}</th></tr></thead><tbody>{preview.sample.map((record) => <tr key={record.key}><td>{record.key}</td><td>{record.value}</td></tr>)}</tbody></table></div><button type="button" onClick={() => void commitImport()}>{dt(locale, 'import')}</button></>}
    </fieldset>}
  </section>;
}
