import type { UiLocale } from '../../lib/types';
import { paneTitle } from '../../lib/i18n';
import type { WorkspaceController } from './useWorkspaceController';
import { wt } from './i18n';

export function WorkspaceSettings({ controller, locale }: { controller: WorkspaceController; locale: UiLocale }) {
  const { options, viewState, sourceLanguage } = controller.state;
  return <section className="workspace-settings" aria-labelledby="workspace-settings-title">
    <h3 id="workspace-settings-title">{wt(locale, 'editorSettings')}</h3>
    <label><span>{wt(locale, 'algorithm')}</span><select value={options.algorithm} onChange={(event) => controller.setOptions({ ...options, algorithm: event.target.value as typeof options.algorithm })}>{(['longest', 'leftToRight', 'longestConditional'] as const).map((algorithm) => <option key={algorithm} value={algorithm}>{wt(locale, algorithm)}</option>)}</select></label>
    <label className="workspace-check"><input type="checkbox" checked={options.prioritizeNames} onChange={(event) => controller.setOptions({ ...options, prioritizeNames: event.target.checked })} /><span>{wt(locale, 'prioritizeNames')}</span></label>
    <label><span>{wt(locale, 'fullWrap')}</span><select value={options.fullWrap} onChange={(event) => controller.setOptions({ ...options, fullWrap: event.target.value as typeof options.fullWrap })}>{(['none', 'all', 'ambiguous'] as const).map((mode) => <option key={mode} value={mode}>{wt(locale, mode)}</option>)}</select></label>
    <label><span>{wt(locale, 'singleWrap')}</span><select value={options.singleWrap} onChange={(event) => controller.setOptions({ ...options, singleWrap: event.target.value as typeof options.singleWrap })}>{(['none', 'all'] as const).map((mode) => <option key={mode} value={mode}>{wt(locale, mode)}</option>)}</select></label>
    <label className="workspace-check"><input type="checkbox" checked={viewState.autoScroll} onChange={(event) => controller.setViewState({ ...viewState, autoScroll: event.target.checked })} /><span>{wt(locale, 'autoScroll')}</span></label>
    <label className="workspace-check"><input type="checkbox" checked={viewState.wrap} onChange={(event) => controller.setViewState({ ...viewState, wrap: event.target.checked, wraps: { source: event.target.checked, readings: event.target.checked, phrases: event.target.checked, singleMeaning: event.target.checked, meanings: event.target.checked, target: event.target.checked } })} /><span>{wt(locale, 'wrap')}</span></label>
    <div className="pane-font-settings">{(Object.keys(viewState.fonts) as (keyof typeof viewState.fonts)[]).map((pane) => <div key={pane}>
      <label><span>{paneTitle(locale, sourceLanguage, pane)}</span><input type="number" min="8" max="96" aria-label={`${paneTitle(locale, sourceLanguage, pane)} · ${wt(locale, 'fontSize')}`} value={viewState.fonts[pane]} onChange={(event) => {
        const size = event.target.valueAsNumber;
        if (Number.isInteger(size) && size >= 8 && size <= 96) controller.setViewState({ ...viewState, fonts: { ...viewState.fonts, [pane]: size } });
      }} /></label>
      <label className="workspace-check"><input type="checkbox" checked={viewState.wraps?.[pane] ?? viewState.wrap} onChange={(event) => {
        const wraps = viewState.wraps ?? { source: viewState.wrap, readings: viewState.wrap, phrases: viewState.wrap, singleMeaning: viewState.wrap, meanings: viewState.wrap, target: viewState.wrap };
        controller.setViewState({ ...viewState, wraps: { ...wraps, [pane]: event.target.checked } });
      }} /><span>{wt(locale, 'wrap')} · {paneTitle(locale, sourceLanguage, pane)}</span></label>
    </div>)}</div>
  </section>;
}
