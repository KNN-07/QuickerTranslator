pub mod models;
pub mod state;
pub mod storage;
pub mod dictionaries;
pub mod documents;
pub mod settings;
pub mod ai;

pub mod engine;

#[cfg(feature = "native")]
pub mod commands;

#[cfg(feature = "native")]
pub fn run() {
    use std::sync::Arc;

    use tauri::Manager;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("bundled-navigation")
                .on_navigation(|_, url| {
                    let bundled = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost") && url.port().is_none());
                    let development = cfg!(debug_assertions)
                        && url.scheme() == "http"
                        && url.host_str() == Some("127.0.0.1")
                        && url.port() == Some(1420);
                    bundled || development
                })
                .build(),
        )
        .setup(|app| {
            let state = Arc::new(state::AppState::default());
            let app_data = app.path().app_data_dir()?;
            let worker_state = Arc::clone(&state);
            let (store, snapshot, documents, settings, profiles, jobs) = tauri::async_runtime::block_on(tauri::async_runtime::spawn_blocking(move || {
                worker_state.initialize_tokenizer()?;
                let store = Arc::new(dictionaries::DictionaryStore::open(&app_data)?);
                let snapshot = store.initial_snapshot()?;
                let documents = Arc::new(documents::DocumentService::open(&app_data)?);
                let settings = Arc::new(settings::SettingsStore::open(&app_data)?);
                let profiles = Arc::new(ai::profiles::ProfileService::open(&app_data, Arc::clone(&settings))?);
                let jobs = Arc::new(ai::runtime::JobRuntime::new()?);
                Ok::<_, models::AppError>((store, snapshot, documents, settings, profiles, jobs))
            }))??;
            state.replace_dictionaries(snapshot)?;
            state.register_window("main")?;
            app.manage(state);
            app.manage(store);
            app.manage(documents);
            app.manage(settings);
            app.manage(profiles);
            app.manage(jobs);
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(state) = window.try_state::<Arc<state::AppState>>() {
                    let _ = state.remove_window(window.label());
                }
                if let Some(jobs) = window.try_state::<Arc<ai::runtime::JobRuntime>>() {
                    jobs.remove_window(window.label());
                }
                if let Some(documents) = window.try_state::<Arc<documents::DocumentService>>() {
                    if let Err(error) = documents.release_window(window.label()) {
                        eprintln!("Recovery ownership cleanup failed ({}); restart restores recovery access.", error.code);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::foundation_health,
            commands::new_document_window,
            commands::translate_offline,
            commands::observe_document_source,
            commands::observe_document_target,
            commands::cancel_offline_translation,
            commands::lookup_dictionary_entries,
            dictionaries::dictionary_catalog,
            dictionaries::dictionary_storage_status,
            dictionaries::preview_dictionary_config,
            dictionaries::preview_dictionary_import,
            dictionaries::commit_dictionary_import,
            dictionaries::reload_dictionary,
            dictionaries::search_dictionary_entries,
            dictionaries::save_dictionary_metadata,
            dictionaries::save_dictionary_entry,
            dictionaries::delete_dictionary_entry,
            dictionaries::dictionary_entry_history,
            dictionaries::export_dictionary,
            dictionaries::list_shortcuts,
            dictionaries::preview_shortcuts_import,
            dictionaries::commit_shortcuts_import,
            dictionaries::save_shortcut,
            dictionaries::delete_shortcut,
            dictionaries::export_shortcuts,
            dictionaries::repair_dictionary_database,
            documents::native::preview_document_import,
            documents::native::open_document,
            documents::native::save_document,
            documents::native::export_document,
            documents::native::document_siblings,
            documents::native::set_document_title,
            documents::native::recovery_list,
            documents::native::recovery_read,
            documents::native::recovery_release,
            documents::native::recovery_write,
            documents::native::recovery_discard,
            settings::native::load_settings,
            settings::native::save_settings,
            ai::profiles::list_ai_profiles,
            ai::profiles::save_ai_profile,
            ai::profiles::delete_ai_profile,
            ai::profiles::set_ai_credential,
            ai::profiles::delete_ai_credential,
            ai::profiles::ai_credential_status,
            ai::profiles::ai_credential_store_status,
            ai::profiles::preview_ai_profile,
            ai::profiles::ai_profile_presets,
            ai::profiles::test_ai_profile,
            ai::runtime::preview_ai_translation,
            ai::runtime::start_ai_translation,
            ai::runtime::cancel_ai_translation,
        ])
        .run(tauri::generate_context!())
        .expect("could not run QuickTranslator native application");
}
