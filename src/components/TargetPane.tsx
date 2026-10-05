import { useState } from 'react';
import { EditorContent } from '@tiptap/react';
import type { Editor } from '@tiptap/core';
import type { UiLocale } from '../lib/types';
import { wt } from '../features/workspace/i18n';

export interface TargetPaneProps { editor: Editor | null; fontSize: number; locale: UiLocale; wrap: boolean }
export function TargetPane({ editor, fontSize, locale, wrap }: TargetPaneProps) {
  const [, refresh] = useState(0);
  const run = (action: () => void) => { action(); refresh((value) => value + 1); };
  return <section className={`target-pane${wrap ? '' : ' target-pane--no-wrap'}`} style={{ fontSize }}>
    <div className="editor-format-toolbar" aria-label={wt(locale, 'formatting')}>
      <button type="button" title={wt(locale, 'bold')} aria-pressed={editor?.isActive('bold') ?? false} onMouseDown={(event) => event.preventDefault()} onClick={() => run(() => { editor?.chain().focus().toggleBold().run(); })}><b>B</b></button>
      <button type="button" title={wt(locale, 'italic')} aria-pressed={editor?.isActive('italic') ?? false} onMouseDown={(event) => event.preventDefault()} onClick={() => run(() => { editor?.chain().focus().toggleItalic().run(); })}><i>I</i></button>
      <button type="button" title={wt(locale, 'underline')} aria-pressed={editor?.isActive('underline') ?? false} onMouseDown={(event) => event.preventDefault()} onClick={() => run(() => { editor?.chain().focus().toggleUnderline().run(); })}><u>U</u></button>
      <label><span className="sr-only">{wt(locale, 'textColor')}</span><input type="color" aria-label={wt(locale, 'textColor')} defaultValue="#24282d" onChange={(event) => run(() => { editor?.chain().focus().setColor(event.target.value).run(); })} /></label>
      <select aria-label={wt(locale, 'fontSize')} defaultValue="" onChange={(event) => run(() => { if (event.target.value) editor?.chain().focus().setFontSize(`${event.target.value}px`).run(); else editor?.chain().focus().unsetFontSize().run(); })}>
        <option value="">{wt(locale, 'fontSize')}</option>{[12, 14, 16, 18, 20, 24, 28, 32].map((size) => <option key={size} value={size}>{size}</option>)}
      </select>
      <select aria-label={wt(locale, 'alignment')} defaultValue="left" onChange={(event) => run(() => { editor?.chain().focus().setTextAlign(event.target.value).run(); })}>
        <option value="left">{wt(locale, 'alignLeft')}</option><option value="center">{wt(locale, 'alignCenter')}</option><option value="right">{wt(locale, 'alignRight')}</option><option value="justify">{wt(locale, 'alignJustify')}</option>
      </select>
      <button type="button" title={wt(locale, 'undo')} onMouseDown={(event) => event.preventDefault()} onClick={() => run(() => { editor?.chain().focus().undo().run(); })}>↶</button>
      <button type="button" title={wt(locale, 'redo')} onMouseDown={(event) => event.preventDefault()} onClick={() => run(() => { editor?.chain().focus().redo().run(); })}>↷</button>
    </div>
    <EditorContent editor={editor} className="target-editor-host" />
  </section>;
}
