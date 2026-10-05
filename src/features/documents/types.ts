import type { WorkspaceDocument } from '../workspace/state';

export interface ProjectDocument extends WorkspaceDocument {
  format: 'quicktranslator-project';
  version: 1;
  targetLanguage: 'vi';
  legacyRtf?: string;
}
export interface DocumentImportPreview {
  path: string;
  kind: 'project' | 'legacyQt' | 'text' | 'html';
  encoding: string;
  detectedEncoding: boolean;
  previewText: string;
  warnings: string[];
}
export type OpenDocumentResult = {
  status: 'ready'; path: string; project: ProjectDocument; imported: boolean; warnings: string[];
} | { status: 'encodingConfirmationRequired'; preview: DocumentImportPreview };
export interface SaveResult { path: string; directorySyncConfirmed: boolean }
export interface RecoveryRecord { documentId: string; name: string; path: string | null; updatedAt: number }
export interface RecoveryDocument extends RecoveryRecord { project: ProjectDocument }
export type ExportFormat = 'txt' | 'html' | 'docx' | 'rtf';
export type ExportColumn = 'source' | 'readings' | 'phrases' | 'singleMeaning' | 'target';
export interface ExportOptions { format: ExportFormat; columns: ExportColumn[]; blankLines: number }
export type UnsavedDecision = 'save' | 'discard' | 'cancel';
export function projectDocument(document: WorkspaceDocument, legacyRtf?: string): ProjectDocument {
  return { ...document, format: 'quicktranslator-project', version: 1, targetLanguage: 'vi', ...(legacyRtf !== undefined ? { legacyRtf } : {}) };
}
export function fileName(path: string): string { return path.split(/[\\/]/u).pop() || path; }
export function projectName(path: string): string { return fileName(path).replace(/\.[^.]+$/u, '') + '.qtp'; }
export function withExtension(path: string, extension: string): string {
  return path.toLowerCase().endsWith(`.${extension}`) ? path : `${path}.${extension}`;
}
