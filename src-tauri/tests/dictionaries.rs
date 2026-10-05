#![cfg(feature = "bundled-dictionaries")]

use std::{collections::HashMap, fs, path::Path};

use quickertranslator_lib::{
    dictionaries::{DictionaryStore, encoding::decode_bytes, import::{compile_rule, parse_dictionary, preview_config}, *},
    models::{DictionaryKind as Kind, DictionaryLayer, SourceLanguage as Language, TranslationOptions, TranslationRequest},
    state::AppState,
};

fn service(directory: &Path) -> (DictionaryStore, AppState) {
    let store = DictionaryStore::open(directory).unwrap();
    assert!(store.storage_status().unwrap().ready);
    let state = AppState::default();
    state.replace_dictionaries(store.initial_snapshot().unwrap()).unwrap();
    (store, state)
}

fn entry(headword: &str, meanings: &[&str]) -> EntryRecord {
    EntryRecord { headword: headword.to_owned(), meanings: meanings.iter().map(|value| (*value).to_owned()).collect(), reading: None, pos: None, aliases: Vec::new(), source_urls: Vec::new(), payload: None }
}

fn import_request(path: &Path, kind: Kind, id: Option<String>) -> ImportRequest {
    ImportRequest { path: path.to_string_lossy().into_owned(), name: "Independent test dictionary".to_owned(), language: Language::Zh, kind, dictionary_id: id, encoding: None, format: None }
}

fn commit(store: &DictionaryStore, state: &AppState, preview: &ImportPreview) {
    store.commit_import(CommitImportRequest { preview_ids: vec![preview.preview_id.clone()], rule_algorithm: None }, state).unwrap();
}

#[test]
fn legacy_preview_reports_exact_line_errors_and_first_duplicate_without_folding_keys() {
    let parsed = parse_dictionary(include_str!("fixtures/dictionary-legacy.txt"), Kind::VietPhrase, ImportFormat::Legacy);
    assert_eq!(parsed.entries.len(), 3);
    assert_eq!(parsed.duplicates, 1);
    assert_eq!(parsed.entries["你好"].meanings, ["xin chào", "chào bạn", "lời chào"]);
    assert!(parsed.entries.contains_key("AbC"));
    assert!(parsed.entries.contains_key("abc"));
    let diagnostics: Vec<_> = parsed.issues.iter().map(|issue| (issue.line, issue.code.as_str())).collect();
    assert_eq!(diagnostics, [(2, "duplicate"), (5, "malformedLine"), (6, "emptyKey"), (7, "malformedLine"), (8, "emptyMeaning")]);
}

#[test]
fn auxiliary_records_keep_raw_payload_and_cedict_indexes_both_spellings() {
    let parsed = parse_dictionary("词=nghĩa một/nghĩa hai|ghi chú", Kind::LacViet, ImportFormat::Legacy);
    assert_eq!(parsed.entries["词"].meanings, ["nghĩa một/nghĩa hai|ghi chú"]);
    assert_eq!(parsed.entries["词"].payload.as_deref(), Some("nghĩa một/nghĩa hai|ghi chú"));
    let parsed = parse_dictionary(include_str!("fixtures/dictionary-cedict.txt"), Kind::Cedict, ImportFormat::Cedict);
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries["學校"].aliases, ["学校"]);
    assert_eq!(parsed.entries["學校"].reading.as_deref(), Some("xue2 xiao4"));
    assert_eq!(parsed.issues[0].line, 4);
    let symbols = parse_dictionary("符号 符号 [fu2 hao4] /equals = symbol/", Kind::Cedict, ImportFormat::Cedict);
    assert_eq!(symbols.entries["符号"].meanings, ["equals = symbol"]);
    let legacy_export = parse_dictionary("學校=trường học/học đường", Kind::Cedict, ImportFormat::Legacy);
    assert_eq!(legacy_export.entries["學校"].meanings, ["trường học/học đường"]);
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let path = directory.path().join("cedict.txt");
    fs::write(&path, include_str!("fixtures/dictionary-cedict.txt")).unwrap();
    let preview = store.preview_import(import_request(&path, Kind::Cedict, None)).unwrap();
    commit(&store, &state, &preview);
    let snapshot = state.dictionaries().unwrap();
    let traditional = snapshot.lookup(Language::Zh, Kind::Cedict, "學校").unwrap();
    let simplified = snapshot.lookup(Language::Zh, Kind::Cedict, "学校").unwrap();
    assert_eq!(simplified.meanings, traditional.meanings);
    assert_eq!(traditional.meanings, ["trường học/học đường"]);
}

#[test]
fn ignored_import_retains_nonempty_lines_and_has_no_fabricated_gloss() {
    let parsed = parse_dictionary(include_str!("fixtures/dictionary-ignored.txt"), Kind::Ignored, ImportFormat::Ignored);
    assert_eq!(parsed.entries.len(), 3);
    assert_eq!(parsed.duplicates, 1);
    assert!(parsed.entries["广告"].meanings.is_empty());
    assert!(parsed.entries.contains_key("x=y"));
}

#[test]
fn rule_preview_reports_invalid_patterns_and_named_capture_survives_other_groups() {
    let parsed = parse_dictionary(include_str!("fixtures/dictionary-rules.txt"), Kind::Rules, ImportFormat::Legacy);
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.issues.iter().map(|issue| issue.line).collect::<Vec<_>>(), [3, 4, 5]);
    assert!(parsed.issues.iter().all(|issue| issue.code == "invalidRule"));
    let rule = compile_rule("(foo){0}bar").unwrap();
    let captured = rule.captures("foo日本bar").unwrap();
    assert_eq!(captured.name("qt_capture").unwrap().as_str(), "日本");
    assert!(rule.captures("foobar").is_none());
    assert!(compile_rule("{0}{0}").is_err());
    assert!(compile_rule("(?=a){0}").is_err());
}

#[test]
fn encoding_boms_are_authoritative_and_invalid_utf16_utf8_never_replace() {
    let source = "你好=Tiếng Việt 🙂\r\n";
    let mut little = vec![0xff, 0xfe];
    for unit in source.encode_utf16() { little.extend_from_slice(&unit.to_le_bytes()); }
    let decoded = decode_bytes(&little, Some("GBK")).unwrap();
    assert_eq!(decoded.text, source);
    assert_eq!(decoded.encoding, "UTF-16LE");
    assert!(!decoded.detected);
    let mut big = vec![0xfe, 0xff];
    for unit in source.encode_utf16() { big.extend_from_slice(&unit.to_be_bytes()); }
    assert_eq!(decode_bytes(&big, None).unwrap().text, source);
    assert_eq!(decode_bytes(&little[2..], Some("UTF-16LE")).unwrap().text, source);
    assert_eq!(decode_bytes(&big[2..], Some("UTF-16BE")).unwrap().text, source);
    assert_eq!(decode_bytes(&[0xef, 0xbb, 0xbf, b'a'], Some("Big5")).unwrap().text, "a");
    assert_eq!(decode_bytes(&[0xff, 0xfe, 0x00, 0xd8], None).unwrap_err().code, "invalidEncoding");
    assert_eq!(decode_bytes(&[0xfe, 0xff, 0x00], None).unwrap_err().code, "invalidEncoding");
    assert_eq!(decode_bytes(&[0xef, 0xbb, 0xbf, 0xff], None).unwrap_err().code, "invalidEncoding");
    assert_eq!(decode_bytes(&[0x82], Some("Shift-JIS")).unwrap_err().code, "invalidEncoding");
    assert_eq!(decode_bytes(b"abc", Some("unknown-encoding")).unwrap_err().code, "encodingUnavailable");
    assert_eq!(decode_bytes(&[0xff, 0xfe, 0x00, 0x00, b'a', 0, 0, 0], None).unwrap_err().code, "encodingUnavailable");
}

#[test]
fn supported_legacy_encoding_overrides_round_trip_without_loss() {
    for (label, text) in [("GB18030", "你好=中文"), ("GBK", "你好=中文"), ("Big5", "學校=學校"), ("Shift-JIS", "学校=日本語"), ("EUC-JP", "学校=日本語"), ("Windows-1258", "abc=Viê\u{0323}t")] {
        let encoding = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap();
        let (bytes, _, errors) = encoding.encode(text);
        assert!(!errors, "fixture must be representable in {label}");
        assert_eq!(decode_bytes(&bytes, Some(label)).unwrap().text, text, "{label}");
    }
}

#[test]
fn config_resolves_relative_backslashes_reports_foreign_drives_and_remaps_missing() {
    let directory = tempfile::tempdir().unwrap();
    let subdirectory = directory.path().join("local");
    fs::create_dir(&subdirectory).unwrap();
    let names = subdirectory.join("Names.txt");
    fs::write(&names, "张三=Trương Tam").unwrap();
    let remapped = directory.path().join("replacement.txt");
    fs::write(&remapped, "你好=xin chào").unwrap();
    let config = directory.path().join("Dictionaries.config");
    fs::write(&config, "Names=local\\Names.txt\nNamesPhu=missing.txt\nVietPhrase=C:\\legacy\\VietPhrase.txt\nThuatToanNhan=3\nUnknown=abc\n").unwrap();
    let mut request = ConfigRequest { path: config.to_string_lossy().into_owned(), encoding: None, remappings: HashMap::new() };
    let preview = preview_config(&request).unwrap();
    assert_eq!(preview.rule_algorithm, 3);
    let (store, state) = service(directory.path());
    let resolved = Path::new(preview.dictionaries[0].resolved_path.as_deref().unwrap());
    let import = store.preview_import(import_request(resolved, Kind::PrimaryNames, None)).unwrap();
    commit(&store, &state, &import);
    assert_eq!(state.dictionaries().unwrap().lookup(Language::Zh, Kind::PrimaryNames, "张三").unwrap().meanings, ["Trương Tam"]);
    assert_eq!(preview.dictionaries[1].problem.as_deref(), Some("missingPath"));
    assert!(preview.dictionaries[2].resolved_path.is_none());
    assert_eq!(preview.issues[0].code, "unknownConfigKey");
    request.remappings.insert("NamesPhu".to_owned(), remapped.to_string_lossy().into_owned());
    request.remappings.insert("VietPhrase".to_owned(), remapped.to_string_lossy().into_owned());
    let remapped_preview = preview_config(&request).unwrap();
    assert!(remapped_preview.dictionaries.iter().all(|dictionary| dictionary.resolved_path.is_some()));
}

#[test]
fn imports_publish_real_layer_provenance_and_reload_preserves_edits_tombstones_and_history() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let path = directory.path().join("phrases.txt");
    fs::write(&path, "你好=xin chào/chào bạn\n张三=Trương Tam\n").unwrap();
    let preview = store.preview_import(import_request(&path, Kind::VietPhrase, None)).unwrap();
    assert_eq!(state.dictionaries().unwrap().revision(), 1);
    commit(&store, &state, &preview);
    let snapshot = state.dictionaries().unwrap();
    let imported = snapshot.lookup(Language::Zh, Kind::VietPhrase, "你好").unwrap();
    assert_eq!(imported.meanings, ["xin chào", "chào bạn"]);
    assert_eq!(imported.provenance[0].layer, DictionaryLayer::Imported);
    assert_eq!(imported.provenance[0].dictionary_id, preview.dictionary_id);
    store.save_entry(EntryMutation { dictionary_id: preview.dictionary_id.clone(), entry: entry("你好", &["chào riêng", "xin chào"]) }, &state).unwrap();
    store.delete_entry(EntryKey { dictionary_id: preview.dictionary_id.clone(), headword: "张三".to_owned() }, &state).unwrap();
    fs::write(&path, "你好=giá trị mới\n张三=không được sống lại\n再见=tạm biệt\n").unwrap();
    let reload = store.reload_preview(&preview.dictionary_id).unwrap();
    // Changing the file after preview cannot substitute unpreviewed content.
    fs::write(&path, "坏=không có trong xem trước\n").unwrap();
    commit(&store, &state, &reload);
    let snapshot = state.dictionaries().unwrap();
    assert_eq!(snapshot.lookup(Language::Zh, Kind::VietPhrase, "你好").unwrap().meanings, ["chào riêng", "xin chào"]);
    assert_eq!(snapshot.lookup(Language::Zh, Kind::VietPhrase, "你好").unwrap().provenance[0].layer, DictionaryLayer::Edited);
    assert!(snapshot.lookup(Language::Zh, Kind::VietPhrase, "张三").is_none());
    assert_eq!(snapshot.lookup(Language::Zh, Kind::VietPhrase, "再见").unwrap().meanings, ["tạm biệt"]);
    assert!(snapshot.lookup(Language::Zh, Kind::VietPhrase, "坏").is_none_or(|entry| entry.provenance[0].dictionary_id != preview.dictionary_id));
    let history = store.entry_history(EntryKey { dictionary_id: preview.dictionary_id.clone(), headword: "你好".to_owned() }).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].before.as_ref().unwrap().meanings, ["xin chào", "chào bạn"]);
    assert_eq!(history[0].after.as_ref().unwrap().meanings, ["chào riêng", "xin chào"]);
    let deleted = store.entry_history(EntryKey { dictionary_id: preview.dictionary_id.clone(), headword: "张三".to_owned() }).unwrap();
    assert_eq!(deleted[0].action, "delete");
    assert!(deleted[0].after.is_none());
    let catalog = store.catalog().unwrap();
    let metadata = catalog.dictionaries.iter().find(|metadata| metadata.id == preview.dictionary_id).unwrap();
    assert_eq!(metadata.entry_count, 2);
    assert_eq!(metadata.tombstone_count, 1);
    drop(store);
    let (reopened, reopened_state) = service(directory.path());
    assert!(reopened_state.dictionaries().unwrap().lookup(Language::Zh, Kind::VietPhrase, "张三").is_none());
    assert_eq!(reopened.catalog().unwrap().revision, catalog.revision);
}

#[test]
fn failed_multi_dictionary_import_rolls_back_every_selected_import_and_revision() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let path = directory.path().join("names.txt");
    fs::write(&path, "测试名字=ten thu nghiem").unwrap();
    let first = store.preview_import(import_request(&path, Kind::PrimaryNames, None)).unwrap();
    let second = store.preview_import(import_request(&path, Kind::SecondaryNames, None)).unwrap();
    store.save_metadata(MetadataMutation { id: Some(second.dictionary_id.clone()), name: "changed kind before confirmation".to_owned(), language: Language::Zh, kind: Kind::VietPhrase }, &state).unwrap();
    let before = store.catalog().unwrap().revision;
    let error = store.commit_import(CommitImportRequest { preview_ids: vec![first.preview_id.clone(), second.preview_id.clone()], rule_algorithm: Some(3) }, &state).unwrap_err();
    assert_eq!(error.code, "invalidImportPreview");
    assert_eq!(store.catalog().unwrap().revision, before);
    assert_eq!(store.catalog().unwrap().rule_algorithm, 1);
    assert!(store.catalog().unwrap().dictionaries.iter().all(|metadata| metadata.id != first.dictionary_id));
    assert!(state.dictionaries().unwrap().lookup(Language::Zh, Kind::PrimaryNames, "测试名字").is_none());
}

#[test]
fn bundled_reading_deletion_is_persistent_and_explicit_save_restores_it() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    assert!(state.dictionaries().unwrap().lookup(Language::Zh, Kind::HanViet, "你").is_some());
    let key = EntryKey { dictionary_id: "bundled:han".to_owned(), headword: "你".to_owned() };
    store.delete_entry(key.clone(), &state).unwrap();
    assert!(state.dictionaries().unwrap().lookup(Language::Zh, Kind::HanViet, "你").is_none());
    drop(store);
    let (store, state) = service(directory.path());
    assert!(state.dictionaries().unwrap().lookup(Language::Zh, Kind::HanViet, "你").is_none());
    store.save_entry(EntryMutation { dictionary_id: key.dictionary_id.clone(), entry: entry("你", &["nhĩ riêng", "nhĩ"]) }, &state).unwrap();
    assert_eq!(state.dictionaries().unwrap().lookup(Language::Zh, Kind::HanViet, "你").unwrap().reading.as_deref(), Some("nhĩ riêng"));
    let invalid = store.save_entry(EntryMutation { dictionary_id: key.dictionary_id, entry: entry("你好", &["không phải một chữ"]) }, &state).unwrap_err();
    assert_eq!(invalid.code, "invalidHanCharacter");
}

#[test]
fn japanese_editing_preserves_reading_pos_aliases_and_deterministic_ambiguous_provenance() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let metadata = store.save_metadata(MetadataMutation { id: None, name: "Japanese custom glosses".to_owned(), language: Language::Ja, kind: Kind::Japanese }, &state).unwrap();
    for (headword, meaning, reading, url) in [("甲", "nghĩa A", "こう", "https://example.test/a"), ("乙", "nghĩa B", "おつ", "https://example.test/b")] {
        let mut record = entry(headword, &[meaning]);
        record.reading = Some(reading.to_owned()); record.pos = Some("noun".to_owned()); record.aliases = vec!["共同別名".to_owned()]; record.source_urls = vec![url.to_owned()];
        store.save_entry(EntryMutation { dictionary_id: metadata.id.clone(), entry: record }, &state).unwrap();
    }
    let snapshot = state.dictionaries().unwrap();
    let ambiguous = snapshot.lookup(Language::Ja, Kind::Japanese, "共同別名").unwrap();
    assert_eq!(ambiguous.meanings, ["nghĩa B", "nghĩa A"]);
    assert_eq!(ambiguous.provenance.len(), 2);
    assert_eq!(snapshot.lookup(Language::Ja, Kind::Japanese, "甲").unwrap().reading.as_deref(), Some("こう"));
    assert_eq!(snapshot.lookup(Language::Ja, Kind::Japanese, "甲").unwrap().part_of_speech.as_deref(), Some("noun"));
    store.save_entry(EntryMutation { dictionary_id: metadata.id.clone(), entry: entry("共同別名", &["chính xác thắng bí danh"]) }, &state).unwrap();
    assert_eq!(state.dictionaries().unwrap().lookup(Language::Ja, Kind::Japanese, "共同別名").unwrap().meanings, ["chính xác thắng bí danh"]);
    drop(store);
    let (_, state) = service(directory.path());
    assert_eq!(state.dictionaries().unwrap().lookup(Language::Ja, Kind::Japanese, "甲").unwrap().reading.as_deref(), Some("こう"));
}

#[test]
fn name_kind_choice_reindexes_and_dictionary_changes_cancel_every_window_without_touching_target() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let mut work = Vec::new();
    for label in ["first", "second"] {
        let document = state.register_window(label).unwrap();
        state.observe_target(label, &document.document_id, 7).unwrap();
        work.push(state.prepare_translation(label, TranslationRequest { document_id: document.document_id, source_revision: 1, source_language: Language::Zh, source_text: "测试名字".to_owned(), options: TranslationOptions::default() }).unwrap());
    }
    let metadata = store.save_metadata(MetadataMutation { id: None, name: "Names".to_owned(), language: Language::Zh, kind: Kind::PrimaryNames }, &state).unwrap();
    assert!(work.iter().all(|work| work.cancellation.is_cancelled()));
    assert_eq!(state.health("first").unwrap().target_revision, 7);
    assert_eq!(state.health("second").unwrap().target_revision, 7);
    store.save_entry(EntryMutation { dictionary_id: metadata.id.clone(), entry: entry("测试名字", &["tên"]) }, &state).unwrap();
    store.save_metadata(MetadataMutation { id: Some(metadata.id), name: "NamesPhu".to_owned(), language: Language::Zh, kind: Kind::SecondaryNames }, &state).unwrap();
    let snapshot = state.dictionaries().unwrap();
    assert!(snapshot.lookup(Language::Zh, Kind::PrimaryNames, "测试名字").is_none());
    assert_eq!(snapshot.lookup(Language::Zh, Kind::SecondaryNames, "测试名字").unwrap().meanings, ["tên"]);
}

#[test]
fn shortcuts_lowercase_first_wins_preserve_edits_across_reload_and_export_sorted_utf8() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let path = directory.path().join("Shortcuts.txt");
    fs::write(&path, include_str!("fixtures/dictionary-shortcuts.txt")).unwrap();
    let request = ShortcutImportRequest { path: path.to_string_lossy().into_owned(), encoding: None };
    let preview = store.preview_shortcuts(request.clone()).unwrap();
    assert_eq!(preview.accepted, 4);
    assert_eq!(preview.duplicates, 1);
    assert_eq!(preview.malformed, 1);
    store.commit_shortcuts(&preview.preview_id, &state).unwrap();
    assert_eq!(store.shortcuts().unwrap().iter().find(|record| record.key == "ab").unwrap().value, "đầu tiên");
    store.save_shortcut(ShortcutRecord { key: "AB".to_owned(), value: "tự sửa".to_owned() }, &state).unwrap();
    store.delete_shortcut("B", &state).unwrap();
    let reload = store.preview_shortcuts(request).unwrap();
    store.commit_shortcuts(&reload.preview_id, &state).unwrap();
    let destination = directory.path().join("export-shortcuts.txt");
    store.export_shortcuts(destination.to_str().unwrap()).unwrap();
    assert_eq!(fs::read_to_string(destination).unwrap(), "abcd=dài nhất\naa=cùng độ dài\nab=tự sửa\n");
}

#[test]
fn dictionary_export_requires_new_destination_and_failure_preserves_previous_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let path = directory.path().join("original.txt");
    fs::write(&path, "你好=xin chào|chào bạn").unwrap();
    let preview = store.preview_import(import_request(&path, Kind::VietPhrase, None)).unwrap();
    commit(&store, &state, &preview);
    assert_eq!(store.export(ExportRequest { dictionary_id: preview.dictionary_id.clone(), destination: String::new() }).unwrap_err().code, "invalidDestination");
    assert_eq!(store.export(ExportRequest { dictionary_id: preview.dictionary_id.clone(), destination: path.to_string_lossy().into_owned() }).unwrap_err().code, "invalidDestination");
    let destination = directory.path().join("export.txt");
    fs::write(&destination, "previous").unwrap();
    store.export(ExportRequest { dictionary_id: preview.dictionary_id.clone(), destination: destination.to_string_lossy().into_owned() }).unwrap();
    assert_eq!(fs::read_to_string(&destination).unwrap(), "你好=xin chào|chào bạn\n");
    let mut edited = preview.sample[0].clone();
    edited.meanings = vec!["đã sửa".to_owned()]; // Old payload must not override edited values.
    store.save_entry(EntryMutation { dictionary_id: preview.dictionary_id.clone(), entry: edited }, &state).unwrap();
    store.export(ExportRequest { dictionary_id: preview.dictionary_id.clone(), destination: destination.to_string_lossy().into_owned() }).unwrap();
    assert_eq!(fs::read_to_string(&destination).unwrap(), "你好=đã sửa\n");
    let occupied = directory.path().join("occupied"); fs::create_dir(&occupied).unwrap(); fs::write(occupied.join("retained"), "keep").unwrap();
    assert!(store.export(ExportRequest { dictionary_id: preview.dictionary_id.clone(), destination: occupied.to_string_lossy().into_owned() }).is_err());
    assert_eq!(fs::read_to_string(occupied.join("retained")).unwrap(), "keep");
    assert_eq!(fs::read_to_string(&path).unwrap(), "你好=xin chào|chào bạn");
    store.save_entry(EntryMutation { dictionary_id: preview.dictionary_id.clone(), entry: entry("你好", &["nghĩa = hợp lệ trong SQLite"]) }, &state).unwrap();
    let error = store.export(ExportRequest { dictionary_id: preview.dictionary_id, destination: destination.to_string_lossy().into_owned() }).unwrap_err();
    assert_eq!(error.code, "dictionaryExportNotRepresentable");
    assert_eq!(fs::read_to_string(&destination).unwrap(), "你好=đã sửa\n");
}


#[test]
fn corrupted_database_stays_byte_exact_and_explicit_repair_backs_up_before_new_store() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("user-data.sqlite3");
    let original = b"not a SQLite database\0private local edits";
    fs::write(&path, original).unwrap();
    let store = DictionaryStore::open(directory.path()).unwrap();
    let status = store.storage_status().unwrap();
    assert!(!status.ready);
    assert_eq!(status.error.unwrap().code, "dictionaryDatabaseCorrupt");
    assert_eq!(fs::read(&path).unwrap(), original);
    let state = AppState::default(); state.replace_dictionaries(store.initial_snapshot().unwrap()).unwrap();
    assert!(state.dictionaries().unwrap().lookup(Language::Ja, Kind::Japanese, "日本語").is_some());
    assert!(store.save_entry(EntryMutation { dictionary_id: "bundled:han".to_owned(), entry: entry("你", &["nhĩ"]) }, &state).is_err());
    let backup = directory.path().join("preserved.sqlite3");
    fs::write(&backup, "existing backup").unwrap();
    assert_eq!(store.repair_database(RepairDatabaseRequest { backup_destination: backup.to_string_lossy().into_owned() }, &state).unwrap_err().code, "backupExists");
    assert_eq!(fs::read_to_string(&backup).unwrap(), "existing backup");
    let backup = directory.path().join("new-backup.sqlite3");
    store.repair_database(RepairDatabaseRequest { backup_destination: backup.to_string_lossy().into_owned() }, &state).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(store.storage_status().unwrap().ready);
    assert_ne!(store.storage_status().unwrap().path, path.to_string_lossy());
    drop(store);
    let reopened = DictionaryStore::open(directory.path()).unwrap();
    assert!(reopened.storage_status().unwrap().ready);
    assert_ne!(reopened.storage_status().unwrap().path, path.to_string_lossy());
}

#[test]
fn unsupported_database_version_is_reported_without_overwriting_user_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("user-data.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE private_user_records(value TEXT); INSERT INTO private_user_records VALUES('retain me'); PRAGMA user_version=999;").unwrap();
    }
    let before = fs::read(&path).unwrap();
    let store = DictionaryStore::open(directory.path()).unwrap();
    assert_eq!(store.storage_status().unwrap().error.unwrap().code, "dictionaryDatabaseVersion");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn rule_algorithm_is_committed_transactionally_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let path = directory.path().join("rules.txt");
    fs::write(&path, "对{0}说=nói với {0}\n").unwrap();
    let preview = store.preview_import(import_request(&path, Kind::Rules, None)).unwrap();
    assert_eq!(store.commit_import(CommitImportRequest { preview_ids: vec![preview.preview_id.clone()], rule_algorithm: Some(4) }, &state).unwrap_err().code, "invalidRuleAlgorithm");
    store.commit_import(CommitImportRequest { preview_ids: vec![preview.preview_id], rule_algorithm: Some(3) }, &state).unwrap();
    assert_eq!(state.dictionaries().unwrap().rule_algorithm(), 3);
    drop(store);
    let (_, state) = service(directory.path());
    assert_eq!(state.dictionaries().unwrap().rule_algorithm(), 3);
}

#[test]
fn exact_headword_search_precedes_common_meaning_substrings_before_pagination() {
    let directory = tempfile::tempdir().unwrap();
    let (store, state) = service(directory.path());
    let path = directory.path().join("search.txt");
    let mut text = String::from("x=exact result\n");
    for index in 0..120 { text.push_str(&format!("a{index:03}=x meaning\n")); }
    fs::write(&path, text).unwrap();
    let preview = store.preview_import(import_request(&path, Kind::VietPhrase, None)).unwrap();
    commit(&store, &state, &preview);
    let result = store.search(SearchRequest { query: "x".to_owned(), dictionary_id: Some(preview.dictionary_id), language: Some(Language::Zh), kind: Some(Kind::VietPhrase), offset: 0, limit: 1 }).unwrap();
    assert_eq!(result.total, 121);
    assert_eq!(result.entries[0].entry.headword, "x");
    assert_eq!(result.entries[0].layer, DictionaryLayer::Imported);
}


#[test]
fn incomplete_schema_is_preserved_as_unavailable_instead_of_partly_initialized() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("user-data.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE dictionary_metadata(id TEXT); PRAGMA user_version=1;").unwrap();
    }
    let original = fs::read(&path).unwrap();
    let store = DictionaryStore::open(directory.path()).unwrap();
    assert!(!store.storage_status().unwrap().ready);
    assert_eq!(store.storage_status().unwrap().error.unwrap().code, "dictionaryDatabaseCorrupt");
    assert_eq!(fs::read(&path).unwrap(), original);
}
