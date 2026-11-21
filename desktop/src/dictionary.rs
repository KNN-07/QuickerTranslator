use super::*;
use quickertranslator_lib::dictionaries as d;
use rfd::FileDialog;

pub struct Panel {
    pub query: Field,
    headword: Field,
    meanings: Field,
    rows: Vec<d::EntryView>,
    selected: Option<d::EntryView>,
    offset: usize,
    total: usize,
    details: String,
    shortcut: bool,
    manage: bool,
    attribution: bool,
    name: Field,
    active: Option<String>,
    kind: DictionaryKind,
}
impl Panel {
    pub fn new(w: &mut Window, c: &mut App) -> Self {
        Self {
            query: field("", false, w, c),
            headword: field("", false, w, c),
            meanings: field("", true, w, c),
            rows: vec![],
            selected: None,
            offset: 0,
            total: 0,
            details: String::new(),
            shortcut: false,
            manage: false,
            attribution: false,
            name: field("", false, w, c),
            active: None,
            kind: DictionaryKind::VietPhrase,
        }
    }
}
impl Workspace {
    pub fn search(&mut self, _: &mut Window, c: &mut Context<Self>) -> Result<()> {
        self.dict.offset = 0;
        self.dict.selected = None;
        self.page_results(c)
    }
    fn page_results(&mut self, c: &App) -> Result<()> {
        let found = self.services.dictionaries.search(d::SearchRequest {
            query: value(&self.dict.query, c),
            dictionary_id: self.dict.active.clone(),
            language: Some(self.project.source_language),
            kind: None,
            offset: self.dict.offset,
            limit: 40,
        })?;
        self.dict.rows = found.entries;
        self.dict.total = found.total;
        self.dict.details = format!("{} matching entries", found.total);
        Ok(())
    }
    fn edit_entry(&mut self, w: &mut Window, c: &mut Context<Self>) -> Result<()> {
        let key = value(&self.dict.headword, c);
        let meanings = value(&self.dict.meanings, c);
        let store = &self.services.dictionaries;
        if self.dict.shortcut {
            store.save_shortcut(
                d::ShortcutRecord {
                    key,
                    value: meanings,
                },
                &self.services.state,
            )?;
        } else {
            let mut entry = if let Some(v) = &self.dict.selected {
                v.entry.clone()
            } else {
                d::EntryRecord {
                    headword: key.clone(),
                    meanings: vec![],
                    reading: None,
                    pos: None,
                    aliases: vec![],
                    source_urls: vec![],
                    payload: None,
                }
            };
            let id = if let Some(v) = &self.dict.selected {
                v.dictionary_id.clone()
            } else if let Some(id) = &self.dict.active {
                id.clone()
            } else {
                let kind = match self.project.source_language {
                    SourceLanguage::Ja => DictionaryKind::Japanese,
                    SourceLanguage::Zh => DictionaryKind::VietPhrase,
                };
                store
                    .catalog()?
                    .dictionaries
                    .into_iter()
                    .find(|v| v.kind == kind && v.language == self.project.source_language)
                    .context("No matching dictionary")?
                    .id
            };
            entry.headword = key.clone();
            entry.meanings = meanings.lines().map(str::to_owned).collect();
            store.save_entry(
                d::EntryMutation {
                    dictionary_id: id,
                    entry,
                },
                &self.services.state,
            )?;
            set(&self.dict.query, key, w, c);
            self.search(w, c)?;
        }
        self.invalidate();
        self.status = "Entry saved".into();
        Ok(())
    }
    fn import_dictionary(&mut self, _: &mut Window, _: &mut Context<Self>) -> Result<()> {
        let Some(path) = FileDialog::new().pick_file() else {
            return Ok(());
        };
        let store = &self.services.dictionaries;
        let path = path.to_string_lossy().into_owned();
        if self.dict.shortcut {
            let p = store.preview_shortcuts(d::ShortcutImportRequest {
                path,
                encoding: None,
            })?;
            if confirm(
                &format!("Import {} shortcuts?", p.accepted),
                &format!(
                    "{} · {} duplicates · {} malformed\n{}",
                    p.encoding, p.duplicates, p.malformed, p.decoded_text
                ),
            ) {
                store.commit_shortcuts(&p.preview_id, &self.services.state)?;
            }
        } else {
            let kind = if self.project.source_language == SourceLanguage::Ja {
                DictionaryKind::Japanese
            } else {
                self.dict.kind
            };
            let name = std::path::Path::new(&path)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let p = store.preview_import(d::ImportRequest {
                path,
                name,
                language: self.project.source_language,
                kind,
                dictionary_id: None,
                encoding: None,
                format: None,
            })?;
            if confirm(
                &format!("Import {} entries as {:?}?", p.accepted, p.kind),
                &format!(
                    "{} · {} duplicates · {} malformed\n{}",
                    p.encoding, p.duplicates, p.malformed, p.decoded_text
                ),
            ) {
                store.commit_import(
                    d::CommitImportRequest {
                        preview_ids: vec![p.preview_id],
                        rule_algorithm: None,
                    },
                    &self.services.state,
                )?;
            }
        }
        self.invalidate();
        Ok(())
    }
    fn import_config(&mut self, _: &mut Window, _: &mut Context<Self>) -> Result<()> {
        let Some(path) = FileDialog::new().pick_file() else {
            return Ok(());
        };
        let path = path.to_string_lossy().into_owned();
        let config = d::import::preview_config(&d::ConfigRequest {
            path,
            encoding: None,
            remappings: Default::default(),
        })?;
        let mut previews = vec![];
        let mut lines = vec![];
        for entry in config.dictionaries {
            let path = if let Some(path) = entry.resolved_path {
                path
            } else {
                let Some(path) = FileDialog::new()
                    .set_title(format!("Locate {} ({})", entry.key, entry.original_path))
                    .pick_file()
                else {
                    return Ok(());
                };
                path.to_string_lossy().into_owned()
            };
            let p = self
                .services
                .dictionaries
                .preview_import(d::ImportRequest {
                    path,
                    name: entry.key,
                    kind: entry.kind,
                    language: SourceLanguage::Zh,
                    encoding: None,
                    dictionary_id: None,
                    format: None,
                })?;
            lines.push(format!(
                "{} · {} · {} entries · {} malformed · {} duplicates",
                p.name, p.encoding, p.accepted, p.malformed, p.duplicates
            ));
            previews.push(p.preview_id);
        }
        lines.extend(config.issues.into_iter().map(|i| i.message));
        if confirm("Import these dictionaries?", &lines.join("\n")) {
            self.services.dictionaries.commit_import(
                d::CommitImportRequest {
                    preview_ids: previews,
                    rule_algorithm: Some(config.rule_algorithm),
                },
                &self.services.state,
            )?;
        }
        Ok(())
    }
    fn manage_dictionaries(&self, c: &Context<Self>) -> Div {
        let mut form = div()
            .flex()
            .flex_col()
            .gap_2()
            .child("Dictionary name")
            .child(Input::new(&self.dict.name));
        if let Ok(cat) = self.services.dictionaries.catalog() {
            let mut list = row();
            for v in cat
                .dictionaries
                .into_iter()
                .filter(|v| v.language == self.project.source_language)
            {
                list = list.child(button(
                    format!("{} ({})", v.name, v.entry_count),
                    c,
                    move |s, w, c| {
                        s.dict.active = Some(v.id.clone());
                        s.dict.kind = v.kind;
                        set(&s.dict.name, v.name.clone(), w, c);
                        s.search(w, c)
                    },
                ));
            }
            form = form.child(list)
        }
        form.child(
            row()
                .child(button(
                    format!("Type · {:?}", self.dict.kind),
                    c,
                    |s, _, _| {
                        use DictionaryKind::*;
                        let kinds = [
                            VietPhrase,
                            PrimaryNames,
                            SecondaryNames,
                            HanViet,
                            Pronouns,
                            Rules,
                            Ignored,
                            Cedict,
                            Babylon,
                            LacViet,
                            ThieuChuu,
                            Auxiliary,
                            Japanese,
                        ];
                        s.dict.kind =
                            kinds[(kinds.iter().position(|k| *k == s.dict.kind).unwrap_or(0) + 1)
                                % kinds.len()];
                        Ok(())
                    },
                ))
                .child(button("New dictionary", c, |s, w, c| {
                    s.dict.active = None;
                    s.dict.selected = None;
                    set(&s.dict.name, "", w, c);
                    Ok(())
                }))
                .child(button("Save metadata", c, |s, w, c| {
                    let v = s.services.dictionaries.save_metadata(
                        d::MetadataMutation {
                            id: s.dict.active.clone(),
                            name: value(&s.dict.name, c),
                            language: s.project.source_language,
                            kind: s.dict.kind,
                        },
                        &s.services.state,
                    )?;
                    s.dict.active = Some(v.id);
                    s.search(w, c)
                }))
                .child(button("Import configuration…", c, Self::import_config))
                .child(button("Reload from file", c, |s, _, _| {
                    let id = s
                        .dict
                        .active
                        .as_deref()
                        .context("Choose an imported dictionary")?;
                    let p = s.services.dictionaries.reload_preview(id)?;
                    if confirm(&format!("Reload {} entries?", p.accepted), &p.decoded_text) {
                        s.services.dictionaries.commit_import(
                            d::CommitImportRequest {
                                preview_ids: vec![p.preview_id],
                                rule_algorithm: None,
                            },
                            &s.services.state,
                        )?;
                    }
                    Ok(())
                }))
                .child(button("Storage status", c, |s, _, _| {
                    s.dict.details =
                        serde_json::to_string_pretty(&s.services.dictionaries.storage_status()?)?;
                    Ok(())
                }))
                .child(button("Repair storage…", c, |s, _, _| {
                    anyhow::ensure!(
                        !s.services.dictionaries.storage_status()?.ready,
                        "The database is healthy"
                    );
                    if let Some(p) = FileDialog::new()
                        .set_title("Choose an unused backup destination")
                        .save_file()
                    {
                        s.services.dictionaries.repair_database(
                            d::RepairDatabaseRequest {
                                backup_destination: p.to_string_lossy().into_owned(),
                            },
                            &s.services.state,
                        )?;
                    }
                    Ok(())
                })),
        )
    }
    pub fn dictionary_page(&self, c: &Context<Self>) -> Div {
        let mut results = div().flex().flex_col().gap_1();
        for (i, v) in self.dict.rows.iter().enumerate() {
            results = results.child(
                div()
                    .id(("entry", i))
                    .p_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(c.theme().secondary))
                    .child(format!(
                        "{}   {}",
                        v.entry.headword,
                        v.entry.meanings.join(" / ")
                    ))
                    .on_click(c.listener(move |s, _, w, c| {
                        let entry = s.dict.rows[i].clone();
                        set(&s.dict.headword, entry.entry.headword.clone(), w, c);
                        set(&s.dict.meanings, entry.entry.meanings.join("\n"), w, c);
                        s.dict.details = format!(
                            "{:?} · {:?} · {}",
                            entry.kind, entry.layer, entry.dictionary_id
                        );
                        s.dict.selected = Some(entry);
                        c.notify();
                    })),
            )
        }
        let vi = self.settings.locale == UiLocale::Vi;
        let mut tabs = row();
        for (i, en, local) in [
            (0, "Search / edit", "Tra cứu / biên tập"),
            (1, "Import / reload", "Nhập / nạp lại"),
            (2, "Text shortcuts", "Gõ tắt"),
            (3, "Data sources / licenses", "Nguồn dữ liệu / giấy phép"),
        ] {
            tabs = tabs.child(button(if vi { local } else { en }, c, move |s, w, c| {
                s.dict.manage = i == 1;
                s.dict.shortcut = i == 2;
                s.dict.attribution = i == 3;
                s.dict.details.clear();
                s.dict.selected = None;
                if i == 0 {
                    s.search(w, c)?;
                } else if i == 3 {
                    let cat = s.services.dictionaries.catalog()?;
                    let mut notices = cat.licenses.into_iter().collect::<Vec<_>>();
                    notices.sort_by(|a, b| a.0.cmp(&b.0));
                    s.dict.details = format!(
                        "{}\n{}\n{}",
                        cat.attribution,
                        notices
                            .into_iter()
                            .map(|(n, s)| format!("{n}\n{s}"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        serde_json::to_string_pretty(&cat.manifest)?
                    );
                }
                Ok(())
            }));
        }
        if self.dict.attribution {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(tabs)
                .child(scroll("license-view", self.dict.details.clone()));
        }
        if self.dict.shortcut {
            results = div().flex().flex_col().gap_1();
            for v in &self.shortcuts {
                let v = v.clone();
                results = results.child(button(
                    format!("{}   {}", v.key, v.value),
                    c,
                    move |s, w, c| {
                        set(&s.dict.headword, v.key.clone(), w, c);
                        set(&s.dict.meanings, v.value.clone(), w, c);
                        Ok(())
                    },
                ));
            }
        }
        let controls = row()
            .child(button("Search", c, Self::search))
            .child(button("Previous", c, |s, _, c| {
                s.dict.offset = s.dict.offset.saturating_sub(40);
                s.page_results(c)
            }))
            .child(button("Next", c, |s, _, c| {
                if s.dict.offset + 40 < s.dict.total {
                    s.dict.offset += 40;
                }
                s.page_results(c)
            }))
            .child(button("Import…", c, Self::import_dictionary))
            .child(button("Export…", c, |s, _, _| {
                let Some(p) = FileDialog::new().save_file() else {
                    return Ok(());
                };
                let destination = p.to_string_lossy().into_owned();
                if s.dict.shortcut {
                    s.services.dictionaries.export_shortcuts(&destination)?;
                } else {
                    s.services.dictionaries.export(d::ExportRequest {
                        destination,
                        dictionary_id: s
                            .dict
                            .selected
                            .as_ref()
                            .context("Select a dictionary entry first")?
                            .dictionary_id
                            .clone(),
                    })?;
                }
                Ok(())
            }));
        let edit = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(if self.dict.shortcut {
                "Shortcut"
            } else {
                "Headword"
            })
            .child(Input::new(&self.dict.headword))
            .child("Meanings (one per line) / expansion")
            .child(Input::new(&self.dict.meanings).h(px(112.)))
            .child(
                row()
                    .child(button("Save entry", c, Self::edit_entry))
                    .child(button("History", c, |s, _, _| {
                        let v = s.dict.selected.as_ref().context("Select an entry")?;
                        s.dict.details = serde_json::to_string_pretty(
                            &s.services.dictionaries.entry_history(d::EntryKey {
                                dictionary_id: v.dictionary_id.clone(),
                                headword: v.entry.headword.clone(),
                            })?,
                        )?;
                        Ok(())
                    }))
                    .child(button("Delete entry", c, |s, w, c| {
                        if !confirm("Delete entry?", &value(&s.dict.headword, c)) {
                            return Ok(());
                        }
                        let store = &s.services.dictionaries;
                        if s.dict.shortcut {
                            store
                                .delete_shortcut(&value(&s.dict.headword, c), &s.services.state)?;
                        } else {
                            let v = s.dict.selected.as_ref().context("Select an entry")?;
                            anyhow::ensure!(
                                v.entry.headword == value(&s.dict.headword, c),
                                "Select the entry to delete"
                            );
                            store.delete_entry(
                                d::EntryKey {
                                    dictionary_id: v.dictionary_id.clone(),
                                    headword: v.entry.headword.clone(),
                                },
                                &s.services.state,
                            )?;
                        }
                        s.invalidate();
                        s.search(w, c)
                    })),
            )
            .child(self.dict.details.clone());
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(tabs)
            .when(!self.dict.shortcut && !self.dict.manage, |d| {
                d.child(if vi {
                    "Tìm kiếm"
                } else {
                    "Dictionary search"
                })
                .child(Input::new(&self.dict.query))
            })
            .child(controls)
            .when(self.dict.manage, |d| d.child(self.manage_dictionaries(c)))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_3()
                    .child(pane("Entries", scroll("results", results), c))
                    .child(pane("Entry editor", scroll("dict-editor", edit), c)),
            )
    }
}
