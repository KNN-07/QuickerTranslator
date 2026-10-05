import type { SerializedDockview } from 'dockview-react';
import type { ThemePreference, TranslationOptions, UiLocale } from '../lib/types';
import { DEFAULT_TRANSLATION_OPTIONS } from '../lib/types';
import type { WorkspaceViewState } from '../features/workspace/state';
import { DEFAULT_VIEW_STATE } from '../features/workspace/state';

export const TRAVERSAL_ACTIONS = ['wordNext', 'wordPrevious', 'lineNext', 'linePrevious', 'paragraphNext', 'paragraphPrevious'] as const;
export type TraversalAction = typeof TRAVERSAL_ACTIONS[number];
export interface InterfacePreferences {
  format: 'quicktranslator-settings';
  version: 1;
  locale: UiLocale;
  theme: ThemePreference;
  dockLayout: SerializedDockview | null;
  editorOptions: WorkspaceViewState;
  translationOptions: TranslationOptions;
  snippets: string[];
  keybindings: Record<TraversalAction, string>;
  /** Non-secret records are synchronized from native profile commands before settings saves. */
  profiles: unknown[];
}
export function defaultPreferences(): InterfacePreferences {
  return {
    format: 'quicktranslator-settings', version: 1, locale: 'vi', theme: 'light', dockLayout: null,
    editorOptions: structuredClone(DEFAULT_VIEW_STATE), translationOptions: { ...DEFAULT_TRANSLATION_OPTIONS }, snippets: Array<string>(9).fill(''),
    keybindings: { wordNext: 'k', wordPrevious: 'j', lineNext: 'm', linePrevious: 'i', paragraphNext: 'n', paragraphPrevious: 'u' }, profiles: [],
  };
}
