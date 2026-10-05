import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, toAppError } from '../../lib/ipc';
import type { ConfigPreview, DictionaryCatalog, DictionaryMetadata, ImportFormat, ImportPreview, ImportRequest } from '../../lib/dictionaryTypes';
import type { AppError, DictionaryKind, SourceLanguage, UiLocale } from '../../lib/types';
import { DICTIONARY_KINDS, dictionaryError, dt, kindLabel, languageLabel } from './i18n';
import { EncodingSelect, ImportIssues, ImportReport } from './ImportReport';

export function DictionaryImports({ locale, catalog, onError, onNotice, onChanged, onBusyChange }: {
  locale: UiLocale; catalog: DictionaryCatalog; onError: (error: AppError) => void;
  onNotice: (notice: 'importedNotice') => void; onChanged: () => Promise<void>; onBusyChange: (busy: boolean) => void;
}) {
  const [mode, setMode] = useState<'single' | 'config'>('single');
  const [single, setSingle] = useState<ImportRequest>({ path: '', name: '', language: 'zh', kind: 'vietPhrase', dictionaryId: null, encoding: null, format: 'legacy' });
  const [configPath, setConfigPath] = useState('');
  const [configEncoding, setConfigEncoding] = useState('');
  const [remappings, setRemappings] = useState<Record<string, string>>({});
  const [config, setConfig] = useState<ConfigPreview | null>(null);
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [fileEncodings, setFileEncodings] = useState<Record<string, string>>({});
  const [previews, setPreviews] = useState<ImportPreview[]>([]);
  const [previewFailures, setPreviewFailures] = useState<Record<string, AppError>>({});
  const [ruleAlgorithm, setRuleAlgorithm] = useState(catalog.ruleAlgorithm);
  const [busy, setBusy] = useState(false);

  function changeSingle(next: ImportRequest) {
    setSingle(next);
    setPreviews([]);
    setPreviewFailures({});
  }

  async function chooseFile() {
    if (busy) return;
    setBusy(true); onBusyChange(true);
    try {
      const path = await open({ title: dt(locale, 'chooseFile'), multiple: false, directory: false, filters: [{ name: dt(locale, 'dictionaryFiles'), extensions: ['txt', 'dic', 'dict', 'dat'] }, { name: dt(locale, 'allFiles'), extensions: ['*'] }] });
      if (typeof path !== 'string') return;
      setMode('single');
      changeSingle({ ...single, path, name: single.dictionaryId ? single.name : path.split(/[\\/]/).at(-1)?.replace(/\.[^.]+$/, '') || single.name });
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); onBusyChange(false); }
  }

  async function readConfig(path: string, encoding: string, mappings: Record<string, string>, retainSelection = false) {
    setPreviews([]); setPreviewFailures({});
    const value = await invokeCommand<ConfigPreview>('preview_dictionary_config', { request: { path, encoding: encoding || null, remappings: mappings } });
    setConfig(value);
    setRuleAlgorithm(value.ruleAlgorithm);
    setSelected((current) => Object.fromEntries(value.dictionaries.map((item) => [item.key, item.resolvedPath !== null && (!retainSelection || current[item.key] !== false || mappings[item.key] !== undefined)])));
  }

  async function chooseConfig() {
    if (busy) return;
    setBusy(true); onBusyChange(true);
    try {
      const path = await open({ title: dt(locale, 'chooseConfig'), multiple: false, directory: false, filters: [{ name: 'Dictionaries.config', extensions: ['config'] }, { name: dt(locale, 'allFiles'), extensions: ['*'] }] });
      if (typeof path !== 'string') return;
      setMode('config'); setConfigPath(path); setRemappings({}); setConfig(null); setSelected({}); setFileEncodings({});
      await readConfig(path, configEncoding, {});
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); onBusyChange(false); }
  }

  async function remap(key: string) {
    if (busy) return;
    setBusy(true); onBusyChange(true);
    try {
      const path = await open({ title: `${dt(locale, 'remap')} — ${key}`, multiple: false, directory: false });
      if (typeof path !== 'string') return;
      const mappings = { ...remappings, [key]: path };
      setRemappings(mappings);
      await readConfig(configPath, configEncoding, mappings, true);
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); onBusyChange(false); }
  }

  async function refreshConfig() {
    if (busy || !configPath) return;
    setBusy(true); onBusyChange(true);
    setConfig(null);
    try { await readConfig(configPath, configEncoding, remappings); }
    catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); onBusyChange(false); }
  }

  async function previewImport() {
    if (busy) return;
    setBusy(true); onBusyChange(true);
    setPreviews([]); setPreviewFailures({});
    try {
      if (mode === 'single') {
        const preview = await invokeCommand<ImportPreview>('preview_dictionary_import', { request: single });
        setPreviews([preview]);
      } else if (config) {
        const next: ImportPreview[] = [];
        const failures: Record<string, AppError> = {};
        for (const item of config.dictionaries.filter((item) => selected[item.key] && item.resolvedPath)) {
          const existing = catalog.dictionaries.find((dictionary) => !dictionary.bundled && dictionary.language === 'zh' && dictionary.kind === item.kind && dictionary.name === item.key);
          try {
            next.push(await invokeCommand<ImportPreview>('preview_dictionary_import', { request: {
              path: item.resolvedPath, name: item.key, language: 'zh', kind: item.kind, dictionaryId: existing?.id ?? null,
              encoding: fileEncodings[item.key] || null, format: item.kind === 'cedict' ? 'cedict' : item.kind === 'ignored' ? 'ignored' : 'legacy',
            } }));
          } catch (error: unknown) { failures[item.key] = toAppError(error); }
        }
        setPreviews(next); setPreviewFailures(failures);
      }
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); onBusyChange(false); }
  }

  async function reload(dictionary: DictionaryMetadata) {
    if (busy) return;
    setBusy(true); onBusyChange(true); setMode('single'); setPreviews([]); setPreviewFailures({});
    changeSingle({ path: dictionary.sourcePath || '', name: dictionary.name, language: dictionary.language, kind: dictionary.kind, dictionaryId: dictionary.id, encoding: dictionary.encoding, format: dictionary.format });
    try {
      const preview = await invokeCommand<ImportPreview>('reload_dictionary', { dictionaryId: dictionary.id });
      setPreviews([preview]);
    } catch (error: unknown) { onError(toAppError(error)); }
    finally { setBusy(false); onBusyChange(false); }
  }

  async function commitImport() {
    if (busy || previews.length === 0 || Object.keys(previewFailures).length > 0) return;
    setBusy(true); onBusyChange(true);
    try {
      await invokeCommand<number>('commit_dictionary_import', { request: { previewIds: previews.map((preview) => preview.previewId), ruleAlgorithm } });
      setPreviews([]); setPreviewFailures({});
      await onChanged();
      onNotice('importedNotice');
    } catch (error: unknown) { setPreviews([]); onError(toAppError(error)); }
    finally { setBusy(false); onBusyChange(false); }
  }

  const reloadable = catalog.dictionaries.filter((dictionary) => dictionary.sourcePath !== null && !dictionary.bundled);
  return <section className="dictionary-imports">
    <p>{dt(locale, 'importInstructions')}</p>
    <div className="dictionary-actions"><button type="button" disabled={busy} onClick={() => void chooseFile()}>{dt(locale, 'chooseFile')}</button><button type="button" disabled={busy} onClick={() => void chooseConfig()}>{dt(locale, 'chooseConfig')}</button></div>
    {mode === 'single' && single.path && <fieldset disabled={busy}>
      <legend>{dt(locale, 'sourceFile')}</legend><p className="dictionary-path">{single.path}</p>
      <label>{dt(locale, 'dictionary')}<select value={single.dictionaryId || ''} onChange={(event) => {
        const target = catalog.dictionaries.find((item) => item.id === event.target.value);
        changeSingle({ ...single, dictionaryId: target?.id ?? null, ...(target ? { name: target.name, language: target.language, kind: target.kind, format: target.format ?? (target.kind === 'cedict' ? 'cedict' : target.kind === 'ignored' ? 'ignored' : 'legacy') } : {}) });
      }}><option value="">{dt(locale, 'newDictionary')}</option>{catalog.dictionaries.filter((item) => !item.bundled).map((item) => <option key={item.id} value={item.id}>{item.name} — {languageLabel(locale, item.language)} · {kindLabel(locale, item.kind)}</option>)}</select></label>
      <div className="dictionary-form-row"><label>{dt(locale, 'dictionaryName')}<input value={single.name} readOnly={single.dictionaryId !== null} onChange={(event) => changeSingle({ ...single, name: event.target.value })} /></label>
        <label>{dt(locale, 'language')}<select value={single.language} disabled={single.dictionaryId !== null} onChange={(event) => changeSingle({ ...single, language: event.target.value as SourceLanguage, kind: event.target.value === 'ja' ? 'japanese' : 'vietPhrase' })}><option value="zh">{languageLabel(locale, 'zh')}</option><option value="ja">{languageLabel(locale, 'ja')}</option></select></label>
        <label>{dt(locale, 'kind')}<select value={single.kind} disabled={single.dictionaryId !== null} onChange={(event) => {
          const kind = event.target.value as DictionaryKind;
          changeSingle({ ...single, kind, format: kind === 'cedict' ? 'cedict' : kind === 'ignored' ? 'ignored' : 'legacy' });
        }}>{DICTIONARY_KINDS.map((kind) => <option key={kind} value={kind}>{kindLabel(locale, kind)}</option>)}</select></label>
      </div>
      <div className="dictionary-form-row"><EncodingSelect locale={locale} value={single.encoding || ''} onChange={(encoding) => changeSingle({ ...single, encoding: encoding || null })} />
        <label>{dt(locale, 'format')}<select value={single.format || 'legacy'} onChange={(event) => changeSingle({ ...single, format: event.target.value as ImportFormat })}><option value="legacy">{dt(locale, 'legacyFormat')}</option><option value="cedict">{dt(locale, 'cedictFormat')}</option><option value="ignored">{dt(locale, 'ignoredFormat')}</option></select></label>
      </div>
    </fieldset>}
    {mode === 'config' && configPath && <fieldset disabled={busy}>
      <legend>Dictionaries.config</legend><p className="dictionary-path">{configPath}</p>
      <div className="dictionary-form-row"><EncodingSelect locale={locale} value={configEncoding} onChange={(encoding) => { setConfigEncoding(encoding); setConfig(null); setPreviews([]); setPreviewFailures({}); }} /><button type="button" onClick={() => void refreshConfig()}>{dt(locale, 'preview')}</button></div>
      {config && <>
        <p>{dt(locale, 'detectedEncoding')}: <strong>{config.encoding}</strong></p><p>{dt(locale, 'selectResolved')}</p>
        <div className="dictionary-table-scroll"><table><thead><tr><th>{dt(locale, 'selected')}</th><th>{dt(locale, 'configKey')}</th><th>{dt(locale, 'path')}</th><th>{dt(locale, 'encoding')}</th><th>{dt(locale, 'sourceFile')}</th></tr></thead><tbody>
          {config.dictionaries.map((item) => <tr key={item.key}><td><input type="checkbox" aria-label={`${dt(locale, 'selected')} ${item.key}`} checked={selected[item.key] || false} disabled={!item.resolvedPath} onChange={(event) => { setSelected({ ...selected, [item.key]: event.target.checked }); setPreviews([]); setPreviewFailures({}); }} /></td>
            <td>{item.key}<small>{kindLabel(locale, item.kind)}</small></td><td className="dictionary-path">{item.resolvedPath || item.originalPath}<small className={!item.resolvedPath ? 'dictionary-warning-text' : ''}>{dt(locale, item.resolvedPath ? 'resolved' : 'unresolved')}</small></td>
            <td><EncodingSelect locale={locale} value={fileEncodings[item.key] || ''} disabled={!item.resolvedPath} onChange={(encoding) => { setFileEncodings({ ...fileEncodings, [item.key]: encoding }); setPreviews([]); setPreviewFailures({}); }} /></td>
            <td><button type="button" onClick={() => void remap(item.key)}>{dt(locale, 'remap')}</button></td></tr>)}
        </tbody></table></div>
        <ImportIssues locale={locale} issues={config.issues} />
        <details><summary>{dt(locale, 'rawPreview')}</summary><pre className="dictionary-raw-preview">{config.decodedText}</pre></details>
      </>}
    </fieldset>}
    {(mode === 'single' ? Boolean(single.path) : Boolean(config)) && <>
      <div className="dictionary-form-row"><label>{dt(locale, 'ruleMode')}<select value={ruleAlgorithm} disabled={busy} onChange={(event) => setRuleAlgorithm(Number(event.target.value))}><option value="1">{dt(locale, 'mode1')}</option><option value="2">{dt(locale, 'mode2')}</option><option value="3">{dt(locale, 'mode3')}</option></select></label>
        <button type="button" disabled={busy || (mode === 'config' && !config?.dictionaries.some((item) => selected[item.key] && item.resolvedPath))} onClick={() => void previewImport()}>{busy ? dt(locale, 'working') : dt(locale, 'preview')}</button>
      </div>
      {Object.entries(previewFailures).map(([key, error]) => <p role="alert" className="dictionary-error" key={key}>{key}: {dictionaryError(locale, error)}</p>)}
      {previews.map((preview) => <article className="dictionary-preview" key={preview.previewId}><h3>{preview.name} — {languageLabel(locale, preview.language)} · {kindLabel(locale, preview.kind)}</h3>
        <ImportReport locale={locale} preview={preview} />
        <details open><summary>{dt(locale, 'previewEntries')}</summary><div className="dictionary-table-scroll"><table><thead><tr><th>{dt(locale, 'headword')}</th><th>{dt(locale, 'meaningsPreview')}</th><th>{dt(locale, 'reading')}</th></tr></thead><tbody>{preview.sample.map((entry) => <tr key={entry.headword}><td>{entry.headword}</td><td>{entry.meanings.join(' / ') || entry.payload}</td><td>{entry.reading}</td></tr>)}</tbody></table></div></details>
      </article>)}
      <div className="dictionary-actions"><button type="button" disabled={busy || previews.length === 0 || Object.keys(previewFailures).length > 0} onClick={() => void commitImport()}>{dt(locale, 'import')}</button>{previews.length === 0 && <span className="dictionary-muted">{dt(locale, 'previewRequired')}</span>}</div>
    </>}
    <hr /><h3>{dt(locale, 'reload')}</h3><p>{dt(locale, 'reloadInstructions')}</p>
    {reloadable.length === 0 ? <p>{dt(locale, 'noReloadable')}</p> : <ul className="dictionary-reload-list">{reloadable.map((dictionary) => <li key={dictionary.id}><div><strong>{dictionary.name}</strong><small className="dictionary-path">{dictionary.sourcePath}</small></div><button type="button" disabled={busy} onClick={() => void reload(dictionary)}>{dt(locale, 'reload')}</button></li>)}</ul>}
  </section>;
}
