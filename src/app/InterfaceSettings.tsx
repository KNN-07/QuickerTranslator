import { useEffect, useRef } from 'react';
import { translate } from '../lib/i18n';
import type { ThemePreference, UiLocale } from '../lib/types';
import { dt } from '../features/dictionaries/i18n';
import type { DictionaryTab } from '../features/dictionaries/DictionaryDialog';
import { WorkspaceSettings } from '../features/workspace/WorkspaceSettings';
import type { WorkspaceController } from '../features/workspace/useWorkspaceController';
import type { InterfacePreferences } from './preferences';
import { ShortcutSettings } from '../features/settings/ShortcutSettings';
import { ProfileSettings } from '../features/ai/ProfileSettings';
import type { AiProfilesController } from '../features/ai/useAiProfiles';

export interface InterfaceSettingsProps {
  open: boolean;
  locale: UiLocale;
  theme: ThemePreference;
  workspace: WorkspaceController;
  aiProfiles: AiProfilesController;
  aiBusy: boolean;
  focusAiProfiles: boolean;
  preferences: InterfacePreferences;
  onPreferencesChange: (preferences: InterfacePreferences) => void;
  onLocaleChange: (locale: UiLocale) => void;
  onThemeChange: (theme: ThemePreference) => void;
  onClose: () => void;
  onDictionaries: (tab: DictionaryTab) => void;
}

export function InterfaceSettings({ open, locale, theme, workspace, aiProfiles, aiBusy, focusAiProfiles, preferences, onPreferencesChange, onLocaleChange, onThemeChange, onClose, onDictionaries }: InterfaceSettingsProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const profileSection = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  }, [open]);
  useEffect(() => {
    if (!open || !focusAiProfiles) return;
    const frame = window.requestAnimationFrame(() => {
      profileSection.current?.scrollIntoView({ block: 'start' });
      profileSection.current?.querySelector<HTMLSelectElement>('select')?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [open, focusAiProfiles]);

  return (
    <dialog className="settings-dialog" ref={dialogRef} aria-labelledby="interface-settings-title" onClose={onClose}>
      <h2 id="interface-settings-title">{translate(locale, 'settingsTitle')}</h2>
      <label>
        <span>{translate(locale, 'interfaceLanguage')}</span>
        <select value={locale} onChange={(event) => onLocaleChange(event.target.value === 'en' ? 'en' : 'vi')}>
          <option value="vi">{translate(locale, 'vietnamese')}</option>
          <option value="en">{translate(locale, 'english')}</option>
        </select>
      </label>
      <label>
        <span>{translate(locale, 'appearance')}</span>
        <select value={theme} onChange={(event) => onThemeChange(event.target.value === 'dark' ? 'dark' : 'light')}>
          <option value="light">{translate(locale, 'light')}</option>
          <option value="dark">{translate(locale, 'dark')}</option>
        </select>
      </label>
      <p>{translate(locale, 'nativeSettingsNotice')}</p>
      <WorkspaceSettings controller={workspace} locale={locale} />
      <ShortcutSettings preferences={preferences} locale={locale} onChange={onPreferencesChange} />
      <div ref={profileSection}><ProfileSettings profiles={aiProfiles} locale={locale} active={open} aiBusy={aiBusy} /></div>
      <section className="dictionary-settings-section" aria-labelledby="dictionary-settings-title">
        <h3 id="dictionary-settings-title">{dt(locale, 'title')}</h3>
        <div className="dictionary-actions">
          <button type="button" onClick={() => { dialogRef.current?.close(); onDictionaries('entries'); }}>{dt(locale, 'entries')}</button>
          <button type="button" onClick={() => { dialogRef.current?.close(); onDictionaries('imports'); }}>{dt(locale, 'imports')}</button>
          <button type="button" onClick={() => { dialogRef.current?.close(); onDictionaries('shortcuts'); }}>{dt(locale, 'shortcuts')}</button>
          <button type="button" onClick={() => { dialogRef.current?.close(); onDictionaries('attribution'); }}>{dt(locale, 'attribution')}</button>
        </div>
      </section>
      <div className="dialog-actions">
        <button type="button" onClick={() => dialogRef.current?.close()} autoFocus>{translate(locale, 'close')}</button>
      </div>
    </dialog>
  );
}
