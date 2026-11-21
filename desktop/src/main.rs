mod ai;
mod dictionary;
mod files;
mod keyboard;
mod preferences;
mod review;
mod rich;
mod shell;

use anyhow::{Context as _, Result};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Root,
    input::{Input, InputEvent, InputState},
};
use quickertranslator_lib::{
    ai::{profiles::ProfileService, runtime::JobRuntime},
    dictionaries::DictionaryStore,
    documents::{DocumentService, Project},
    engine,
    models::*,
    settings::{Settings, SettingsStore},
    state::AppState,
};
use std::sync::Arc;
use uuid::Uuid;

struct Services {
    layout_path: std::path::PathBuf,
    state: Arc<AppState>,
    dictionaries: DictionaryStore,
    documents: DocumentService,
    settings: Arc<SettingsStore>,
    profiles: Arc<ProfileService>,
    jobs: Arc<JobRuntime>,
    runtime: tokio::runtime::Runtime,
}
impl Services {
    fn open() -> Result<Arc<Self>> {
        let path = dirs::data_dir()
            .context("No application data directory")?
            .join("io.github.quickertranslator.desktop");
        let state = Arc::new(AppState::default());
        state.initialize_tokenizer()?;
        let dictionaries = DictionaryStore::open(&path)?;
        state.replace_dictionaries(dictionaries.initial_snapshot()?)?;
        let settings = Arc::new(SettingsStore::open(&path)?);
        Ok(Arc::new(Self {
            layout_path: path.join("gpui-layout.json"),
            profiles: Arc::new(ProfileService::open(&path, settings.clone())?),
            settings,
            documents: DocumentService::open(&path)?,
            jobs: Arc::new(JobRuntime::new()?),
            runtime: tokio::runtime::Runtime::new()?,
            state,
            dictionaries,
        }))
    }
}
type Field = Entity<InputState>;
fn field(value: &str, multi: bool, w: &mut Window, cx: &mut App) -> Field {
    cx.new(|cx| {
        InputState::new(w, cx)
            .multi_line(multi)
            .default_value(value.to_owned())
    })
}
fn value(f: &Field, cx: &App) -> String {
    f.read(cx).value().to_string()
}
fn set(f: &Field, s: impl Into<SharedString>, w: &mut Window, cx: &mut App) {
    f.update(cx, |f, cx| f.set_value(s, w, cx));
}
fn selection(field: &Field, w: &mut Window, c: &mut App) -> Result<std::ops::Range<usize>> {
    let range = field
        .update(c, |x, c| x.selected_text_range(false, w, c))
        .context("No text selection")?
        .range;
    let offsets = engine::alignment::OffsetMap::new(&value(field, c))?;
    Ok(offsets.utf16_range_to_byte(TextRange {
        start: range.start.try_into()?,
        end: range.end.try_into()?,
    })?)
}
fn confirm(title: &str, detail: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(detail)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Workspace,
    Dictionaries,
    Preferences,
    Ai,
    Export,
}
struct Workspace {
    focus: FocusHandle,
    services: Arc<Services>,
    identity: DocumentWindow,
    project: Project,
    path: Option<String>,
    recovery_id: String,
    origin: Option<String>,
    source: Field,
    target: Field,
    draft: Field,
    structured: bool,
    paragraph: usize,
    segment: usize,
    revision: u64,
    target_revision: u64,
    run: u64,
    dirty: bool,
    draft_custom: bool,
    synced_draft: Option<String>,
    synced_target: Option<String>,
    translate_timer: Option<Task<()>>,
    result: Option<TranslationResult>,
    settings: Settings,
    export_columns: Vec<quickertranslator_lib::documents::ExportColumn>,
    blank_lines: u8,
    advanced: bool,
    preview: bool,
    recovery_timer: Option<Task<()>>,
    preferences: Field,
    typed: Option<(usize, String)>,
    shortcuts: Vec<quickertranslator_lib::dictionaries::ShortcutRecord>,
    page: Page,
    layout: shell::Layout,
    status: String,
    dict: dictionary::Panel,
    ai: ai::Panel,
}

type UiHandler = fn(&mut Workspace, &mut Window, &mut Context<Workspace>) -> Result<()>;
fn button(
    label: impl Into<SharedString>,
    cx: &Context<Workspace>,
    action: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) -> Result<()> + 'static,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(label.clone())
        .px_2()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .cursor_pointer()
        .hover(|s| s.bg(cx.theme().secondary))
        .child(label)
        .on_click(cx.listener(move |this, _, w, cx| {
            let revision = this
                .services
                .state
                .dictionaries()
                .ok()
                .map(|d| d.revision());
            if let Err(e) = action(this, w, cx) {
                this.status = e.to_string();
            }
            if revision
                != this
                    .services
                    .state
                    .dictionaries()
                    .ok()
                    .map(|d| d.revision())
            {
                cx.set_global(preferences::DictionariesChanged);
            }
            cx.notify();
        }))
}
fn row() -> Div {
    div().flex().flex_wrap().items_center().gap_2()
}
fn pane(title: &str, body: impl IntoElement, cx: &App) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .bg(cx.theme().background)
                .child(body),
        )
}
fn scroll(id: &'static str, body: impl IntoElement) -> impl IntoElement {
    div()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .child(body)
}
impl Workspace {
    fn new(
        loaded: quickertranslator_lib::settings::SettingsLoad,
        services: Arc<Services>,
        identity: DocumentWindow,
        project: Project,
        path: Option<String>,
        dirty: bool,
        recovery_id: Option<String>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let preferences = field(
            &serde_json::to_string_pretty(&loaded.settings).unwrap(),
            true,
            w,
            cx,
        );
        let source = cx.new(|c| {
            InputState::new(w, c)
                .multi_line(true)
                .searchable(true)
                .soft_wrap(
                    project
                        .view_state
                        .wraps
                        .as_ref()
                        .map_or(project.view_state.wrap, |o| o.source),
                )
                .default_value(project.source_text.clone())
        });
        let plain = quickertranslator_lib::documents::schema::plain_text(&project.target_document);
        let structured = files::has_formatting(&project.target_document);
        let target = field(
            &if structured {
                rich::paragraphs(&project.target_document)
                    .first()
                    .map(|path| {
                        quickertranslator_lib::documents::schema::plain_text(rich::block(
                            &project.target_document,
                            path,
                        ))
                    })
                    .unwrap_or_default()
            } else {
                plain
            },
            true,
            w,
            cx,
        );
        let draft_custom = !project.draft_edits.overrides.is_empty()
            || !project.draft_edits.order.is_empty()
            || project
                .draft_edits
                .preview_text
                .as_ref()
                .is_some_and(|s| !s.is_empty());
        let draft = field(
            project.draft_edits.preview_text.as_deref().unwrap_or(""),
            true,
            w,
            cx,
        );
        for (index, input) in [&source, &target, &draft].into_iter().enumerate() {
            cx.subscribe_in(
                input,
                w,
                move |this: &mut Self, _, ev: &InputEvent, w, cx| {
                    if index == 0 && matches!(ev, InputEvent::Focus | InputEvent::Blur) {
                        this.layout.source_edit = matches!(ev, InputEvent::Focus);
                        cx.notify();
                    }
                    if matches!(ev, InputEvent::Change) {
                        if index == 2
                            && this
                                .synced_draft
                                .take()
                                .is_some_and(|v| v == value(&this.draft, cx))
                        {
                            return;
                        }
                        if index == 1
                            && this
                                .synced_target
                                .take()
                                .is_some_and(|v| v == value(&this.target, cx))
                        {
                            return;
                        }
                        this.dirty = true;
                        if index == 0 {
                            this.invalidate();
                            this.translate_timer = Some(cx.spawn_in(w, async move |this, c| {
                                c.background_executor()
                                    .timer(std::time::Duration::from_millis(300))
                                    .await;
                                let _ = this.update_in(c, |s, w, c| {
                                    if let Err(e) = s.translate(w, c) {
                                        s.status = e.to_string();
                                    }
                                    c.notify();
                                });
                            }));
                        }
                        if index == 2 {
                            this.draft_custom = true;
                            this.project.draft_edits =
                                Project::new(this.project.source_language, value(&this.source, cx))
                                    .draft_edits;
                            this.project.draft_edits.source_revision = this.revision;
                        }
                        if index == 1 {
                            if this.structured {
                                this.sync_paragraph(cx);
                            }
                            this.ai.review = None;
                            this.target_revision += 1;
                            let _ = this.services.state.observe_target(
                                &this.identity.window_label,
                                &this.identity.document_id,
                                this.target_revision,
                            );
                        }
                        this.recovery_timer = Some(cx.spawn(async move |this, c| {
                            c.background_executor()
                                .timer(std::time::Duration::from_millis(500))
                                .await;
                            let _ = this.update(c, |s, c| {
                                if s.dirty {
                                    if let Err(e) = s.recover(c) {
                                        s.status = e.to_string();
                                        c.notify();
                                    }
                                }
                            });
                        }));
                        cx.notify();
                    }
                },
            )
            .detach();
        }
        cx.observe_global_in::<preferences::DictionariesChanged>(w, |s, w, c| {
            s.invalidate();
            s.shortcuts = s.services.dictionaries.shortcuts().unwrap_or_default();
            if let Err(e) = s.translate(w, c) {
                s.status = e.to_string();
            }
            c.notify()
        })
        .detach();
        cx.observe_global_in::<preferences::PreferencesChanged>(w, |s, w, c| {
            if let Ok(loaded) = s.services.settings.load() {
                let changed = loaded.settings.translation_options != s.settings.translation_options;
                s.settings = loaded.settings;
                if changed {
                    s.invalidate();
                    if let Err(e) = s.translate(w, c) {
                        s.status = e.to_string();
                    }
                }
                set(
                    &s.preferences,
                    serde_json::to_string_pretty(&s.settings).unwrap(),
                    w,
                    c,
                );
            }
            c.notify();
        })
        .detach();
        let dict = dictionary::Panel::new(w, cx);
        let ai = ai::Panel::new(w, cx);
        let recovery_id = recovery_id.unwrap_or_else(|| identity.document_id.clone());
        let layout = shell::Layout::load(&services.layout_path);
        Self {
            focus: cx.focus_handle(),
            shortcuts: services.dictionaries.shortcuts().unwrap_or_default(),
            typed: None,
            services,
            identity,
            project,
            path: path.clone().filter(|p| {
                std::path::Path::new(p)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("qtp"))
            }),
            recovery_id,
            origin: path.clone(),
            source,
            target,
            draft,
            structured,
            paragraph: 0,
            segment: 0,
            revision: 1,
            target_revision: 0,
            run: 0,
            dirty,
            result: None,
            draft_custom,
            synced_draft: None,
            synced_target: None,
            translate_timer: None,
            settings: loaded.settings,
            advanced: false,
            export_columns: vec![quickertranslator_lib::documents::ExportColumn::Target],
            blank_lines: 0,
            preview: structured,
            recovery_timer: None,
            preferences,
            page: Page::Workspace,
            layout,
            status: loaded.warning.unwrap_or_default(),
            dict,
            ai,
        }
    }
    fn invalidate(&mut self) {
        self.revision += 1;
        self.run += 1;
        self.result = None;
        self.segment = 0;
        self.layout.selected = false;
        self.ai.review = None;
        self.ai.completed = None;
        let _ = self
            .services
            .state
            .cancel_translation(&self.identity.window_label);
        if let Some(job) = &self.ai.active {
            let _ = self.services.jobs.cancel(&self.identity.window_label, job);
        }
    }
    fn translate(&mut self, w: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        self.run += 1;
        let run = self.run;
        let request = TranslationRequest {
            document_id: self.identity.document_id.clone(),
            source_revision: self.revision,
            source_language: self.project.source_language,
            source_text: value(&self.source, cx),
            options: self.settings.translation_options.clone(),
        };
        let state = self.services.state.clone();
        let label = self.identity.window_label.clone();
        self.status = "Translating…".into();
        let job = cx.background_executor().spawn(async move {
            let work = state.prepare_translation(&label, request)?;
            let result = match work.source.language {
                SourceLanguage::Zh => engine::chinese::translate(&work),
                SourceLanguage::Ja => engine::japanese::translate(&work),
            };
            state.finish_translation(&work, result)
        });
        cx.spawn_in(w, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, w, cx| {
                if this.run != run {
                    return;
                }
                match result {
                    Ok(r) => {
                        if !this.draft_custom {
                            this.synced_draft = Some(r.single_meaning.clone());
                            set(&this.draft, r.single_meaning.clone(), w, cx);
                            this.project.draft_edits =
                                Project::new(this.project.source_language, value(&this.source, cx))
                                    .draft_edits;
                            this.project.draft_edits.source_revision = this.revision;
                        }
                        this.segment = this.segment.min(r.segments.len().saturating_sub(1));
                        this.status = format!(
                            "{}/{} · {} r{}",
                            r.segments
                                .iter()
                                .filter(|v| !v.unknown && !v.meanings.is_empty())
                                .count(),
                            r.segments
                                .iter()
                                .filter(|v| v.unknown || !v.meanings.is_empty())
                                .count(),
                            if this.settings.locale == UiLocale::Vi {
                                "Từ điển đã nạp"
                            } else {
                                "Dictionaries loaded"
                            },
                            r.dictionary_revision
                        );
                        this.result = Some(r);
                    }
                    Err(e) => this.status = e.message,
                }
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }
}
impl Render for Workspace {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.shell(w, cx)
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|s, event, w, c| {
                match s.key(event, w, c) {
                    Ok(false) => return,
                    Err(e) => s.status = e.to_string(),
                    _ => {}
                }
                c.stop_propagation();
                w.prevent_default();
                c.notify();
            }))
            .on_action(cx.listener(|s, _: &Save, w, c| {
                if let Err(e) = s.save(w, c) {
                    s.status = e.to_string()
                }
                c.notify()
            }))
            .on_action(cx.listener(|s, _: &Open, w, c| {
                if let Err(e) = s.open(w, c) {
                    s.status = e.to_string()
                }
                c.notify()
            }))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        self.services
            .jobs
            .remove_window(&self.identity.window_label);
        let _ = self
            .services
            .state
            .remove_window(&self.identity.window_label);
        let _ = self
            .services
            .documents
            .release_window(&self.identity.window_label);
    }
}
actions!(quicktranslator, [Open, Save]);
fn apply_theme(settings: &Settings, cx: &mut App) {
    use gpui_component::{Theme, ThemeMode};
    Theme::change(
        match settings.theme {
            quickertranslator_lib::settings::Theme::Dark => ThemeMode::Dark,
            _ => ThemeMode::Light,
        },
        None,
        cx,
    );
}
fn open_window(
    services: Arc<Services>,
    project: Project,
    path: Option<String>,
    dirty: bool,
    recovery_id: Option<String>,
    cx: &mut App,
) -> Result<()> {
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let identity = services
        .state
        .register_window(&Uuid::new_v4().to_string())?;
    let loaded = services.settings.load()?;
    let label = identity.window_label.clone();
    let backend = services.clone();
    if let Some(id) = &recovery_id {
        services.documents.read_recovery(&label, id)?;
    }
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        |w, c| {
            let view = c.new(|cx| {
                Workspace::new(
                    loaded,
                    services,
                    identity,
                    project,
                    path,
                    dirty,
                    recovery_id,
                    w,
                    cx,
                )
            });
            view.update(c, |s, c| {
                if !s.project.source_text.is_empty() {
                    if let Err(e) = s.translate(w, c) {
                        s.status = e.to_string();
                    }
                }
            });
            view.read(c).focus.focus(w);
            let weak = view.downgrade();
            w.on_window_should_close(c, move |_, cx| {
                weak.update(cx, |s, c| {
                    if !s.dirty {
                        return true;
                    }
                    match s.recover(c) {
                        Ok(()) => confirm(
                            "Close document?",
                            "Unsaved changes will be available under Recover.",
                        ),
                        Err(e) => {
                            s.status = e.to_string();
                            c.notify();
                            false
                        }
                    }
                })
                .unwrap_or(true)
            });
            c.new(|cx| Root::new(view, w, cx))
        },
    );
    if let Err(e) = result {
        backend.state.remove_window(&label)?;
        backend.documents.release_window(&label)?;
        return Err(e);
    }
    Ok(())
}
fn main() -> Result<()> {
    let services = Services::open()?;
    let settings = services.settings.load()?.settings;
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            apply_theme(&settings, cx);
            let modifier = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            cx.bind_keys([
                KeyBinding::new(&format!("{modifier}-o"), Open, None),
                KeyBinding::new(&format!("{modifier}-s"), Save, None),
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit()
                }
            })
            .detach();
            let result = if let Some(path) = std::env::args_os().nth(1) {
                files::open_path(
                    services,
                    SourceLanguage::Zh,
                    path.to_string_lossy().into_owned(),
                    cx,
                )
            } else {
                open_window(
                    services,
                    Project::new(SourceLanguage::Zh, String::new()),
                    None,
                    false,
                    None,
                    cx,
                )
            };
            if let Err(e) = result {
                eprintln!("{e}");
                cx.quit();
            }
            cx.activate(true);
        });
    Ok(())
}
