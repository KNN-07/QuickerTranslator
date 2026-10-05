fn main() {
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=resources/dictionaries");

    #[cfg(feature = "bundled-dictionaries")]
    require_bundled_dictionaries();

    #[cfg(feature = "native")]
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new()
                .commands(&[
                    "foundation_health", "new_document_window",
                    "translate_offline", "observe_document_source",
                    "observe_document_target", "cancel_offline_translation",
                    "lookup_dictionary_entries",
                    "dictionary_catalog", "dictionary_storage_status",
                    "preview_dictionary_config", "preview_dictionary_import",
                    "commit_dictionary_import", "reload_dictionary",
                    "search_dictionary_entries", "save_dictionary_metadata",
                    "save_dictionary_entry", "delete_dictionary_entry",
                    "dictionary_entry_history", "export_dictionary",
                    "list_shortcuts", "preview_shortcuts_import",
                    "commit_shortcuts_import", "save_shortcut", "delete_shortcut",
                    "export_shortcuts", "repair_dictionary_database",
                    "preview_document_import", "open_document", "save_document",
                    "export_document", "document_siblings", "set_document_title",
                    "recovery_list", "recovery_read", "recovery_release", "recovery_write", "recovery_discard",
                    "load_settings", "save_settings",
                    "list_ai_profiles", "save_ai_profile", "delete_ai_profile",
                    "set_ai_credential", "delete_ai_credential", "ai_credential_status",
                    "ai_credential_store_status", "preview_ai_profile", "ai_profile_presets",
                    "test_ai_profile", "preview_ai_translation", "start_ai_translation",
                    "cancel_ai_translation",
                ]),
        ),
    )
    .expect("could not prepare native QuickTranslator application");
}

#[cfg(feature = "bundled-dictionaries")]
fn require_bundled_dictionaries() {
    for file in [
        "zh-vi.jsonl",
        "ja-vi.jsonl",
        "han-viet.jsonl",
        "manifest.json",
        "ATTRIBUTION.txt",
        "licenses/IPADIC.txt",
        "licenses/IPADIC-original.COPYING",
        "licenses/Lindera-MIT.txt",
        "licenses/Unicode-V3.txt",
        "licenses/CC-BY-SA-4.0.txt",
    ] {
        let path = std::path::Path::new("resources/dictionaries").join(file);
        let usable = path.metadata().is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
        assert!(
            usable,
            "Missing or empty bundled dictionary resource {}. Run npm run data:prepare (local resource copies may be supplied with --wiktextract PATH --unihan PATH); never substitute demonstration data.",
            path.display(),
        );
    }
    use std::io::Read;
    use sha2::{Digest, Sha256};

    let root = std::path::Path::new("resources/dictionaries");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("manifest.json")).expect("Cannot read dictionary manifest"),
    ).expect("Invalid dictionary manifest; run npm run data:prepare");
    assert_eq!(manifest["format"], "quicktranslator-dictionaries", "Invalid dictionary resource format");
    assert_eq!(manifest["version"], 1, "Unsupported dictionary manifest version");
    for section in ["resources", "notices"] {
        let records = manifest[section].as_array().expect("Missing manifest records");
        assert!(!records.is_empty(), "Empty dictionary manifest section");
        for record in records {
            let file = record["file"].as_str().expect("Missing resource filename");
            let relative = std::path::Path::new(file);
            assert!(relative.components().all(|part| matches!(part, std::path::Component::Normal(_))),
                "Invalid resource path in dictionary manifest");
            let mut input = std::fs::File::open(root.join(relative)).expect("Missing locked dictionary resource");
            let mut digest = Sha256::new();
            let mut buffer = [0_u8; 65536];
            let mut bytes = 0_u64;
            loop {
                let count = input.read(&mut buffer).expect("Cannot read locked dictionary resource");
                if count == 0 { break; }
                bytes += count as u64;
                digest.update(&buffer[..count]);
            }
            assert_eq!(record["bytes"].as_u64(), Some(bytes), "Resource size changed: {file}; explicit data:prepare --refresh is required");
            let expected = record["sha256"].as_str().expect("Missing locked resource digest");
            assert!(expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Invalid locked resource digest");
            let actual = digest.finalize();
            assert!(actual.iter().enumerate().all(|(index, byte)| {
                u8::from_str_radix(&expected[index * 2..index * 2 + 2], 16) == Ok(*byte)
            }), "Resource digest changed: {file}; restore locked resources or explicitly refresh");
        }
    }
}
