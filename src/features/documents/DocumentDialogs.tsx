import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { UiLocale } from '../../lib/types';
import { paneTitle } from '../../lib/i18n';
import type { DocumentController } from './useDocuments';
import type { ExportColumn, ExportFormat } from './types';
import { ft, localizedDocumentWarning } from './i18n';

function ActionDialog({ open, title, children, onCancel, className = '' }: { open: boolean; title: string; children: ReactNode; onCancel: () => void; className?: string }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (open && !ref.current?.open) ref.current?.showModal();
    else if (!open && ref.current?.open) ref.current.close();
  }, [open]);
  return <dialog ref={ref} className={`document-dialog ${className}`} aria-label={title} onCancel={(event) => { event.preventDefault(); onCancel(); }}>
    <h2>{title}</h2>{children}
  </dialog>;
}
const ALL_COLUMNS: ExportColumn[] = ['source', 'readings', 'phrases', 'singleMeaning', 'target'];
const ENCODINGS = ['utf-8', 'utf-16le', 'utf-16be', 'gb18030', 'gbk', 'big5', 'shift_jis', 'euc-jp', 'windows-1258'];
export function DocumentDialogs({ controller: d, locale, sourceLanguage }: { controller: DocumentController; locale: UiLocale; sourceLanguage: 'zh' | 'ja' }) {
  const [format, setFormat] = useState<ExportFormat>('txt');
  const [columns, setColumns] = useState<ExportColumn[]>(['target']);
  const [blankLines, setBlankLines] = useState(0);
  return <>
    <ActionDialog open={d.importPreview !== null} title={ft(locale, 'importTitle')} onCancel={() => { if (!d.busy) d.cancelImport(); }}>
      <p className="document-path">{d.importPreview?.path}</p>
      <label className="document-form-row"><span>{ft(locale, 'encoding')}</span><select value={d.importEncoding} onChange={(event) => d.setImportEncoding(event.target.value)} disabled={d.busy}>
        <option value="">{ft(locale, 'auto')}</option>{ENCODINGS.map((encoding) => <option key={encoding} value={encoding}>{encoding}</option>)}
      </select><button type="button" disabled={d.busy} onClick={() => void d.previewEncoding()}>{ft(locale, 'preview')}</button></label>
      <p>{d.importPreview?.encoding}{d.importPreview?.detectedEncoding ? ` — ${ft(locale, 'detected')}` : ''}</p>
      <pre className="document-preview">{d.importPreview?.previewText}</pre>
      {!!d.importPreview?.warnings.length && <ul>{d.importPreview.warnings.map((warning, index) => <li key={index}>{localizedDocumentWarning(locale, warning)}</li>)}</ul>}
      <div className="document-actions"><button type="button" disabled={d.busy} onClick={() => void d.confirmImport()}>{ft(locale, 'import')}</button><button type="button" disabled={d.busy} onClick={d.cancelImport}>{ft(locale, 'cancel')}</button></div>
    </ActionDialog>
    <ActionDialog open={d.exportOpen} title={ft(locale, 'exportTitle')} onCancel={() => { if (!d.busy) d.setExportOpen(false); }}>
      <label className="document-form-row"><span>{ft(locale, 'format')}</span><select value={format} disabled={d.busy} onChange={(event) => setFormat(event.target.value as ExportFormat)}>
        <option value="txt">UTF-8 TXT</option><option value="html">HTML</option><option value="docx">Word DOCX</option>
      </select></label>
      {format !== 'txt' && <fieldset disabled={d.busy}><legend>{ft(locale, 'columns')}</legend>
        {ALL_COLUMNS.map((column) => <label key={column} className="document-column-option"><input type="checkbox" checked={columns.includes(column)} onChange={(event) => setColumns((current) => event.target.checked ? [...current, column] : current.filter((item) => item !== column))} />{paneTitle(locale, sourceLanguage, column)}</label>)}
        <ol className="document-column-order">{columns.map((column, index) => <li key={column}><span>{paneTitle(locale, sourceLanguage, column)}</span>{([-1, 1] as const).map((direction) => <button key={direction} type="button" disabled={index + direction < 0 || index + direction >= columns.length} aria-label={`${ft(locale, direction < 0 ? 'up' : 'down')}: ${paneTitle(locale, sourceLanguage, column)}`} onClick={() => setColumns((current) => {
          const reordered = [...current]; const moved = reordered.splice(index, 1)[0]!; reordered.splice(index + direction, 0, moved); return reordered;
        })}>{direction < 0 ? '↑' : '↓'}</button>)}</li>)}</ol>
        <label className="document-form-row"><span>{ft(locale, 'blankLines')}</span><input type="number" min="0" max="255" value={blankLines} onChange={(event) => { const value = event.target.valueAsNumber; if (Number.isInteger(value) && value >= 0 && value <= 255) setBlankLines(value); }} /></label>
      </fieldset>}
      <div className="document-actions"><button type="button" disabled={d.busy || (format !== 'txt' && columns.length === 0)} onClick={() => void d.exportDocument({ format, columns: format === 'txt' ? ['target'] : columns, blankLines })}>{ft(locale, 'export')}</button><button type="button" disabled={d.busy} onClick={() => d.setExportOpen(false)}>{ft(locale, 'cancel')}</button></div>
    </ActionDialog>
    <ActionDialog open={d.recoveryOpen && d.recoveries.length > 0} title={ft(locale, 'recoveryTitle')} onCancel={() => { if (!d.busy) d.setRecoveryOpen(false); }}>
      <p>{ft(locale, 'recoveryNotice')}</p>
      <ul className="recovery-records">{d.recoveries.map((record) => <li key={record.documentId}><strong>{record.name}</strong><span className="document-path">{record.path}</span><time>{new Date(record.updatedAt * 1000).toLocaleString(locale)}</time><div className="document-actions"><button type="button" disabled={d.busy} onClick={() => void d.recover(record)}>{ft(locale, 'recover')}</button><button type="button" disabled={d.busy} onClick={() => void d.discardRecovery(record)}>{ft(locale, 'discard')}</button></div></li>)}</ul>
      <div className="document-actions"><button type="button" disabled={d.busy} onClick={() => d.setRecoveryOpen(false)}>{ft(locale, 'later')}</button></div>
    </ActionDialog>
    <ActionDialog open={d.unsavedOpen} title={ft(locale, 'unsavedTitle')} onCancel={() => d.answerUnsaved('cancel')} className="unsaved-dialog">
      <p>{ft(locale, 'unsaved')}</p><div className="document-actions"><button type="button" onClick={() => d.answerUnsaved('save')}>{ft(locale, 'save')}</button><button type="button" onClick={() => d.answerUnsaved('discard')}>{ft(locale, 'discard')}</button><button type="button" autoFocus onClick={() => d.answerUnsaved('cancel')}>{ft(locale, 'cancel')}</button></div>
    </ActionDialog>
  </>;
}
