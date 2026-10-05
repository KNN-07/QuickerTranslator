import type { ImportIssue } from '../../lib/dictionaryTypes';
import type { UiLocale } from '../../lib/types';
import { diagnosticLabel, dt } from './i18n';

export function EncodingSelect({ locale, value, onChange, disabled = false }: {
  locale: UiLocale; value: string; onChange: (value: string) => void; disabled?: boolean;
}) {
  return <label>{dt(locale, 'encoding')}<select value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
    <option value="">{dt(locale, 'automatic')}</option>
    {['UTF-8', 'UTF-16LE', 'UTF-16BE', 'GB18030', 'GBK', 'Big5', 'Shift_JIS', 'EUC-JP', 'windows-1258'].map((encoding) => <option key={encoding} value={encoding}>{encoding}</option>)}
  </select></label>;
}

export function ImportIssues({ locale, issues }: { locale: UiLocale; issues: ImportIssue[] }) {
  return <details className="dictionary-diagnostics" open={issues.length > 0}>
    <summary>{dt(locale, 'diagnostics')} ({issues.length.toLocaleString(locale)})</summary>
    {issues.length === 0 ? <p>{dt(locale, 'noDiagnostics')}</p> : <div className="dictionary-table-scroll"><table>
      <thead><tr><th>{dt(locale, 'line')}</th><th>{dt(locale, 'code')}</th></tr></thead>
      <tbody>{issues.map((issue, index) => <tr key={`${issue.line}-${index}`}><td>{issue.line}</td><td>{diagnosticLabel(locale, issue.code)} <code>({issue.code})</code></td></tr>)}</tbody>
    </table></div>}
  </details>;
}

export function ImportReport({ locale, preview }: { locale: UiLocale; preview: {
  encoding: string; encodingDetected: boolean; decodedText: string; accepted: number; duplicates: number; malformed: number; issues: ImportIssue[];
} }) {
  return <>
    <p>{dt(locale, 'detectedEncoding')}: <strong>{preview.encoding}</strong> ({dt(locale, preview.encodingDetected ? 'autoDetected' : 'explicitEncoding')})</p>
    <dl className="dictionary-counts">
      <div><dt>{dt(locale, 'accepted')}</dt><dd>{preview.accepted.toLocaleString(locale)}</dd></div>
      <div><dt>{dt(locale, 'duplicates')}</dt><dd>{preview.duplicates.toLocaleString(locale)}</dd></div>
      <div><dt>{dt(locale, 'malformed')}</dt><dd>{preview.malformed.toLocaleString(locale)}</dd></div>
    </dl>
    {preview.accepted === 0 && <p className="dictionary-warning">{dt(locale, 'emptyPreview')}</p>}
    <ImportIssues locale={locale} issues={preview.issues} />
    <details><summary>{dt(locale, 'rawPreview')}</summary><pre className="dictionary-raw-preview">{preview.decodedText}</pre></details>
  </>;
}
