use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::models::{AppError, AppResult};
use super::*;

type Service<'a> = State<'a, Arc<DocumentService>>;
fn worker_error() -> AppError { AppError::new("documentWorkerUnavailable", "Document work was interrupted. The current editor contents were not replaced.") }
async fn work<T, F>(operation: F) -> AppResult<T> where T: Send + 'static, F: FnOnce() -> AppResult<T> + Send + 'static {
    tokio::task::spawn_blocking(operation).await.map_err(|_| worker_error())?
}
#[tauri::command]
pub async fn preview_document_import(request: ImportRequest, _window: WebviewWindow) -> AppResult<DocumentImportPreview> {
    work(move || preview_import(&request)).await
}
#[tauri::command]
pub async fn open_document(request: ImportRequest, _window: WebviewWindow) -> AppResult<ImportOutcome> {
    work(move || open_import(&request)).await
}
#[tauri::command]
pub async fn save_document(request: SaveRequest, _window: WebviewWindow) -> AppResult<SaveResult> {
    work(move || save_project(&request)).await
}
#[tauri::command]
pub async fn export_document(request: ExportRequest, _window: WebviewWindow) -> AppResult<SaveResult> {
    work(move || export::export_document(&request)).await
}
#[tauri::command]
pub async fn document_siblings(path: String, _window: WebviewWindow) -> AppResult<SiblingDocuments> {
    work(move || sibling_documents(&path)).await
}
#[tauri::command]
pub fn set_document_title(name: String, dirty: bool, window: WebviewWindow) -> AppResult<()> {
    let name: String = name.chars().filter(|ch| !ch.is_control()).take(256).collect();
    let title = format!("{}{} — QuickTranslator", if dirty { "* " } else { "" }, if name.is_empty() { "Untitled" } else { &name });
    window.set_title(&title).map_err(|_| AppError::new("windowTitleUnavailable", "The native window title could not be updated."))
}
#[tauri::command]
pub async fn recovery_list(window: WebviewWindow, service: Service<'_>) -> AppResult<RecoveryList> {
    let window = window.label().to_owned(); let service = Arc::clone(service.inner());
    work(move || service.list_recovery(&window)).await
}
#[tauri::command]
pub async fn recovery_read(document_id: String, window: WebviewWindow, service: Service<'_>) -> AppResult<RecoveryRecord> {
    let window = window.label().to_owned(); let service = Arc::clone(service.inner());
    work(move || service.read_recovery(&window, &document_id)).await
}
#[tauri::command]
pub async fn recovery_release(document_id: String, window: WebviewWindow, service: Service<'_>) -> AppResult<()> {
    let window = window.label().to_owned(); let service = Arc::clone(service.inner());
    work(move || service.release_recovery(&window, &document_id)).await
}
#[tauri::command]
pub async fn recovery_write(request: RecoveryWrite, window: WebviewWindow, service: Service<'_>) -> AppResult<()> {
    let window = window.label().to_owned(); let service = Arc::clone(service.inner());
    work(move || service.write_recovery(&window, &request)).await
}
#[tauri::command]
pub async fn recovery_discard(document_id: String, window: WebviewWindow, service: Service<'_>) -> AppResult<()> {
    let window = window.label().to_owned(); let service = Arc::clone(service.inner());
    work(move || service.discard_recovery(&window, &document_id)).await
}
