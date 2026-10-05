use std::{collections::{BTreeMap, HashSet}, fs, path::{Path, PathBuf}, sync::Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{documents::{SaveResult, ViewState}, models::{AppError, AppResult, TranslationOptions, UiLocale}, storage};

#[cfg(feature = "native")]
pub mod native;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme { Light, Dark }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub format: String,
    pub version: u32,
    pub locale: UiLocale,
    pub theme: Theme,
    pub dock_layout: Option<Value>,
    pub editor_options: ViewState,
    pub translation_options: TranslationOptions,
    pub snippets: [String; 9],
    pub keybindings: BTreeMap<String, String>,
    pub profiles: Vec<Value>,
}
impl Default for Settings {
    fn default() -> Self {
        Self { format: "quicktranslator-settings".into(), version: 1, locale: UiLocale::Vi, theme: Theme::Light, dock_layout: None,
            editor_options: ViewState::default(), translation_options: TranslationOptions::default(), snippets: std::array::from_fn(|_| String::new()),
            keybindings: [("wordNext", "k"), ("wordPrevious", "j"), ("lineNext", "m"), ("linePrevious", "i"), ("paragraphNext", "n"), ("paragraphPrevious", "u")].into_iter().map(|(key, value)| (key.into(), value.into())).collect(), profiles: vec![] }
    }
}
impl Settings {
    pub fn validate(&self) -> AppResult<()> {
        if self.format != "quicktranslator-settings" || self.version != 1 {
            return Err(AppError::new("unsupportedSettings", "The settings format is invalid or belongs to another application version. The original file was preserved."));
        }
        self.editor_options.validate()?;
        if let Some(layout) = &self.dock_layout {
            validate_dock_layout(layout)?;
            reject_credentials(layout)?;
        }
        let allowed = ["wordNext", "wordPrevious", "lineNext", "linePrevious", "paragraphNext", "paragraphPrevious"];
        let mut seen_keys = HashSet::new();
        if self.keybindings.len() != allowed.len() { return Err(invalid_settings()); }
        for (action, key) in &self.keybindings {
            let letter = key.as_bytes().first().copied().unwrap_or_default().to_ascii_lowercase();
            if !allowed.contains(&action.as_str()) || key.len() != 1 || !letter.is_ascii_alphabetic()
                || b"osefzycvxa".contains(&letter) || !seen_keys.insert(letter) {
                return Err(AppError::new("invalidKeybindings", "Traversal keybindings must use six distinct Ctrl/Cmd letter keys, excluding O/S/E/F/Z/Y/C/V/X/A reserved for standard document and editor actions."));
            }
        }
        validate_profiles(&self.profiles)
    }
}
fn invalid_settings() -> AppError { AppError::new("invalidSettings", "The settings contain invalid or unsupported values. The existing file was not changed.") }
fn credential_error() -> AppError { AppError::new("secretInSettings", "Credentials must stay in the native credential store or explicit session memory, never in settings JSON.") }

pub fn validate_dock_layout(layout: &Value) -> AppResult<()> {
    let grid = layout.get("grid").and_then(Value::as_object).ok_or_else(invalid_settings)?;
    let panels = layout.get("panels").and_then(Value::as_object).ok_or_else(invalid_settings)?;
    if !grid.get("root").is_some_and(Value::is_object)
        || !matches!(grid.get("orientation").and_then(Value::as_str), Some("HORIZONTAL" | "VERTICAL"))
        || ["width", "height"].iter().any(|key| !grid.get(*key).and_then(Value::as_f64).is_some_and(|value| value.is_finite() && value >= 0.0)) {
        return Err(invalid_settings());
    }
    if let Some(popouts) = layout.get("popoutGroups") {
        if !popouts.as_array().is_some_and(Vec::is_empty) { return Err(invalid_settings()); }
    }
    let pane_ids = ["source", "readings", "phrases", "singleMeaning", "meanings", "target", "ai"];
    for (id, panel) in panels {
        if !pane_ids.contains(&id.as_str()) || !panel.is_object()
            || panel.get("id").and_then(Value::as_str) != Some(id.as_str())
            || panel.get("contentComponent").and_then(Value::as_str) != Some("workspace")
            || panel.get("params").and_then(|params| params.get("paneId")).and_then(Value::as_str) != Some(id.as_str()) {
            return Err(invalid_settings());
        }
    }
    Ok(())
}

fn reject_credentials(value: &Value) -> AppResult<()> {
    match value {
        Value::Object(object) => for (key, value) in object {
            let normalized: String = key.chars().filter(char::is_ascii_alphanumeric).flat_map(char::to_lowercase).collect();
            if matches!(normalized.as_str(), "apikey" | "key" | "password" | "authorization" | "credential" | "credentials" | "secret" | "token" | "accesstoken" | "refreshtoken" | "headers" | "headersecret" | "sessionkey" | "bearer" | "clientsecret") {
                return Err(credential_error());
            }
            reject_credentials(value)?;
        },
        Value::Array(values) => for value in values { reject_credentials(value)?; },
        _ => {},
    }
    Ok(())
}

pub fn validate_profiles(profiles: &[Value]) -> AppResult<()> {
    let allowed = ["id", "name", "protocol", "baseUrl", "model", "stream", "maxOutputTokens", "authMode", "allowInsecureHttp", "tokenLimitField"];
    let mut ids = HashSet::new();
    for profile in profiles {
        reject_credentials(profile)?;
        let object = profile.as_object().ok_or_else(invalid_settings)?;
        if object.keys().any(|key| !allowed.contains(&key.as_str())) { return Err(invalid_settings()); }
        for field in ["id", "name", "protocol", "baseUrl", "model", "authMode"] {
            if !object.get(field).is_some_and(Value::is_string) { return Err(invalid_settings()); }
        }
        let id = object["id"].as_str().unwrap();
        if id.is_empty() || !ids.insert(id) { return Err(invalid_settings()); }
        let protocol = object["protocol"].as_str().unwrap();
        if !matches!(protocol, "openai-responses" | "openai-chat" | "gemini" | "anthropic") || !matches!(object["authMode"].as_str(), Some("apiKey" | "none")) { return Err(invalid_settings()); }
        if !object.get("stream").is_some_and(Value::is_boolean) || !object.get("allowInsecureHttp").is_some_and(Value::is_boolean)
            || !object.get("maxOutputTokens").and_then(Value::as_u64).is_some_and(|value| value > 0 && value <= u32::MAX as u64) { return Err(invalid_settings()); }
        let endpoint = reqwest::Url::parse(object["baseUrl"].as_str().unwrap()).map_err(|_| invalid_settings())?;
        if !matches!(endpoint.scheme(), "https" | "http") || endpoint.host_str().is_none() || !endpoint.username().is_empty() || endpoint.password().is_some() || endpoint.query().is_some() || endpoint.fragment().is_some() { return Err(invalid_settings()); }
        if let Some(field) = object.get("tokenLimitField") {
            if protocol != "openai-chat" || !matches!(field.as_str(), Some("max_tokens" | "max_completion_tokens" | "omit")) { return Err(invalid_settings()); }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsLoad { pub settings: Settings, pub preserved_invalid_file: bool, pub warning: Option<String> }

pub struct SettingsStore { path: PathBuf, gate: Mutex<()> }
impl SettingsStore {
    pub fn open(app_data: &Path) -> AppResult<Self> {
        fs::create_dir_all(app_data).map_err(|error| AppError::io("settingsUnavailable", "Application settings storage could not be opened", &error))?;
        Ok(Self { path: app_data.join("settings.json"), gate: Mutex::new(()) })
    }
    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, ()>> { self.gate.lock().map_err(|_| AppError::new("settingsUnavailable", "Settings storage could not be accessed.")) }
    fn load_unlocked(&self) -> AppResult<SettingsLoad> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(SettingsLoad { settings: Settings::default(), preserved_invalid_file: false, warning: None }),
            Err(error) => return Err(AppError::io("settingsUnavailable", "The settings file could not be read; it was not replaced", &error)),
        };
        let parsed = serde_json::from_slice::<Settings>(&bytes).ok().filter(|settings| settings.validate().is_ok());
        match parsed {
            Some(settings) => Ok(SettingsLoad { settings, preserved_invalid_file: false, warning: None }),
            None => Ok(SettingsLoad { settings: Settings::default(), preserved_invalid_file: true,
                warning: Some("The settings file is invalid or unsupported and was preserved. Safe defaults are active; explicitly choose to replace the invalid file before saving settings.".into()) }),
        }
    }
    pub fn load(&self) -> AppResult<SettingsLoad> { let _guard = self.lock()?; self.load_unlocked() }
    fn save_unlocked(&self, settings: &Settings, replace_invalid: bool) -> AppResult<SaveResult> {
        settings.validate()?;
        if self.load_unlocked()?.preserved_invalid_file && !replace_invalid {
            return Err(AppError::new("settingsRecoveryRequired", "The invalid settings file was preserved. Explicitly confirm replacing it, or copy it aside for manual recovery before saving."));
        }
        // The frontend may detect deeper Dockview corruption that the native
        // structural validator cannot. Explicit recovery always retains bytes.
        if replace_invalid && self.path.exists() {
            let bytes = fs::read(&self.path).map_err(|error| AppError::io("settingsBackupFailed", "The previous settings could not be preserved; replacement was cancelled", &error))?;
            let backup = self.path.with_file_name(format!("settings.invalid-{}.json", uuid::Uuid::new_v4()));
            storage::atomic_replace(&backup, &bytes)?;
        }
        let outcome = storage::atomic_replace_json(&self.path, settings)?;
        Ok(SaveResult { path: self.path.to_string_lossy().into_owned(), directory_sync_confirmed: outcome.directory_sync_confirmed })
    }
    /// Preference writes cannot overwrite profiles concurrently committed by another window.
    pub fn save(&self, settings: &Settings, replace_invalid: bool) -> AppResult<SaveResult> {
        let _guard = self.lock()?;
        let loaded = self.load_unlocked()?;
        let mut preferences = settings.clone();
        if !loaded.preserved_invalid_file {
            preferences.profiles = loaded.settings.profiles;
        }
        self.save_unlocked(&preferences, replace_invalid)
    }
    /// Stage-six profile writes preserve all interface preferences and never read or write keys.
    pub fn profiles(&self) -> AppResult<Vec<Value>> {
        let _guard = self.lock()?;
        let loaded = self.load_unlocked()?;
        if loaded.preserved_invalid_file { return Err(AppError::new("settingsRecoveryRequired", "Recover or explicitly replace invalid settings before changing provider profiles.")); }
        Ok(loaded.settings.profiles)
    }
    pub fn replace_profiles(&self, profiles: Vec<Value>) -> AppResult<SaveResult> {
        validate_profiles(&profiles)?;
        let _guard = self.lock()?;
        let mut loaded = self.load_unlocked()?;
        if loaded.preserved_invalid_file { return Err(AppError::new("settingsRecoveryRequired", "Recover or explicitly replace invalid settings before changing provider profiles.")); }
        loaded.settings.profiles = profiles;
        self.save_unlocked(&loaded.settings, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn defaults_do_not_overwrite_invalid_or_future_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::open(directory.path()).unwrap();
        let invalid = b"{broken private settings";
        fs::write(&store.path, invalid).unwrap();
        assert!(store.load().unwrap().preserved_invalid_file);
        assert!(store.save(&Settings::default(), false).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), invalid);
        let mut future = Settings::default(); future.version = 2;
        let bytes = serde_json::to_vec(&future).unwrap(); fs::write(&store.path, &bytes).unwrap();
        assert!(store.load().unwrap().preserved_invalid_file);
        assert!(store.save(&Settings::default(), false).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
    }
    #[test]
    fn preferences_roundtrip_without_keys_and_profiles_preserve_them() {
        let directory = tempfile::tempdir().unwrap(); let store = SettingsStore::open(directory.path()).unwrap();
        let mut settings = Settings::default(); settings.locale = UiLocale::En; settings.snippets[0] = "Tiếng Việt 日本語 🙂".into();
        store.save(&settings, false).unwrap();
        let profile = json!({"id":"local","name":"Local","protocol":"openai-chat","baseUrl":"http://127.0.0.1:8080/proxy/v1","model":"model","stream":true,"maxOutputTokens":4096,"authMode":"none","allowInsecureHttp":false,"tokenLimitField":"max_tokens"});
        store.replace_profiles(vec![profile.clone()]).unwrap();
        assert_eq!(store.load().unwrap().settings.snippets[0], settings.snippets[0]);
        assert_eq!(store.profiles().unwrap(), vec![profile.clone()]);
        // A different window saved this stale preference snapshot before the profile commit.
        settings.locale = UiLocale::Vi;
        store.save(&settings, false).unwrap();
        assert_eq!(store.profiles().unwrap(), vec![profile.clone()]);
        assert_eq!(store.load().unwrap().settings.locale, UiLocale::Vi);
        for secret in ["apiKey", "password", "authorization", "headers", "sessionKey"] {
            let mut bad = profile.clone(); bad[secret] = json!("never persist");
            assert!(store.replace_profiles(vec![bad]).is_err());
            assert_eq!(store.profiles().unwrap(), vec![profile.clone()]);
        }
    }
    #[test]
    fn explicit_settings_replacement_retains_invalid_bytes_for_recovery() {
        let directory = tempfile::tempdir().unwrap(); let store = SettingsStore::open(directory.path()).unwrap();
        let invalid = b"{broken old settings";
        fs::write(&store.path, invalid).unwrap();
        store.save(&Settings::default(), true).unwrap();
        assert!(!store.load().unwrap().preserved_invalid_file);
        let backups: Vec<_> = fs::read_dir(directory.path()).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("settings.invalid-")).collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), invalid);
    }
    #[test]
    fn traversal_configuration_retains_legacy_n_and_never_overrides_standard_shortcuts() {
        Settings::default().validate().unwrap();
        for key in ["O", "s", "E", "f", "Z", "y", "C", "v", "X", "a"] {
            let mut settings = Settings::default();
            settings.keybindings.insert("wordNext".into(), key.into());
            assert_eq!(settings.validate().unwrap_err().code, "invalidKeybindings");
        }
    }
    #[test]
    fn unsupported_dock_components_and_popouts_are_rejected_before_persistence() {
        let valid = json!({"grid":{"root":{"type":"branch","data":[]},"width":1280,"height":800,"orientation":"HORIZONTAL"},"panels":{"source":{"id":"source","contentComponent":"workspace","params":{"paneId":"source"}}}});
        validate_dock_layout(&valid).unwrap();
        let mut invalid = valid.clone(); invalid["panels"]["source"]["params"]["paneId"] = json!("target");
        assert!(validate_dock_layout(&invalid).is_err());
        let mut invalid = valid.clone(); invalid["panels"]["source"]["contentComponent"] = json!("external");
        assert!(validate_dock_layout(&invalid).is_err());
        let mut invalid = valid.clone(); invalid["panels"]["unknown"] = json!({"id":"unknown","contentComponent":"workspace","params":{"paneId":"unknown"}});
        assert!(validate_dock_layout(&invalid).is_err());
        for popouts in [json!([{"url":"https://invalid.test"}]), json!({}), Value::Null] {
            let mut invalid = valid.clone(); invalid["popoutGroups"] = popouts;
            assert!(validate_dock_layout(&invalid).is_err());
        }
    }
    #[test]
    fn explicit_frontend_recovery_preserves_even_structurally_valid_previous_settings() {
        let directory = tempfile::tempdir().unwrap(); let store = SettingsStore::open(directory.path()).unwrap();
        store.save(&Settings::default(), false).unwrap();
        let previous = fs::read(&store.path).unwrap();
        let mut replacement = Settings::default(); replacement.snippets[0] = "restored".into();
        store.save(&replacement, true).unwrap();
        let backup = fs::read_dir(directory.path()).unwrap().map(|entry| entry.unwrap().path()).find(|path| path.file_name().unwrap().to_string_lossy().starts_with("settings.invalid-")).unwrap();
        assert_eq!(fs::read(backup).unwrap(), previous);
        assert_eq!(store.load().unwrap().settings.snippets[0], "restored");
    }
}
