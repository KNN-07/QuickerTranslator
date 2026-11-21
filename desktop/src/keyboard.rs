use super::*;
use gpui_component::input::Position;

impl Workspace {
    pub fn find(&mut self, w: &mut Window, c: &mut Context<Self>) -> Result<()> {
        self.page = Page::Workspace;
        self.layout.show(shell::Pane::Source);
        self.layout.source_edit = true;
        c.on_next_frame(w, |s, w, c| {
            s.source.update(c, |f, c| f.focus(w, c));
            w.dispatch_action(Box::new(gpui_component::input::Search), c);
        });
        Ok(())
    }
    fn expand_shortcut(
        &mut self,
        event: &KeyDownEvent,
        w: &mut Window,
        c: &mut Context<Self>,
    ) -> Result<bool> {
        let previous = self.typed.take();
        let k = &event.keystroke;
        if self.shortcuts.is_empty()
            || !self.target.focus_handle(c).is_focused(w)
            || k.modifiers.control
            || k.modifiers.platform
            || k.modifiers.alt
        {
            return Ok(false);
        }
        let ch = match k.key.as_str() {
            "space" => " ",
            "enter" => "\n",
            _ => k.key_char.as_deref().unwrap_or(""),
        };
        if ch.chars().count() != 1
            || self
                .target
                .update(c, |f, c| f.marked_text_range(w, c))
                .is_some()
        {
            return Ok(false);
        }
        let range = selection(&self.target, w, c)?;
        if !range.is_empty() {
            return Ok(false);
        }
        let text = value(&self.target, c);
        let pos = range.start;
        let previous = previous.filter(|(start, s)| text.get(*start..pos) == Some(s));
        if !ch.chars().next().unwrap().is_alphanumeric() {
            if let Some((start, typed)) = &previous {
                if let Some(entry) = self
                    .shortcuts
                    .iter()
                    .find(|e| e.key == typed.to_lowercase())
                {
                    let replacement = format!("{}{ch}", entry.value);
                    let utf = engine::alignment::OffsetMap::new(&text)?
                        .byte_range_to_utf16(*start..pos)?;
                    self.target.update(c, |f, c| {
                        f.replace_text_in_range(
                            Some(utf.start as usize..utf.end as usize),
                            &replacement,
                            w,
                            c,
                        )
                    });
                    return Ok(true);
                }
            }
        }
        if previous.is_some()
            || text[..pos]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_alphanumeric())
        {
            let (start, mut typed) = previous.unwrap_or((pos, String::new()));
            typed.push_str(ch);
            if self
                .shortcuts
                .iter()
                .any(|e| e.key.starts_with(&typed.to_lowercase()))
            {
                self.typed = Some((start, typed));
            }
        }
        Ok(false)
    }
    pub fn key(
        &mut self,
        event: &KeyDownEvent,
        w: &mut Window,
        c: &mut Context<Self>,
    ) -> Result<bool> {
        let key = &event.keystroke.key;
        let m = event.keystroke.modifiers;
        if key == "escape" && (self.layout.menu.is_some() || self.page != Page::Workspace) {
            self.layout.menu = None;
            self.page = Page::Workspace;
            return Ok(true);
        }
        if self.page != Page::Workspace {
            return Ok(false);
        }
        if self.expand_shortcut(event, w, c)? {
            return Ok(true);
        }
        if m.alt && m.shift && matches!(key.as_str(), "left" | "right") {
            self.edit_token(None, if key == "left" { -1 } else { 1 }, w, c)?;
            return Ok(true);
        }
        if (m.control || m.platform) && !m.alt {
            match (key.as_str(), m.shift) {
                ("s", true) => self.save_as(w, c)?,
                ("n", true) => self.new_file(w, c)?,
                ("e", false) => self.export(w, c)?,
                ("f", false) => self.find(w, c)?,
                (_, false) if self.page == Page::Workspace => {
                    if let Some(action) =
                        self.settings.keybindings.iter().find_map(|(action, k)| {
                            (key.eq_ignore_ascii_case(k)).then_some(action.clone())
                        })
                    {
                        let forward = action.ends_with("Next");
                        if action.starts_with("word") && self.result.is_some() {
                            self.select_segment(
                                if forward {
                                    self.segment + 1
                                } else {
                                    self.segment.saturating_sub(1)
                                },
                                w,
                                c,
                            );
                        } else {
                            let source = value(&self.source, c);
                            let pos = self.source.read(c).cursor();
                            let starts = std::iter::once(0)
                                .chain(source.match_indices('\n').map(|(i, _)| i + 1))
                                .collect::<Vec<_>>();
                            let locations = if action.starts_with("word") {
                                source
                                    .char_indices()
                                    .filter_map(|(i, ch)| ch.is_alphanumeric().then_some(i))
                                    .collect()
                            } else if action.starts_with("paragraph") {
                                starts
                                    .into_iter()
                                    .filter(|i| {
                                        source[*i..]
                                            .lines()
                                            .next()
                                            .is_some_and(|s| !s.trim().is_empty())
                                    })
                                    .collect()
                            } else {
                                starts
                            };
                            let next = if forward {
                                locations.into_iter().find(|i| *i > pos)
                            } else {
                                locations.into_iter().rev().find(|i| *i < pos)
                            };
                            if let Some(next) = next {
                                let prefix = &source[..next];
                                self.source.update(c, |f, c| {
                                    f.set_cursor_position(
                                        Position::new(
                                            prefix.bytes().filter(|b| *b == b'\n').count() as u32,
                                            0,
                                        ),
                                        w,
                                        c,
                                    )
                                });
                                if action.starts_with("word") {
                                    let mut p = self.source.read(c).cursor_position();
                                    p.character =
                                        prefix.rsplit('\n').next().unwrap_or("").chars().count()
                                            as u32;
                                    self.source.update(c, |f, c| f.set_cursor_position(p, w, c));
                                }
                            }
                        }
                    } else if let Some(number) = key.parse::<usize>().ok().filter(|n| *n <= 6) {
                        if let Some((r, s)) = self
                            .result
                            .as_ref()
                            .and_then(|r| r.segments.get(self.segment).map(|s| (r, s)))
                        {
                            let text = if number == 0 {
                                let map = engine::alignment::OffsetMap::new(&r.readings)?;
                                Some(
                                    r.readings[map.utf16_range_to_byte(s.readings_range)?]
                                        .to_owned(),
                                )
                            } else {
                                s.meanings.get(number - 1).cloned()
                            };
                            if let Some(text) = text {
                                self.insert_text(text, w, c)?;
                            }
                        }
                    } else {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        } else if m.alt && !m.shift && (key == "left" || key == "right") {
            let path = self
                .origin
                .as_ref()
                .or(self.path.as_ref())
                .context("Open a file to navigate its directory")?;
            let siblings = quickertranslator_lib::documents::sibling_documents(path)?;
            if let Some(path) = if key == "left" {
                siblings.previous
            } else {
                siblings.next
            } {
                files::open_path(self.services.clone(), self.project.source_language, path, c)?;
            }
        } else if !m.modified() && self.page == Page::Workspace {
            let Some(i) = key
                .strip_prefix('f')
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=9).contains(n))
            else {
                return Ok(false);
            };
            let snippet = self.settings.snippets[i - 1].clone();
            if !snippet.is_empty() {
                self.insert_text(snippet, w, c)?;
            }
        } else {
            return Ok(false);
        }
        Ok(true)
    }
}
