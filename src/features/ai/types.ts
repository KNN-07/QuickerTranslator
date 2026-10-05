import type { AiTranslationRequest, TextRange } from '../../lib/types';

/** Native preview is prepared by the same prompt builder as the actual request. */
export interface AiPayloadPreview {
  profileId: string;
  requestUrl: string;
  endpointIdentity: string;
  host: string;
  model: string;
  mode: AiTranslationRequest['mode'];
  scope: AiTranslationRequest['scope'];
  sourceLanguage: AiTranslationRequest['sourceLanguage'];
  sourceRange: TextRange;
  sourceText: string;
  targetText: string | null;
  chunkCount: number;
  chunks: { chunkIndex: number; sourceRange: TextRange; sourceText: string; system: string; user: string }[];
}
