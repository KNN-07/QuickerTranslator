use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::{models::{AppError, AppResult}, state::AppState};
use super::{DictionaryStore, import, types::*};

type Store<'a> = State<'a, Arc<DictionaryStore>>;
type Application<'a> = State<'a, Arc<AppState>>;

async fn reading<T, F>(store: Arc<DictionaryStore>, operation: F) -> AppResult<T>
where T: Send + 'static, F: FnOnce(&DictionaryStore) -> AppResult<T> + Send + 'static {
    tokio::task::spawn_blocking(move || operation(&store)).await.map_err(|_| worker_error())?
}

#[derive(Clone, Serialize)]
struct DictionaryChanged { revision: u64 }

async fn changing<T, F>(store: Arc<DictionaryStore>, state: Arc<AppState>, app: AppHandle, operation: F) -> AppResult<T>
where T: Send + 'static, F: FnOnce(&DictionaryStore, &AppState) -> AppResult<T> + Send + 'static {
    let (result, revision) = tokio::task::spawn_blocking(move || {
        let result = operation(&store, &state)?;
        Ok::<_, AppError>((result, state.dictionaries()?.revision()))
    }).await.map_err(|_| worker_error())??;
    // Publication already cancelled stale work in every native window. Event
    // delivery is notification only; a missed listener can query the revision.
    let _ = app.emit("dictionaries-changed", DictionaryChanged { revision });
    Ok(result)
}

fn worker_error() -> AppError { AppError::new("dictionaryWorkerUnavailable", "Dictionary work was interrupted; refresh the dictionary state before trying again.") }

#[tauri::command]
pub async fn dictionary_catalog(store: Store<'_>) -> AppResult<DictionaryCatalog> {
    reading(Arc::clone(store.inner()), DictionaryStore::catalog).await
}

#[tauri::command]
pub async fn dictionary_storage_status(store: Store<'_>) -> AppResult<DictionaryStorageStatus> {
    reading(Arc::clone(store.inner()), DictionaryStore::storage_status).await
}

#[tauri::command]
pub async fn preview_dictionary_config(request: ConfigRequest) -> AppResult<ConfigPreview> {
    tokio::task::spawn_blocking(move || import::preview_config(&request)).await.map_err(|_| worker_error())?
}

#[tauri::command]
pub async fn preview_dictionary_import(request: ImportRequest, store: Store<'_>) -> AppResult<ImportPreview> {
    reading(Arc::clone(store.inner()), move |store| store.preview_import(request)).await
}

#[tauri::command]
pub async fn commit_dictionary_import(request: CommitImportRequest, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<u64> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.commit_import(request, state)).await
}

#[tauri::command]
pub async fn reload_dictionary(dictionary_id: String, store: Store<'_>) -> AppResult<ImportPreview> {
    reading(Arc::clone(store.inner()), move |store| store.reload_preview(&dictionary_id)).await
}

#[tauri::command]
pub async fn search_dictionary_entries(request: SearchRequest, store: Store<'_>) -> AppResult<SearchResult> {
    reading(Arc::clone(store.inner()), move |store| store.search(request)).await
}

#[tauri::command]
pub async fn save_dictionary_metadata(request: MetadataMutation, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<DictionaryMetadata> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.save_metadata(request, state)).await
}

#[tauri::command]
pub async fn save_dictionary_entry(request: EntryMutation, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<u64> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.save_entry(request, state)).await
}

#[tauri::command]
pub async fn delete_dictionary_entry(request: EntryKey, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<u64> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.delete_entry(request, state)).await
}

#[tauri::command]
pub async fn dictionary_entry_history(request: EntryKey, store: Store<'_>) -> AppResult<Vec<EntryHistory>> {
    reading(Arc::clone(store.inner()), move |store| store.entry_history(request)).await
}

#[tauri::command]
pub async fn export_dictionary(request: ExportRequest, store: Store<'_>) -> AppResult<ExportResult> {
    reading(Arc::clone(store.inner()), move |store| store.export(request)).await
}

#[tauri::command]
pub async fn list_shortcuts(store: Store<'_>) -> AppResult<Vec<ShortcutRecord>> {
    reading(Arc::clone(store.inner()), DictionaryStore::shortcuts).await
}

#[tauri::command]
pub async fn preview_shortcuts_import(request: ShortcutImportRequest, store: Store<'_>) -> AppResult<ShortcutPreview> {
    reading(Arc::clone(store.inner()), move |store| store.preview_shortcuts(request)).await
}

#[tauri::command]
pub async fn commit_shortcuts_import(preview_id: String, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<u64> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.commit_shortcuts(&preview_id, state)).await
}

#[tauri::command]
pub async fn save_shortcut(request: ShortcutRecord, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<u64> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.save_shortcut(request, state)).await
}

#[tauri::command]
pub async fn delete_shortcut(key: String, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<u64> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.delete_shortcut(&key, state)).await
}

#[tauri::command]
pub async fn export_shortcuts(destination: String, store: Store<'_>) -> AppResult<ExportResult> {
    reading(Arc::clone(store.inner()), move |store| store.export_shortcuts(&destination)).await
}

#[tauri::command]
pub async fn repair_dictionary_database(request: RepairDatabaseRequest, store: Store<'_>, state: Application<'_>, app: AppHandle) -> AppResult<u64> {
    changing(Arc::clone(store.inner()), Arc::clone(state.inner()), app, move |store, state| store.repair_database(request, state)).await
}
