import type { InterfacePreferences, TraversalAction } from '../../app/preferences';
import { TRAVERSAL_ACTIONS } from '../../app/preferences';
import type { UiLocale } from '../../lib/types';
import { ft } from '../documents/i18n';

const RESERVED = ['o', 's', 'e', 'f', 'z', 'y', 'c', 'v', 'x', 'a', '0', '1', '2', '3', '4', '5', '6'];
export function ShortcutSettings({ preferences, locale, onChange }: {
  preferences: InterfacePreferences; locale: UiLocale; onChange: (preferences: InterfacePreferences) => void;
}) {
  function updateKey(action: TraversalAction, key: string, input: HTMLInputElement) {
    const normalized = key.toLowerCase();
    const conflict = normalized.length !== 1 || !/^[a-z]$/u.test(normalized) || RESERVED.includes(normalized)
      || TRAVERSAL_ACTIONS.some((other) => other !== action && preferences.keybindings[other].toLowerCase() === normalized);
    input.setCustomValidity(conflict ? ft(locale, 'shortcutConflict') : '');
    if (conflict) { input.reportValidity(); return; }
    onChange({ ...preferences, keybindings: { ...preferences.keybindings, [action]: normalized } });
  }
  return <section className="shortcut-settings" aria-labelledby="shortcut-settings-title">
    <h3 id="shortcut-settings-title">{ft(locale, 'shortcuts')}</h3><p>{ft(locale, 'shortcutHelp')}</p>
    <div className="traversal-settings">{TRAVERSAL_ACTIONS.map((action) => <label key={action}><span>{ft(locale, action)}</span><span>Ctrl + <input aria-label={ft(locale, action)} maxLength={1} value={preferences.keybindings[action]} onFocus={(event) => event.target.select()} onChange={(event) => updateKey(action, event.target.value, event.target)} /></span></label>)}</div>
    <h4>{ft(locale, 'snippets')}</h4>{preferences.snippets.map((snippet, index) => <label className="snippet-setting" key={index}><span>F{index + 1}</span><textarea rows={2} value={snippet} aria-label={`F${index + 1}`} onChange={(event) => {
      const snippets = [...preferences.snippets]; snippets[index] = event.target.value; onChange({ ...preferences, snippets });
    }} /></label>)}<p>{ft(locale, 'typedHelp')}</p>
  </section>;
}
