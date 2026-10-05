use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{documents::SaveResult, models::{AppError, AppResult}};
use super::{Settings, SettingsLoad, SettingsStore};

type Store<'a> = State<'a, Arc<SettingsStore>>;
fn worker_error() -> AppError { AppError::new("settingsWorkerUnavailable", "Settings work was interrupted. The previous settings file was not replaced.") }
#[tauri::command]
pub async fn load_settings(store: Store<'_>, _window: WebviewWindow) -> AppResult<SettingsLoad> {
    let store = Arc::clone(store.inner());
    tokio::task::spawn_blocking(move || store.load()).await.map_err(|_| worker_error())?
}
#[tauri::command]
pub async fn save_settings(settings: Settings, replace_invalid: Option<bool>, store: Store<'_>, _window: WebviewWindow) -> AppResult<SaveResult> {
    let store = Arc::clone(store.inner());
    tokio::task::spawn_blocking(move || store.save(&settings, replace_invalid.unwrap_or(false))).await.map_err(|_| worker_error())?
}
