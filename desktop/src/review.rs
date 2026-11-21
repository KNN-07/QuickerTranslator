use super::*;
use quickertranslator_lib::{documents::DraftEdits, engine::alignment::OffsetMap};
use std::collections::HashMap;

fn movable(s: &TranslationSegment) -> bool {
    !s.single_meaning_range.is_empty()
        && !s.surface.contains('\n')
        && s.surface.chars().any(char::is_alphanumeric)
}
fn ordered<'a>(r: &'a TranslationResult, d: &DraftEdits) -> Vec<&'a TranslationSegment> {
    let rank: HashMap<_, _> = d.order.iter().enumerate().map(|(i, id)| (id, i)).collect();
    let mut tokens = r.segments.iter().filter(|s| movable(s)).collect::<Vec<_>>();
    tokens.sort_by_key(|s| rank.get(&s.id).copied().unwrap_or(usize::MAX));
    tokens
}
pub fn projected(r: &TranslationResult, d: &DraftEdits, wrapped: bool) -> Result<String> {
    let offsets = OffsetMap::new(&r.single_meaning)?;
    let mut tokens = ordered(r, d).into_iter();
    let mut out = String::new();
    let mut pos = 0;
    for segment in &r.segments {
        let range = offsets.utf16_range_to_byte(segment.single_meaning_range)?;
        out.push_str(&r.single_meaning[pos..range.start]);
        let token = if movable(segment) {
            tokens.next().context("Missing draft token")?
        } else {
            segment
        };
        if let Some(text) = d.overrides.get(&token.id) {
            if wrapped {
                out.push('[')
            }
            out.push_str(text);
            if wrapped {
                out.push(']')
            }
        } else {
            out.push_str(
                &r.single_meaning[offsets.utf16_range_to_byte(token.single_meaning_range)?],
            );
        }
        pos = range.end;
    }
    out.push_str(&r.single_meaning[pos..]);
    Ok(out)
}
impl Workspace {
    pub fn select_segment(&mut self, index: usize, w: &mut Window, c: &mut Context<Self>) {
        let Some(s) = self.result.as_ref().and_then(|r| r.segments.get(index)) else {
            return;
        };
        self.segment = index;
        self.layout.selected = true;
        let text = value(&self.source, c);
        if let Ok(pos) =
            OffsetMap::new(&text).and_then(|map| map.utf16_to_byte(s.source_range.start))
        {
            let prefix = &text[..pos];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
            let col = prefix.rsplit('\n').next().unwrap_or("").chars().count() as u32;
            self.source.update(c, |f, c| {
                f.set_cursor_position(gpui_component::input::Position::new(line, col), w, c)
            });
        }
        c.notify();
    }
    pub fn aligned(&self, pane: shell::Pane, c: &Context<Self>) -> AnyElement {
        let Some(r) = &self.result else {
            return div().into_any_element();
        };
        use shell::Pane::*;
        let text = match pane {
            Readings => r.readings.clone(),
            Phrases => r.phrases.clone(),
            Source => value(&self.source, c),
            _ => r.single_meaning.clone(),
        };
        let Ok(map) = OffsetMap::new(&text) else {
            return div().into_any_element();
        };
        let ranges = r
            .segments
            .iter()
            .map(|s| {
                map.utf16_range_to_byte(match pane {
                    Readings => s.readings_range,
                    Phrases => s.phrases_range,
                    Source => s.source_range,
                    _ => s.single_meaning_range,
                })
                .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        let dark = matches!(
            self.settings.theme,
            quickertranslator_lib::settings::Theme::Dark
        );
        let red = rgb(if dark { 0xff9191 } else { 0xa82626 }).into();
        let highlights = ranges
            .iter()
            .zip(&r.segments)
            .enumerate()
            .filter(|(_, (range, s))| {
                !range.is_empty() && (!s.meanings.is_empty() || !s.provenance.is_empty())
            })
            .map(|(i, (range, _))| {
                (
                    range.clone(),
                    HighlightStyle {
                        color: Some(red),
                        background_color: Some(if self.layout.selected && i == self.segment {
                            rgb(if dark { 0x264b75 } else { 0xd7e9ff }).into()
                        } else {
                            rgb(if dark { 0x30252a } else { 0xfff7f7 }).into()
                        }),
                        underline: Some(UnderlineStyle {
                            color: Some(red),
                            thickness: px(1.),
                            wavy: false,
                        }),
                        ..Default::default()
                    },
                )
            });
        let view = c.entity().downgrade();
        InteractiveText::new(
            ("aligned", pane as usize),
            StyledText::new(text.clone()).with_highlights(highlights),
        )
        .on_click(ranges, move |index, w, c| {
            let _ = view.update(c, |s, c| s.select_segment(index, w, c));
        })
        .into_any_element()
    }
    pub fn edit_token(
        &mut self,
        meaning: Option<String>,
        direction: isize,
        w: &mut Window,
        c: &mut Context<Self>,
    ) -> Result<()> {
        let r = self.result.as_ref().context("Translate the source first")?;
        let seg = r.segments.get(self.segment).context("Choose a token")?;
        let source = value(&self.source, c);
        let d = &mut self.project.draft_edits;
        if d.source_text != source {
            *d = Project::new(self.project.source_language, source.clone()).draft_edits;
        }
        let wrapped = self.settings.translation_options.single_wrap == SingleWrap::All;
        if projected(r, d, wrapped)? != value(&self.draft, c)
            && !confirm(
                "Replace the manually edited draft?",
                "The token edit will regenerate the single-meaning draft.",
            )
        {
            return Ok(());
        }
        if let Some(meaning) = meaning {
            d.overrides.insert(seg.id.clone(), meaning);
        } else {
            let mut tokens = ordered(r, d);
            let i = tokens
                .iter()
                .position(|s| s.id == seg.id)
                .context("Choose a word")?;
            let j = i.saturating_add_signed(direction);
            anyhow::ensure!(j < tokens.len(), "End of draft");
            let map = OffsetMap::new(&source)?;
            let a = map.utf16_to_byte(tokens[i].source_range.start)?;
            let b = map.utf16_to_byte(tokens[j].source_range.start)?;
            anyhow::ensure!(
                !source[a.min(b)..a.max(b)].contains('\n'),
                "Tokens can move within one paragraph"
            );
            tokens.swap(i, j);
            d.order = tokens.iter().map(|s| s.id.clone()).collect();
        }
        d.source_revision = self.revision;
        let text = projected(r, d, wrapped)?;
        d.preview_text = Some(text.clone());
        self.synced_draft = Some(text.clone());
        set(&self.draft, text, w, c);
        self.draft_custom = true;
        self.dirty = true;
        self.recover(c)
    }
    pub fn meanings(&self, c: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_2();
        let Some(r) = &self.result else { return body };
        let Some(seg) = r.segments.get(self.segment) else {
            return body;
        };
        body = body
            .child(
                row()
                    .child(button("‹", c, |s, w, c| {
                        s.select_segment(s.segment.saturating_sub(1), w, c);
                        Ok(())
                    }))
                    .child(seg.surface.clone())
                    .child(button("›", c, |s, w, c| {
                        s.select_segment(s.segment + 1, w, c);
                        Ok(())
                    })),
            )
            .child(
                [
                    seg.lemma.as_deref(),
                    seg.reading.as_deref(),
                    seg.part_of_speech.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
            )
            .child(
                seg.provenance
                    .iter()
                    .map(|p| p.dictionary_name.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        for (i, m) in seg.meanings.iter().enumerate() {
            let meaning = m.clone();
            let insert = m.clone();
            body = body.child(
                div().id(("choice", i)).child(
                    row()
                        .child(button(meaning.clone(), c, move |s, w, c| {
                            s.edit_token(Some(meaning.clone()), 0, w, c)
                        }))
                        .child(button("Insert", c, move |s, w, c| {
                            s.insert_text(insert.clone(), w, c)
                        })),
                ),
            )
        }
        body.child(
            row()
                .child(button("Move left", c, |s, w, c| {
                    s.edit_token(None, -1, w, c)
                }))
                .child(button("Move right", c, |s, w, c| {
                    s.edit_token(None, 1, w, c)
                }))
                .child(button("Look up", c, |s, w, c| {
                    let headword = s.result.as_ref().context("Translate first")?.segments
                        [s.segment]
                        .surface
                        .clone();
                    set(&s.dict.query, headword, w, c);
                    s.page = Page::Dictionaries;
                    s.search(w, c)
                })),
        )
    }
}
