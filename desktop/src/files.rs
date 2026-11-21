use super::*;
use quickertranslator_lib::documents::{
    self as doc, ExportColumn, ExportFormat, ImportOutcome, RecoveryWrite, SaveRequest,
};
use rfd::FileDialog;
use serde_json::{Value, json};

fn text_doc(text: &str) -> Value {
    json!({"type":"doc","content":text.split('\n').map(|line|if line.is_empty(){json!({"type":"paragraph"})}else{json!({"type":"paragraph","content":[{"type":"text","text":line}]})}).collect::<Vec<_>>()})
}
pub fn has_formatting(doc: &Value) -> bool {
    text_doc(&doc::schema::plain_text(doc)) != *doc
}
impl Workspace {
    pub fn sync_paragraph(&mut self, c: &App) {
        if let Some(path) = rich::paragraphs(&self.project.target_document).get(self.paragraph) {
            rich::edit(
                rich::block_mut(&mut self.project.target_document, path),
                &value(&self.target, c),
            );
        }
    }
    fn show_paragraph(&mut self, w: &mut Window, c: &mut Context<Self>) {
        if let Some(path) = rich::paragraphs(&self.project.target_document).get(self.paragraph) {
            self.synced_target = Some(doc::schema::plain_text(rich::block(
                &self.project.target_document,
                path,
            )));
            set(
                &self.target,
                doc::schema::plain_text(rich::block(&self.project.target_document, path)),
                w,
                c,
            );
        }
    }
    pub fn change_paragraph(
        &mut self,
        delta: isize,
        w: &mut Window,
        c: &mut Context<Self>,
    ) -> Result<()> {
        self.sync_paragraph(c);
        self.paragraph = self.paragraph.saturating_add_signed(delta).min(
            rich::paragraphs(&self.project.target_document)
                .len()
                .saturating_sub(1),
        );
        self.show_paragraph(w, c);
        Ok(())
    }
    pub fn add_paragraph(&mut self, w: &mut Window, c: &mut Context<Self>) -> Result<()> {
        self.sync_paragraph(c);
        self.project.target_document["content"]
            .as_array_mut()
            .context("Missing document blocks")?
            .push(json!({"type":"paragraph"}));
        self.paragraph = rich::paragraphs(&self.project.target_document).len() - 1;
        self.show_paragraph(w, c);
        self.dirty = true;
        self.recover(c)
    }
    pub fn apply_format(
        &mut self,
        kind: &str,
        style: Option<String>,
        w: &mut Window,
        c: &mut Context<Self>,
    ) -> Result<()> {
        let selected = selection(&self.target, w, c)?;
        anyhow::ensure!(
            kind == "textAlign" || !selected.is_empty(),
            "Select Vietnamese text to format"
        );
        let mut project = self.snapshot(c)?;
        let paths = rich::paragraphs(&project.target_document);
        let mut offset = 0;
        let mut first = None;
        for (i, path) in paths.iter().enumerate() {
            if self.structured && i != self.paragraph {
                continue;
            }
            let block = rich::block_mut(&mut project.target_document, path);
            let len = doc::schema::plain_text(block).len();
            let start = selected.start.max(offset);
            let end = selected.end.min(offset + len);
            if start < end
                || (kind == "textAlign" && (offset..=offset + len).contains(&selected.start))
            {
                let range = start - offset..end - offset;
                match (kind, style.as_deref()) {
                    ("textAlign", Some(v)) => block["attrs"]["textAlign"] = json!(v),
                    (key, Some(v)) => rich::set_style(block, range, key, v)?,
                    (mark, None) => rich::toggle_mark(block, range, mark)?,
                }
                first.get_or_insert(i);
            }
            offset += len + 1;
        }
        project.validate()?;
        self.project.target_document = project.target_document;
        self.paragraph = first.context("Select a paragraph to format")?;
        self.structured = true;
        self.show_paragraph(w, c);
        self.preview = true;
        self.dirty = true;
        self.recover(c)
    }
    pub fn snapshot(&self, c: &App) -> Result<Project> {
        let mut project = self.project.clone();
        project.source_text = value(&self.source, c);
        project.target_document = if self.structured {
            let mut doc = project.target_document.clone();
            if let Some(path) = rich::paragraphs(&doc).get(self.paragraph) {
                rich::edit(rich::block_mut(&mut doc, path), &value(&self.target, c));
            }
            doc
        } else {
            text_doc(&value(&self.target, c))
        };
        project.draft_edits.preview_text = Some(value(&self.draft, c));
        project.validate()?;
        Ok(project)
    }
    pub fn recover(&self, c: &App) -> Result<()> {
        let project = self.snapshot(c)?;
        self.services.documents.write_recovery(
            &self.identity.window_label,
            &RecoveryWrite {
                document_id: self.recovery_id.clone(),
                name: self.path.clone().unwrap_or_else(|| "Untitled".into()),
                path: self.path.clone(),
                project,
            },
        )?;
        Ok(())
    }
    pub fn insert_text(
        &mut self,
        text: String,
        w: &mut Window,
        c: &mut Context<Self>,
    ) -> Result<()> {
        self.target
            .update(c, |x, c| x.replace_text_in_range(None, &text, w, c));
        if self.structured {
            self.sync_paragraph(c);
        }
        self.target_revision += 1;
        self.services.state.observe_target(
            &self.identity.window_label,
            &self.identity.document_id,
            self.target_revision,
        )?;
        self.dirty = true;
        self.recover(c)
    }
    pub fn new_file(&mut self, _: &mut Window, c: &mut Context<Self>) -> Result<()> {
        open_window(
            self.services.clone(),
            Project::new(self.project.source_language, String::new()),
            None,
            false,
            None,
            c,
        )
    }
    pub fn open(&mut self, _: &mut Window, c: &mut Context<Self>) -> Result<()> {
        let Some(path) = FileDialog::new()
            .add_filter(
                "Translation documents",
                &["qtp", "qt", "txt", "html", "htm"],
            )
            .pick_file()
        else {
            return Ok(());
        };
        open_path(
            self.services.clone(),
            self.project.source_language,
            path.to_string_lossy().into_owned(),
            c,
        )
    }
    pub fn save(&mut self, w: &mut Window, c: &mut Context<Self>) -> Result<()> {
        if let Some(path) = self.path.clone() {
            self.save_to(path, c)
        } else {
            self.save_as(w, c)
        }
    }
    pub fn save_as(&mut self, _: &mut Window, c: &mut Context<Self>) -> Result<()> {
        let Some(path) = FileDialog::new()
            .add_filter("QuickTranslator project", &["qtp"])
            .set_file_name("translation.qtp")
            .save_file()
        else {
            return Ok(());
        };
        self.save_to(path.to_string_lossy().into_owned(), c)
    }
    fn save_to(&mut self, path: String, c: &App) -> Result<()> {
        let project = self.snapshot(c)?;
        doc::save_project(&SaveRequest {
            path: path.clone(),
            project: project.clone(),
        })?;
        self.project = project;
        self.origin = Some(path.clone());
        self.path = Some(path);
        self.dirty = false;
        self.services
            .documents
            .discard_recovery(&self.identity.window_label, &self.recovery_id)?;
        self.status = "Saved".into();
        Ok(())
    }
    pub fn export(&mut self, _: &mut Window, _: &mut Context<Self>) -> Result<()> {
        self.page = Page::Export;
        Ok(())
    }
    pub fn export_page(&self, c: &Context<Self>) -> Div {
        use ExportColumn::*;
        let vi = self.settings.locale == UiLocale::Vi;
        let mut form = div().flex().flex_col().gap_2().child(if vi {
            "Chọn cột theo thứ tự xuất"
        } else {
            "Select columns in export order"
        });
        for col in [Source, Readings, Phrases, SingleMeaning, Target] {
            let pos = self.export_columns.iter().position(|v| *v == col);
            form = form.child(button(
                format!(
                    "{}   {:?}",
                    pos.map(|p| (p + 1).to_string())
                        .unwrap_or_else(|| "☐".into()),
                    col
                ),
                c,
                move |s, _, _| {
                    if let Some(i) = s.export_columns.iter().position(|v| *v == col) {
                        s.export_columns.remove(i);
                    } else {
                        s.export_columns.push(col);
                    }
                    Ok(())
                },
            ));
        }
        form.child(button(
            format!("Blank lines · {}", self.blank_lines),
            c,
            |s, _, _| {
                s.blank_lines = (s.blank_lines + 1) % 5;
                Ok(())
            },
        ))
        .child(button(
            if vi {
                "Chọn tệp đích…"
            } else {
                "Choose destination…"
            },
            c,
            Self::export_file,
        ))
    }
    fn export_file(&mut self, _: &mut Window, c: &mut Context<Self>) -> Result<()> {
        let Some(path) = FileDialog::new()
            .add_filter("Vietnamese translation", &["txt", "html", "docx", "rtf"])
            .set_file_name("translation.txt")
            .save_file()
        else {
            return Ok(());
        };
        let format = match path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "txt" => ExportFormat::Txt,
            "html" => ExportFormat::Html,
            "docx" => ExportFormat::Docx,
            "rtf" => ExportFormat::Rtf,
            _ => anyhow::bail!("Choose TXT, HTML, DOCX or RTF"),
        };
        doc::export::export_document(&doc::ExportRequest {
            path: path.to_string_lossy().into_owned(),
            format,
            project: self.snapshot(c)?,
            columns: self.export_columns.clone(),
            readings: self
                .result
                .as_ref()
                .map(|v| v.readings.clone())
                .unwrap_or_default(),
            phrases: self
                .result
                .as_ref()
                .map(|v| v.phrases.clone())
                .unwrap_or_default(),
            single_meaning: value(&self.draft, c),
            blank_lines: self.blank_lines,
        })?;
        self.page = Page::Workspace;
        self.status = "Exported translation".into();
        Ok(())
    }
    pub fn restore(&mut self, _: &mut Window, c: &mut Context<Self>) -> Result<()> {
        let docs = &self.services.documents;
        let list = docs.list_recovery(&self.identity.window_label)?;
        self.status = if list.warnings.is_empty() {
            "No other recoverable documents selected".into()
        } else {
            list.warnings.join("\n")
        };
        for record in list
            .records
            .into_iter()
            .filter(|r| r.document_id != self.recovery_id)
        {
            if confirm(
                "Restore this document?",
                &format!("{}\n{}", record.name, record.document_id),
            ) {
                let r = docs.read_recovery(&self.identity.window_label, &record.document_id)?;
                docs.release_recovery(&self.identity.window_label, &record.document_id)?;
                return open_window(
                    self.services.clone(),
                    r.project,
                    r.path,
                    true,
                    Some(record.document_id),
                    c,
                );
            }
        }
        Ok(())
    }
}
pub fn open_path(
    services: Arc<Services>,
    source_language: SourceLanguage,
    path: String,
    c: &mut App,
) -> Result<()> {
    let mut request = doc::ImportRequest {
        path,
        encoding: None,
        source_language,
        confirm_detected_encoding: false,
    };
    loop {
        match doc::open_import(&request)? {
            ImportOutcome::Ready {
                project,
                path,
                imported,
                warnings,
            } => {
                if !warnings.is_empty() {
                    rfd::MessageDialog::new()
                        .set_title("Import notes")
                        .set_description(warnings.join("\n"))
                        .show();
                }
                return open_window(services.clone(), project, Some(path), imported, None, c);
            }
            ImportOutcome::EncodingConfirmationRequired { preview } => {
                if !confirm(
                    &format!("Open as {}?", preview.encoding),
                    &preview.preview_text,
                ) {
                    return Ok(());
                }
                request.confirm_detected_encoding = true;
            }
        }
    }
}
