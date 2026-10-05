use std::sync::Arc;

use tauri::{AppHandle, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;

use crate::{
    models::{AppError, AppResult, DictionaryKind, DocumentWindow, FoundationHealth, SourceLanguage, TranslationOptions, TranslationRequest, TranslationResult},
    state::{AppState, DictionaryEntry, dispatch_translation},
};

#[tauri::command]
pub fn foundation_health(window: WebviewWindow, state: State<'_, Arc<AppState>>) -> AppResult<FoundationHealth> {
    state.health(window.label())
}

/// The only window-creation surface: bundled index.html with a native-generated
/// label. Frontend callers cannot supply arbitrary URLs or window identities.
#[tauri::command]
pub async fn new_document_window(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> AppResult<DocumentWindow> {
    state.document_window(window.label())?;
    let label = format!("document-{}", Uuid::new_v4().simple());
    let identity = state.register_window(&label)?;
    let result = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("QuickTranslator")
        .inner_size(1280.0, 800.0)
        .min_inner_size(900.0, 600.0)
        .decorations(true)
        .resizable(true)
        .build();
    if result.is_err() {
        state.remove_window(&label)?;
        return Err(AppError::new("windowCreationFailed", "A new native document window could not be opened."));
    }
    Ok(identity)
}

#[tauri::command]
pub async fn observe_document_source(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
    document_id: String,
    source_revision: u64,
    source_language: SourceLanguage,
    source_text: String,
    options: TranslationOptions,
) -> AppResult<()> {
    let state = Arc::clone(state.inner());
    let label = window.label().to_owned();
    tokio::task::spawn_blocking(move || {
        state.observe_source(&label, &document_id, source_revision, source_language, source_text)?;
        state.observe_options(&label, &document_id, source_revision, options)?;
        Ok(())
    }).await.map_err(|_| AppError::new("translationWorkerFailed", "Source synchronization was interrupted."))?
}

#[tauri::command]
pub fn observe_document_target(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
    document_id: String,
    target_revision: u64,
) -> AppResult<()> {
    state.observe_target(window.label(), &document_id, target_revision)
}

#[tauri::command]
pub fn cancel_offline_translation(window: WebviewWindow, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    state.cancel_translation(window.label())
}

#[tauri::command]
pub async fn translate_offline(
    request: TranslationRequest,
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> AppResult<TranslationResult> {
    dispatch_translation(Arc::clone(state.inner()), window.label().to_owned(), request, |work| {
        match work.source.language {
            SourceLanguage::Zh => crate::engine::chinese::translate(work),
            SourceLanguage::Ja => crate::engine::japanese::translate(work),
        }
    }).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryLookupEntry {
    kind: DictionaryKind,
    #[serde(flatten)]
    entry: Arc<DictionaryEntry>,
}

#[tauri::command]
pub fn lookup_dictionary_entries(
    headword: String,
    language: SourceLanguage,
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<DictionaryLookupEntry>> {
    let snapshot = state.dictionaries()?;
    let kinds = [
        DictionaryKind::PrimaryNames, DictionaryKind::SecondaryNames,
        DictionaryKind::VietPhrase, DictionaryKind::HanViet, DictionaryKind::Japanese,
        DictionaryKind::Pronouns, DictionaryKind::Rules, DictionaryKind::Ignored,
        DictionaryKind::Cedict, DictionaryKind::Babylon, DictionaryKind::LacViet,
        DictionaryKind::ThieuChuu, DictionaryKind::Auxiliary,
    ];
    Ok(kinds.into_iter().filter_map(|kind| {
        snapshot.lookup(language, kind, &headword)
            .map(|entry| DictionaryLookupEntry { kind, entry: Arc::clone(entry) })
    }).collect())
}
