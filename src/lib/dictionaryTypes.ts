import type { AppError, DictionaryKind, DictionaryLayer, DictionaryProvenance, SourceLanguage } from './types';

/** Mirrors src-tauri/src/dictionaries/types.rs; dictionary data is text, never HTML. */
export interface EntryRecord {
  headword: string;
  meanings: string[];
  reading: string | null;
  pos: string | null;
  aliases: string[];
  sourceUrls: string[];
  payload: string | null;
}
export type ImportFormat = 'legacy' | 'cedict' | 'ignored';
export interface ImportRequest {
  path: string;
  name: string;
  language: SourceLanguage;
  kind: DictionaryKind;
  dictionaryId: string | null;
  encoding: string | null;
  format: ImportFormat | null;
}
export interface ImportIssue { line: number; code: string; message: string }
export interface ImportPreview {
  previewId: string;
  dictionaryId: string;
  name: string;
  path: string;
  language: SourceLanguage;
  kind: DictionaryKind;
  encoding: string;
  encodingDetected: boolean;
  decodedText: string;
  accepted: number;
  duplicates: number;
  malformed: number;
  issues: ImportIssue[];
  sample: EntryRecord[];
}
export interface CommitImportRequest { previewIds: string[]; ruleAlgorithm: number | null }
export interface DictionaryMetadata {
  id: string;
  name: string;
  language: SourceLanguage;
  kind: DictionaryKind;
  bundled: boolean;
  sourcePath: string | null;
  encoding: string | null;
  format: ImportFormat | null;
  entryCount: number;
  aliasCount: number;
  importedCount: number;
  editedCount: number;
  tombstoneCount: number;
}
export interface DictionaryCatalog {
  revision: number;
  ruleAlgorithm: number;
  dictionaries: DictionaryMetadata[];
  manifest: unknown;
  attribution: string;
  licenses: Record<string, string>;
}
export interface MetadataMutation { id: string | null; name: string; language: SourceLanguage; kind: DictionaryKind }
export interface EntryMutation { dictionaryId: string; entry: EntryRecord }
export interface EntryKey { dictionaryId: string; headword: string }
export interface SearchRequest {
  query: string;
  dictionaryId: string | null;
  language: SourceLanguage | null;
  kind: DictionaryKind | null;
  offset: number;
  limit: number;
}
export interface EntryView {
  dictionaryId: string;
  language: SourceLanguage;
  kind: DictionaryKind;
  entry: EntryRecord;
  layer: DictionaryLayer;
  provenance: DictionaryProvenance[];
}
export interface SearchResult { total: number; entries: EntryView[] }
export interface EntryHistory { id: number; action: string; timestamp: string; before: EntryRecord | null; after: EntryRecord | null }
export interface ExportRequest { dictionaryId: string; destination: string }
export interface ExportResult { entries: number; directorySyncConfirmed: boolean }
export interface ConfigRequest { path: string; encoding: string | null; remappings: Record<string, string> }
export interface ConfigDictionary { key: string; kind: DictionaryKind; originalPath: string; resolvedPath: string | null; problem: string | null }
export interface ConfigPreview {
  encoding: string;
  encodingDetected: boolean;
  decodedText: string;
  ruleAlgorithm: number;
  dictionaries: ConfigDictionary[];
  issues: ImportIssue[];
}
export interface ShortcutRecord { key: string; value: string }
export interface ShortcutImportRequest { path: string; encoding: string | null }
export interface ShortcutPreview {
  previewId: string;
  encoding: string;
  encodingDetected: boolean;
  decodedText: string;
  accepted: number;
  duplicates: number;
  malformed: number;
  issues: ImportIssue[];
  sample: ShortcutRecord[];
}
export interface DictionaryStorageStatus { ready: boolean; path: string; error: AppError | null }
export interface RepairDatabaseRequest { backupDestination: string }
