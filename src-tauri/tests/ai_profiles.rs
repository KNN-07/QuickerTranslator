use std::{fs, sync::Arc};

use quickertranslator_lib::{
    ai::{AiProfile, AiProtocol, AuthMode, TokenLimitField, profiles::{
        CREDENTIAL_SERVICE, CredentialState, CredentialStorage, ProfileService, SecretString,
        credential_account, endpoint_identity, normalize_profile, official_presets, preview_profile,
    }},
    settings::{Settings, SettingsStore},
};
use serde_json::json;

fn profile(id: &str) -> AiProfile {
    AiProfile {
        id: id.into(), name: "Local endpoint".into(), protocol: AiProtocol::OpenaiChat,
        base_url: "http://127.0.0.1:8080/proxy/team/v1/".into(), model: "owner/model-id".into(),
        stream: false, max_output_tokens: 4096, auth_mode: AuthMode::ApiKey,
        allow_insecure_http: false, token_limit_field: None,
    }
}
fn open_service(directory: &std::path::Path) -> (Arc<SettingsStore>, ProfileService) {
    let settings = Arc::new(SettingsStore::open(directory).unwrap());
    let service = ProfileService::open(directory, Arc::clone(&settings)).unwrap();
    (settings, service)
}

#[test]
fn canonical_endpoint_identity_and_account_are_prefix_sensitive() {
    let mut first = profile("id:with/slashes 🙂");
    first.base_url = "https://EXAMPLE.COM:443/proxy/team/v1///".into();
    let canonical = endpoint_identity(&first).unwrap();
    assert_eq!(canonical, "https://example.com/proxy/team/v1");
    first.base_url = "https://example.com/proxy/team/v1".into();
    assert_eq!(endpoint_identity(&first).unwrap(), canonical);
    assert_eq!(credential_account(&first.id, &canonical), credential_account(&first.id, &endpoint_identity(&first).unwrap()));
    first.base_url = "https://example.com/proxy/other/v1".into();
    assert_ne!(credential_account(&first.id, &canonical), credential_account(&first.id, &endpoint_identity(&first).unwrap()));
    assert_ne!(credential_account("ab", "c"), credential_account("a", "bc"));
    assert_eq!(credential_account("profile", "https://example.com").len(), "ai-profile-".len() + 64);
}

#[test]
fn profile_validation_presets_and_preview_do_not_guess_models() {
    let presets = official_presets();
    assert_eq!(presets.len(), 4);
    for preset in presets {
        assert!(preset.model.is_empty());
        assert_eq!(preset.max_output_tokens, 4096);
        assert!(normalize_profile(preset.clone(), true).is_err());
        assert!(preview_profile(preset.clone()).is_ok());
        assert_eq!(preset.token_limit_field.is_some(), preset.protocol == AiProtocol::OpenaiChat);
    }
    let normalized = normalize_profile(profile("local"), true).unwrap();
    assert!(!normalized.stream);
    assert_eq!(normalized.model, "owner/model-id");
    assert_eq!(normalized.token_limit_field, Some(TokenLimitField::MaxTokens));
    assert_eq!(preview_profile(normalized).unwrap().request_url, "http://127.0.0.1:8080/proxy/team/v1/chat/completions");
    let mut bad = profile("local"); bad.protocol = AiProtocol::Gemini; bad.token_limit_field = Some(TokenLimitField::Omit);
    assert!(normalize_profile(bad, true).is_err());
    let mut bad = profile("local"); bad.max_output_tokens = 0;
    assert!(normalize_profile(bad, true).is_err());
    let mut bad = profile("local"); bad.model = " \t ".into();
    assert!(normalize_profile(bad, true).is_err());
    for endpoint in ["https://key:secret@example.com/v1", "https://@example.com/v1", "https://example.com/v1?key=secret", "https://example.com/#secret", "file:///secret", "http://example.com/v1"] {
        let mut bad = profile("local"); bad.base_url = endpoint.into();
        assert!(normalize_profile(bad, true).is_err());
    }
    let mut explicit_http = profile("local"); explicit_http.base_url = "http://example.com/custom/v1".into(); explicit_http.allow_insecure_http = true;
    assert!(normalize_profile(explicit_http, true).is_ok());
}

#[test]
fn missing_token_budget_defaults_and_unknown_secret_fields_are_rejected() {
    let mut value = serde_json::to_value(profile("local")).unwrap();
    value.as_object_mut().unwrap().remove("maxOutputTokens");
    let defaulted: AiProfile = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(defaulted.max_output_tokens, 4096);
    for name in ["apiKey", "key", "authorization", "sessionKey", "headers"] {
        let mut invalid = value.clone(); invalid[name] = json!("credential-must-not-enter-profile");
        assert!(serde_json::from_value::<AiProfile>(invalid).is_err());
    }
}

#[test]
fn stale_endpoint_consent_cannot_overwrite_another_windows_current_credential() {
    let directory = tempfile::tempdir().unwrap();
    let (_, service) = open_service(directory.path());
    let mut current = profile("shared-profile");
    service.save(current.clone()).unwrap();
    let old_approval = endpoint_identity(&current).unwrap();
    current.base_url = "http://127.0.0.1:8080/different/v1".into();
    service.save(current.clone()).unwrap();
    let new_approval = endpoint_identity(&current).unwrap();
    service.set_credential(&current.id, &new_approval, SecretString::new("new-endpoint-key".into()), CredentialStorage::Session).unwrap();
    let error = service.set_credential(&current.id, &old_approval, SecretString::new("wrong-endpoint-key".into()), CredentialStorage::Session).unwrap_err();
    assert_eq!(error.code, "ai_preview_stale");
    assert!(!error.message.contains("wrong-endpoint-key"));
    let (_, key) = service.resolve_profile_and_credential(&current.id).unwrap();
    assert_eq!(key.unwrap().expose_secret(), "new-endpoint-key");
}

#[test]
fn explicit_session_key_is_redacted_endpoint_bound_and_never_persisted() {
    let directory = tempfile::tempdir().unwrap();
    let (settings, service) = open_service(directory.path());
    let mut preferences = Settings::default(); preferences.snippets[0] = "Tiếng Việt 日本語".into();
    settings.save(&preferences, false).unwrap();
    let mut current = profile("session");
    service.save(current.clone()).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    let key = SecretString::new("ephemeral-test-secret-\n".into());
    assert_eq!(format!("{key:?}"), "SecretString([REDACTED])");
    // Reject control-character injection without exposing the supplied value in an error.
    let error = service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), key, CredentialStorage::Session).unwrap_err();
    assert!(!error.message.contains("ephemeral-test-secret"));
    let key = "ephemeral-test-secret-only-native-memory";
    let status = service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new(key.into()), CredentialStorage::Session).unwrap();
    assert_eq!(status.status, CredentialState::Session);
    assert!(status.present);
    let (_, resolved) = service.resolve_profile_and_credential(&current.id).unwrap();
    assert_eq!(resolved.unwrap().expose_secret(), key);
    let export = serde_json::to_string(&service.export_profiles().unwrap()).unwrap();
    assert!(!export.contains(key));
    assert!(!serde_json::to_string(&service.list().unwrap()).unwrap().contains(key));
    assert_eq!(settings.load().unwrap().settings.snippets[0], preferences.snippets[0]);
    for entry in fs::read_dir(directory.path()).unwrap() {
        let entry = entry.unwrap();
        assert!(!String::from_utf8_lossy(&fs::read(entry.path()).unwrap()).contains(key));
    }
    let (_, restarted) = open_service(directory.path());
    assert_eq!(restarted.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    drop(restarted);
    let initial_endpoint = current.base_url.clone();
    current.base_url = "http://127.0.0.1:8080/proxy/other/v1".into();
    service.save(current.clone()).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    assert_eq!(service.resolve_profile_and_credential(&current.id).unwrap_err().code, "ai_missing_credentials");
    current.base_url = initial_endpoint;
    service.save(current.clone()).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    // A restart does not silently use either a session key or an older endpoint binding.
    drop(service);
    let (_, reopened) = open_service(directory.path());
    assert_eq!(reopened.credential_status(&current.id).unwrap().status, CredentialState::Missing);
}

#[test]
fn auth_none_drops_session_secret_and_never_reads_the_os_vault() {
    let directory = tempfile::tempdir().unwrap();
    let (_, service) = open_service(directory.path());
    let mut current = profile("noauth"); service.save(current.clone()).unwrap();
    service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new("session-key".into()), CredentialStorage::Session).unwrap();
    current.auth_mode = AuthMode::None;
    service.save(current.clone()).unwrap();
    let (_, credential) = service.resolve_profile_and_credential(&current.id).unwrap();
    assert!(credential.is_none());
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::NotRequired);
    assert!(service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new("another-key".into()), CredentialStorage::Session).is_err());
    current.auth_mode = AuthMode::ApiKey;
    service.save(current.clone()).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    service.delete(&current.id).unwrap();
    assert!(service.list().unwrap().is_empty());
}

#[test]
fn corrupt_nonsecret_index_is_preserved_with_only_explicit_session_available() {
    let directory = tempfile::tempdir().unwrap();
    let (settings, service) = open_service(directory.path());
    let current = profile("index-recovery"); service.save(current.clone()).unwrap(); drop(service);
    let invalid = b"{unsupported credential index preserved";
    let path = directory.path().join("credential-index.json"); fs::write(&path, invalid).unwrap();
    let service = ProfileService::open(directory.path(), settings).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Unavailable);
    assert!(service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new("key".into()), CredentialStorage::Keychain).is_err());
    service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new("explicit-session".into()), CredentialStorage::Session).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Session);
    assert_eq!(fs::read(path).unwrap(), invalid);
    assert!(service.delete(&current.id).is_err());
}

struct OsFixtureCleanup { profile_id: String, endpoints: Vec<String> }
impl Drop for OsFixtureCleanup {
    fn drop(&mut self) {
        for endpoint in &self.endpoints {
            if let Ok(entry) = keyring::Entry::new(CREDENTIAL_SERVICE, &credential_account(&self.profile_id, endpoint)) {
                let _ = entry.delete_credential();
            }
        }
    }
}

/// Run deliberately with an unlocked OS vault: no mocks and no pass-on-unavailable fallback.
/// The unique profile ID isolates this test from the user's configured credentials.
#[test]
#[ignore = "requires an unlocked real OS credential store; run explicitly during native verification"]
fn real_os_keyring_tracks_old_endpoints_and_deletes_all_items() {
    keyring::Entry::store_status().as_ref().expect("real OS credential backend must initialize");
    let directory = tempfile::tempdir().unwrap();
    let (_, service) = open_service(directory.path());
    let mut current = profile(&format!("keyring-integration-{}", uuid::Uuid::new_v4()));
    let old_identity = endpoint_identity(&current).unwrap();
    let mut cleanup = OsFixtureCleanup { profile_id: current.id.clone(), endpoints: vec![old_identity.clone()] };
    service.save(current.clone()).unwrap();
    service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new("old-keyring-fixture-key".into()), CredentialStorage::Keychain).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Saved);
    current.base_url = "http://127.0.0.1:9090/another/v1".into();
    service.save(current.clone()).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    let new_identity = endpoint_identity(&current).unwrap();
    cleanup.endpoints.push(new_identity.clone());
    service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new("new-keyring-fixture-key".into()), CredentialStorage::Keychain).unwrap();
    let new_base = current.base_url.clone();
    current.base_url = old_identity.clone();
    service.save(current.clone()).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    assert_eq!(service.resolve_profile_and_credential(&current.id).unwrap_err().code, "ai_missing_credentials");
    current.base_url = new_base;
    service.save(current.clone()).unwrap();
    assert_eq!(service.credential_status(&current.id).unwrap().status, CredentialState::Missing);
    service.set_credential(&current.id, &endpoint_identity(&current).unwrap(), SecretString::new("new-keyring-fixture-key".into()), CredentialStorage::Keychain).unwrap();
    let first_entry = keyring::Entry::new(CREDENTIAL_SERVICE, &credential_account(&current.id, &old_identity)).unwrap();
    let second_entry = keyring::Entry::new(CREDENTIAL_SERVICE, &credential_account(&current.id, &new_identity)).unwrap();
    assert_eq!(first_entry.get_password().unwrap(), "old-keyring-fixture-key");
    assert_eq!(second_entry.get_password().unwrap(), "new-keyring-fixture-key");
    for entry in fs::read_dir(directory.path()).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("old-keyring-fixture-key") && !text.contains("new-keyring-fixture-key"));
    }
    service.delete(&current.id).unwrap();
    assert!(matches!(first_entry.get_password(), Err(keyring::Error::NoEntry)));
    assert!(matches!(second_entry.get_password(), Err(keyring::Error::NoEntry)));
}
