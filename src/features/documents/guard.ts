import type { WorkspaceSnapshot } from '../workspace/state';
import type { UnsavedDecision } from './types';

export interface DocumentStamp {
  documentId: string;
  sourceRevision: number;
  targetRevision: number;
  draftEdits: WorkspaceSnapshot['draftEdits'];
  draftDirty: boolean;
  options: WorkspaceSnapshot['options'];
}
export function documentStamp(state: WorkspaceSnapshot): DocumentStamp {
  return { documentId: state.documentId, sourceRevision: state.sourceRevision, targetRevision: state.targetRevision,
    draftEdits: state.draftEdits, draftDirty: state.draftDirty, options: state.options };
}
export function unchangedDocument(state: WorkspaceSnapshot, stamp: DocumentStamp): boolean {
  return state.documentId === stamp.documentId && state.sourceRevision === stamp.sourceRevision
    && state.targetRevision === stamp.targetRevision && state.options === stamp.options
    && state.draftDirty === stamp.draftDirty && (!stamp.draftDirty || state.draftEdits === stamp.draftEdits);
}
/** A cancelled or failed Save As never authorizes an action that discards the document. */
export async function guardUnsaved(
  getState: () => WorkspaceSnapshot,
  decide: () => Promise<UnsavedDecision>,
  save: () => Promise<boolean>,
): Promise<UnsavedDecision> {
  if (!getState().dirty) return 'save';
  const stamp = documentStamp(getState());
  const decision = await decide();
  if (decision === 'cancel' || !unchangedDocument(getState(), stamp)) return 'cancel';
  if (decision === 'save') return await save() && !getState().dirty ? 'save' : 'cancel';
  return 'discard';
}
