export type SourceLanguage = 'zh' | 'ja';
export type TargetLanguage = 'vi';
export type UiLocale = 'vi' | 'en';
export type ThemePreference = 'light' | 'dark';
export type TranslationAlgorithm = 'longest' | 'leftToRight' | 'longestConditional';
export type FullWrap = 'none' | 'all' | 'ambiguous';
export type SingleWrap = 'none' | 'all';

/** Half-open UTF-16 code-unit offsets. Endpoints must be scalar boundaries. */
export interface TextRange {
  start: number;
  end: number;
}

export interface AppError {
  code: string;
  message: string;
}

export interface TranslationOptions {
  algorithm: TranslationAlgorithm;
  prioritizeNames: boolean;
  fullWrap: FullWrap;
  singleWrap: SingleWrap;
}

export const DEFAULT_SOURCE_LANGUAGE: SourceLanguage = 'zh';
export const DEFAULT_TRANSLATION_OPTIONS: Readonly<TranslationOptions> = Object.freeze({
  algorithm: 'longest',
  prioritizeNames: true,
  fullWrap: 'none',
  singleWrap: 'none',
});

export type DictionaryKind =
  | 'primaryNames'
  | 'secondaryNames'
  | 'vietPhrase'
  | 'hanViet'
  | 'japanese'
  | 'pronouns'
  | 'rules'
  | 'ignored'
  | 'cedict'
  | 'babylon'
  | 'lacViet'
  | 'thieuChuu'
  | 'auxiliary';

export type DictionaryLayer = 'bundled' | 'imported' | 'edited';

export interface DictionaryProvenance {
  dictionaryId: string;
  dictionaryName: string;
  kind: DictionaryKind;
  layer: DictionaryLayer;
  sourceUrls: string[];
}

export interface TranslationSegment {
  id: string;
  sourceRange: TextRange;
  readingsRange: TextRange;
  phrasesRange: TextRange;
  singleMeaningRange: TextRange;
  surface: string;
  lemma: string | null;
  reading: string | null;
  partOfSpeech: string | null;
  meanings: string[];
  provenance: DictionaryProvenance[];
  unknown: boolean;
}

export interface TranslationRequest {
  documentId: string;
  sourceRevision: number;
  sourceLanguage: SourceLanguage;
  sourceText: string;
  options: TranslationOptions;
}

export interface TranslationResult {
  documentId: string;
  sourceRevision: number;
  dictionaryRevision: number;
  readings: string;
  phrases: string;
  singleMeaning: string;
  segments: TranslationSegment[];
}

export interface DocumentWindow {
  documentId: string;
  windowLabel: string;
}

export interface FoundationHealth {
  version: string;
  status: 'ready';
  documentWindow: DocumentWindow;
  sourceRevision: number;
  targetRevision: number;
  dictionaryRevision: number | null;
  dictionaryReady: boolean;
  tokenizerReady: boolean;
}

export type AiProtocol = 'openai-responses' | 'openai-chat' | 'gemini' | 'anthropic';
export type AiAuthMode = 'apiKey' | 'none';
export type TokenLimitField = 'max_tokens' | 'max_completion_tokens' | 'omit';

interface AiProfileFields {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  stream: boolean;
  maxOutputTokens: number;
  authMode: AiAuthMode;
  allowInsecureHttp: boolean;
}

/** Credentials are deliberately excluded from profile persistence and IPC results. */
export type AiProfile = AiProfileFields & (
  | { protocol: 'openai-chat'; tokenLimitField: TokenLimitField }
  | { protocol: Exclude<AiProtocol, 'openai-chat'>; tokenLimitField?: never }
);

export interface AiTranslationRequest {
  jobId: string;
  documentId: string;
  sourceRevision: number;
  targetRevision: number;
  profileId: string;
  mode: 'translate' | 'improve';
  sourceLanguage: SourceLanguage;
  scope: 'selection' | 'document';
  sourceRange: TextRange;
  sourceText: string;
  targetText: string | null;
  instructions: string;
}

export interface AiUsage {
  inputTokens: number | null;
  outputTokens: number | null;
}

export type AiEvent = { jobId: string } & (
  | { type: 'started'; chunkCount: number }
  | { type: 'chunkStarted'; chunkIndex: number; sourceRange: TextRange }
  | { type: 'delta'; chunkIndex: number; text: string }
  | { type: 'chunkCompleted'; chunkIndex: number; text: string; usage: AiUsage | null }
  | { type: 'completed'; text: string; usage: AiUsage | null }
  | { type: 'cancelled' }
  | { type: 'error'; code: string; message: string; chunkIndex: number | null }
);

export type PaneId = 'source' | 'readings' | 'phrases' | 'singleMeaning' | 'meanings' | 'target' | 'ai';
