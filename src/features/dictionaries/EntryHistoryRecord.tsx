import type { EntryRecord } from '../../lib/dictionaryTypes';
import type { UiLocale } from '../../lib/types';
import { dt } from './i18n';

/** Render saved dictionary text, not JSON field names or executable markup. */
export function EntryHistoryRecord({ locale, entry }: { locale: UiLocale; entry: EntryRecord | null }) {
  if (!entry) return <p>—</p>;
  return <dl className="dictionary-history-record">
    <dt>{dt(locale, 'headword')}</dt><dd>{entry.headword}</dd>
    <dt>{dt(locale, 'meaningsPreview')}</dt><dd>{entry.meanings.map((meaning, index) => <div key={index}>{meaning}</div>)}</dd>
    {entry.reading && <><dt>{dt(locale, 'reading')}</dt><dd>{entry.reading}</dd></>}
    {entry.pos && <><dt>{dt(locale, 'pos')}</dt><dd>{entry.pos}</dd></>}
    {entry.aliases.length > 0 && <><dt>{dt(locale, 'aliases')}</dt><dd>{entry.aliases.map((alias) => <div key={alias}>{alias}</div>)}</dd></>}
    {entry.sourceUrls.length > 0 && <><dt>{dt(locale, 'sourceUrls')}</dt><dd>{entry.sourceUrls.map((url) => <div key={url}>{url}</div>)}</dd></>}
    {entry.payload && <><dt>{dt(locale, 'payload')}</dt><dd><pre>{entry.payload}</pre></dd></>}
  </dl>;
}
