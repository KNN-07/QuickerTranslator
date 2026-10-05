pub mod encoding;
pub mod import;
pub mod resources;
pub mod types;
#[cfg(feature = "native")]
mod native;
#[cfg(feature = "native")]
pub use native::*;
pub use types::*;

use std::{collections::{BTreeMap, HashMap, HashSet}, fs, io::Write, path::{Path, PathBuf}, sync::{Arc, Mutex, MutexGuard}};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{models::{AppError, AppResult, DictionaryKind, DictionaryLayer, DictionaryProvenance, SourceLanguage}, state::{AppState, DictionaryEntry, DictionaryIndex, DictionarySnapshot}, storage};
use resources::{BundledData, bundled_data};

const SCHEMA_VERSION: i64 = 1;
const SCHEMA: &str = "
CREATE TABLE dictionary_metadata (id TEXT PRIMARY KEY, name TEXT NOT NULL, language TEXT NOT NULL, kind TEXT NOT NULL, bundled INTEGER NOT NULL DEFAULT 0, source_path TEXT, encoding TEXT, format TEXT);
CREATE TABLE imported_entries (dictionary_id TEXT NOT NULL REFERENCES dictionary_metadata(id), headword TEXT NOT NULL, record TEXT NOT NULL, PRIMARY KEY(dictionary_id,headword));
CREATE TABLE entry_overrides (dictionary_id TEXT NOT NULL REFERENCES dictionary_metadata(id), headword TEXT NOT NULL, record TEXT NOT NULL, PRIMARY KEY(dictionary_id,headword));
CREATE TABLE entry_tombstones (dictionary_id TEXT NOT NULL REFERENCES dictionary_metadata(id), headword TEXT NOT NULL, PRIMARY KEY(dictionary_id,headword));
CREATE TABLE entry_history (id INTEGER PRIMARY KEY, dictionary_id TEXT NOT NULL REFERENCES dictionary_metadata(id), headword TEXT NOT NULL, action TEXT NOT NULL, timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), before_record TEXT, after_record TEXT);
CREATE INDEX history_by_entry ON entry_history(dictionary_id,headword,id);
CREATE TABLE store_settings (key TEXT PRIMARY KEY,value INTEGER NOT NULL);
INSERT INTO store_settings VALUES ('revision',1),('ruleAlgorithm',1);
CREATE TABLE shortcut_imported (key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE shortcut_overrides (key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE shortcut_tombstones (key TEXT PRIMARY KEY);
PRAGMA user_version=1;
";

struct PendingImport { preview: ImportPreview, records: BTreeMap<String, EntryRecord>, format: ImportFormat }
struct Inner {
    connection: Option<Connection>,
    path: PathBuf,
    error: Option<AppError>,
    revision: u64,
    effective: Vec<EffectiveDictionary>,
    rule_algorithm: u8,
    pending: HashMap<String, Arc<PendingImport>>,
    shortcut_pending: Option<(String, BTreeMap<String, String>)>,
}

pub struct DictionaryStore { inner: Mutex<Inner>, bundled: Arc<BundledData>, app_data: PathBuf }

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseSelection { version: u8, active_database: PathBuf }

impl DictionaryStore {
    /// Resource failure is fatal. Database failure is retained as an actionable
    /// read-only status so the application can still show a repair choice.
    pub fn open(app_data: &Path) -> AppResult<Self> {
        let bundled = bundled_data()?;
        let default_path = app_data.join("user-data.sqlite3");
        let selection_path = app_data.join("dictionary-store.json");
        let selection = if selection_path.exists() {
            fs::read(&selection_path).map_err(|error| AppError::io("dictionaryDatabaseUnavailable", "The saved database selection could not be read; its file was preserved", &error))
                .and_then(|bytes| serde_json::from_slice::<DatabaseSelection>(&bytes).map_err(|_| AppError::new("dictionaryDatabaseUnavailable", "The saved database selection is invalid; its file was preserved.")))
                .and_then(|selection| if selection.version == 1 && selection.active_database.parent() == Some(app_data) { Ok(selection.active_database) } else { Err(AppError::new("dictionaryDatabaseVersion", "The saved database selection has an unsupported version or location; its file was preserved.")) })
        } else { Ok(default_path.clone()) };
        let (path, result) = match selection {
            Ok(path) => {
                let result = fs::create_dir_all(app_data).map_err(|error| AppError::io("dictionaryDatabaseUnavailable", "Application data is not writable", &error)).and_then(|_| open_database(&path, &bundled));
                (path, result)
            },
            Err(error) => (default_path, Err(error)),
        };
        let (connection, error, revision, effective, rule_algorithm) = match result {
            Ok((connection, effective, revision, algorithm)) => (Some(connection), None, revision, effective, algorithm),
            Err(error) => (None, Some(error), 1, effective_dictionaries(None, &bundled)?, 1),
        };
        Ok(Self { inner: Mutex::new(Inner { connection, path, error, revision, effective, rule_algorithm, pending: HashMap::new(), shortcut_pending: None }), bundled, app_data: app_data.to_owned() })
    }

    fn lock(&self) -> AppResult<MutexGuard<'_, Inner>> {
        self.inner.lock().map_err(|_| AppError::new("dictionaryDatabaseUnavailable", "Dictionary state is unavailable; restart safely without removing user data."))
    }

    pub fn storage_status(&self) -> AppResult<DictionaryStorageStatus> {
        let inner = self.lock()?;
        Ok(DictionaryStorageStatus { ready: inner.connection.is_some(), path: inner.path.to_string_lossy().into_owned(), error: inner.error.clone() })
    }

    pub fn initial_snapshot(&self) -> AppResult<DictionarySnapshot> {
        let inner = self.lock()?;
        build_snapshot(&inner.effective, &self.bundled, inner.revision, inner.rule_algorithm)
    }

    pub fn catalog(&self) -> AppResult<DictionaryCatalog> {
        let inner = self.lock()?;
        let dictionaries = inner.effective.iter().map(|dictionary| dictionary.metadata.clone()).collect();
        let rule_algorithm = inner.rule_algorithm;
        Ok(DictionaryCatalog { revision: inner.revision, rule_algorithm, dictionaries, manifest: self.bundled.manifest.clone(), attribution: self.bundled.attribution.clone(), licenses: self.bundled.licenses.clone() })
    }

    pub fn preview_import(&self, request: ImportRequest) -> AppResult<ImportPreview> {
        validate_metadata(&request.name, request.language, request.kind)?;
        let decoded = encoding::read_text(Path::new(&request.path), request.encoding.as_deref())?;
        let format = request.format.unwrap_or_else(|| import::format_for(request.kind));
        if format != import::format_for(request.kind) && format != ImportFormat::Legacy {
            return Err(AppError::new("invalidDictionaryEntry", "The selected import grammar does not match the dictionary kind."));
        }
        let parsed = import::parse_dictionary(&decoded.text, request.kind, format);
        let id = request.dictionary_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let mut inner = self.lock()?;
        if let Some(connection) = &inner.connection {
            if let Some(existing) = find_metadata(connection, &id)? {
                if existing.language != request.language || existing.kind != request.kind || existing.bundled {
                    return Err(AppError::new("invalidDictionaryEntry", "Imports must use the existing dictionary language and kind, and cannot replace bundled data."));
                }
            }
        }
        let preview = ImportPreview { preview_id: Uuid::new_v4().to_string(), dictionary_id: id, name: request.name, path: request.path,
            language: request.language, kind: request.kind, encoding: decoded.encoding, encoding_detected: decoded.detected, decoded_text: import::preview_text(&decoded.text),
            accepted: parsed.entries.len(), duplicates: parsed.duplicates, malformed: parsed.issues.iter().filter(|issue| issue.code != "duplicate").count(), issues: parsed.issues,
            sample: parsed.entries.values().take(30).cloned().collect() };
        inner.pending.retain(|_, pending| pending.preview.dictionary_id != preview.dictionary_id);
        inner.pending.insert(preview.preview_id.clone(), Arc::new(PendingImport { preview: preview.clone(), records: parsed.entries, format }));
        Ok(preview)
    }

    pub fn reload_preview(&self, dictionary_id: &str) -> AppResult<ImportPreview> {
        let request = {
            let inner = self.lock()?;
            let metadata = find_metadata(available_connection(&inner)?, dictionary_id)?.ok_or_else(not_found)?;
            ImportRequest { path: metadata.source_path.ok_or_else(|| AppError::new("dictionaryNotReloadable", "This dictionary has no original import file. Choose a file to import."))?, name: metadata.name,
                language: metadata.language, kind: metadata.kind, dictionary_id: Some(metadata.id), encoding: metadata.encoding, format: metadata.format }
        };
        self.preview_import(request)
    }

    pub fn commit_import(&self, request: CommitImportRequest, state: &AppState) -> AppResult<u64> {
        if request.preview_ids.is_empty() { return Err(invalid_preview()); }
        if request.rule_algorithm.is_some_and(|algorithm| !(1..=3).contains(&algorithm)) {
            return Err(AppError::new("invalidRuleAlgorithm", "ThuatToanNhan must be 1, 2 or 3."));
        }
        let mut inner = self.lock()?;
        let previews: Vec<_> = request.preview_ids.iter().map(|id| inner.pending.get(id).cloned().ok_or_else(invalid_preview)).collect::<AppResult<_>>()?;
        let mut ids = HashSet::new();
        if previews.iter().any(|pending| !ids.insert(&pending.preview.dictionary_id)) { return Err(invalid_preview()); }
        let result = self.commit(&mut inner, state, |transaction| {
            for pending in &previews {
                let preview = &pending.preview;
                if let Some(metadata) = find_metadata(transaction, &preview.dictionary_id)? {
                    if metadata.bundled || metadata.language != preview.language || metadata.kind != preview.kind { return Err(invalid_preview()); }
                }
                transaction.execute("INSERT INTO dictionary_metadata(id,name,language,kind,bundled,source_path,encoding,format) VALUES(?1,?2,?3,?4,0,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,source_path=excluded.source_path,encoding=excluded.encoding,format=excluded.format",
                    params![preview.dictionary_id, preview.name, enum_string(&preview.language)?, enum_string(&preview.kind)?, preview.path, preview.encoding, enum_string(&pending.format)?]).map_err(db_error)?;
                transaction.execute("DELETE FROM imported_entries WHERE dictionary_id=?1", [&preview.dictionary_id]).map_err(db_error)?;
                let mut insert = transaction.prepare("INSERT INTO imported_entries(dictionary_id,headword,record) VALUES(?1,?2,?3)").map_err(db_error)?;
                for record in pending.records.values() { insert.execute(params![preview.dictionary_id, record.headword, record_json(record)?]).map_err(db_error)?; }
            }
            if let Some(algorithm) = request.rule_algorithm { transaction.execute("UPDATE store_settings SET value=?1 WHERE key='ruleAlgorithm'", [algorithm]).map_err(db_error)?; }
            Ok(())
        })?;
        for id in request.preview_ids { inner.pending.remove(&id); }
        Ok(result)
    }

    fn commit<F>(&self, inner: &mut Inner, state: &AppState, operation: F) -> AppResult<u64>
    where F: FnOnce(&Transaction<'_>) -> AppResult<()> {
        let next = inner.revision.checked_add(1).filter(|revision| *revision <= i64::MAX as u64).ok_or_else(|| AppError::new("dictionaryDatabaseUnavailable", "The dictionary revision limit was reached."))?;
        let error = inner.error.clone();
        let connection = inner.connection.as_mut().ok_or_else(|| error.unwrap_or_else(database_unavailable))?;
        let transaction = connection.transaction().map_err(db_error)?;
        operation(&transaction)?;
        transaction.execute("UPDATE store_settings SET value=?1 WHERE key='revision'", [next as i64]).map_err(db_error)?;
        let effective = effective_dictionaries(Some(&transaction), &self.bundled)?;
        let algorithm = rule_algorithm(&transaction)?;
        let snapshot = build_snapshot(&effective, &self.bundled, next, algorithm)?;
        transaction.commit().map_err(db_error)?;
        inner.revision = next;
        inner.effective = effective;
        inner.rule_algorithm = algorithm;
        // Held under the store mutex: concurrent commits cannot publish out of order.
        state.replace_dictionaries(snapshot)
    }

    pub fn save_metadata(&self, request: MetadataMutation, state: &AppState) -> AppResult<DictionaryMetadata> {
        validate_metadata(&request.name, request.language, request.kind)?;
        let id = request.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let mut inner = self.lock()?;
        self.commit(&mut inner, state, |transaction| {
            if let Some(existing) = find_metadata(transaction, &id)? {
                if existing.bundled { return Err(AppError::new("invalidDictionaryEntry", "Bundled dictionary metadata is immutable; edit individual entries or create a user dictionary.")); }
                if existing.language != request.language { return Err(AppError::new("invalidDictionaryEntry", "A dictionary's source language cannot be changed.")); }
                let entries = read_records(transaction, "imported_entries", &id)?.into_iter().chain(read_records(transaction, "entry_overrides", &id)?);
                for (_, record) in entries { import::validate_entry(request.kind, &record)?; }
            }
            transaction.execute("INSERT INTO dictionary_metadata(id,name,language,kind) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind",
                params![id, request.name, enum_string(&request.language)?, enum_string(&request.kind)?]).map_err(db_error)?;
            Ok(())
        })?;
        inner.effective.iter().find(|dictionary| dictionary.metadata.id == id).map(|dictionary| dictionary.metadata.clone()).ok_or_else(not_found)
    }

    pub fn save_entry(&self, mut request: EntryMutation, state: &AppState) -> AppResult<u64> {
        let mut inner = self.lock()?;
        let metadata = find_metadata(available_connection(&inner)?, &request.dictionary_id)?.ok_or_else(not_found)?;
        import::validate_entry(metadata.kind, &request.entry)?;
        request.entry.payload = None;
        if metadata.kind == DictionaryKind::HanViet { request.entry.reading = request.entry.meanings.first().cloned(); }
        self.commit(&mut inner, state, |transaction| {
            let before = effective_record(transaction, &self.bundled, &request.dictionary_id, &request.entry.headword)?;
            transaction.execute("INSERT INTO entry_overrides(dictionary_id,headword,record) VALUES(?1,?2,?3) ON CONFLICT(dictionary_id,headword) DO UPDATE SET record=excluded.record",
                params![request.dictionary_id, request.entry.headword, record_json(&request.entry)?]).map_err(db_error)?;
            transaction.execute("DELETE FROM entry_tombstones WHERE dictionary_id=?1 AND headword=?2", params![request.dictionary_id, request.entry.headword]).map_err(db_error)?;
            history(transaction, &request.dictionary_id, &request.entry.headword, "save", before.as_deref(), Some(&request.entry))
        })
    }

    pub fn delete_entry(&self, request: EntryKey, state: &AppState) -> AppResult<u64> {
        let mut inner = self.lock()?;
        if find_metadata(available_connection(&inner)?, &request.dictionary_id)?.is_none() { return Err(not_found()); }
        self.commit(&mut inner, state, |transaction| {
            let before = effective_record(transaction, &self.bundled, &request.dictionary_id, &request.headword)?;
            if before.is_none() { return Err(AppError::new("dictionaryEntryNotFound", "This entry does not exist; refresh the search before editing.")); }
            transaction.execute("DELETE FROM entry_overrides WHERE dictionary_id=?1 AND headword=?2", params![request.dictionary_id, request.headword]).map_err(db_error)?;
            transaction.execute("INSERT OR IGNORE INTO entry_tombstones(dictionary_id,headword) VALUES(?1,?2)", params![request.dictionary_id, request.headword]).map_err(db_error)?;
            history(transaction, &request.dictionary_id, &request.headword, "delete", before.as_deref(), None)
        })
    }

    pub fn search(&self, request: SearchRequest) -> AppResult<SearchResult> {
        let inner = self.lock()?;
        let query = regex::RegexBuilder::new(&regex::escape(&request.query)).case_insensitive(true).build()
            .map_err(|_| AppError::new("invalidDictionaryEntry", "This search query is too large; use a shorter query."))?;
        let mut matched = Vec::new();
        for dictionary in &inner.effective {
            let metadata = &dictionary.metadata;
            if request.dictionary_id.as_ref().is_some_and(|id| id != &metadata.id) || request.language.is_some_and(|language| language != metadata.language) || request.kind.is_some_and(|kind| kind != metadata.kind) { continue; }
            for resolved in dictionary.records.values() {
                let record = &resolved.record;
                let matches = query.is_match(&record.headword) || record.aliases.iter().any(|alias| query.is_match(alias))
                    || record.meanings.iter().any(|meaning| query.is_match(meaning)) || record.reading.as_ref().is_some_and(|reading| query.is_match(reading));
                if matches { matched.push((metadata, resolved)); }
            }
        }
        matched.sort_by(|(left_dictionary, left), (right_dictionary, right)| (right.record.headword == request.query).cmp(&(left.record.headword == request.query))
            .then(left.record.headword.cmp(&right.record.headword)).then(left_dictionary.id.cmp(&right_dictionary.id)));
        let total = matched.len();
        let entries = matched.into_iter().skip(request.offset).take(request.limit.clamp(1, 500)).map(|(metadata, resolved)| EntryView {
            dictionary_id: metadata.id.clone(), language: metadata.language, kind: metadata.kind, entry: resolved.record.as_ref().clone(), layer: resolved.layer,
            provenance: vec![provenance(metadata, resolved.layer, &resolved.record)],
        }).collect();
        Ok(SearchResult { total, entries })
    }

    pub fn entry_history(&self, request: EntryKey) -> AppResult<Vec<EntryHistory>> {
        let inner = self.lock()?;
        let mut statement = available_connection(&inner)?.prepare("SELECT id,action,timestamp,before_record,after_record FROM entry_history WHERE dictionary_id=?1 AND headword=?2 ORDER BY id DESC").map_err(db_error)?;
        let rows = statement.query_map(params![request.dictionary_id, request.headword], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?))).map_err(db_error)?;
        rows.map(|row| {
            let (id, action, timestamp, before, after) = row.map_err(db_error)?;
            Ok(EntryHistory { id, action, timestamp, before: before.map(|json| decode_record(&json)).transpose()?, after: after.map(|json| decode_record(&json)).transpose()? })
        }).collect()
    }

    pub fn export(&self, request: ExportRequest) -> AppResult<ExportResult> {
        if request.destination.trim().is_empty() { return Err(AppError::new("invalidDestination", "Choose a destination for the UTF-8 export.")); }
        let inner = self.lock()?;
        reject_import_destination(&inner, &request.destination)?;
        let dictionary = inner.effective.iter().find(|dictionary| dictionary.metadata.id == request.dictionary_id).ok_or_else(not_found)?;
        let entries = dictionary.records.len();
        if dictionary.metadata.kind != DictionaryKind::Ignored && dictionary.records.values().any(|resolved| {
            let record = &resolved.record;
            record.headword.contains('=') || record.payload.as_ref().is_some_and(|payload| payload.contains(['=', '\r', '\n']))
                || (record.payload.is_none() && record.meanings.iter().any(|meaning| meaning.contains(['=', '\r', '\n'])))
        }) {
            return Err(AppError::new("dictionaryExportNotRepresentable", "This dictionary contains equals signs or multiline values that strict legacy key/value text cannot represent. No destination bytes were replaced."));
        }
        let result = storage::atomic_replace_with(Path::new(&request.destination), |file| {
            for resolved in dictionary.records.values() {
                let record = &resolved.record;
                if dictionary.metadata.kind == DictionaryKind::Ignored { writeln!(file, "{}", record.headword)?; }
                else if let Some(payload) = &record.payload { writeln!(file, "{}={payload}", record.headword)?; }
                else { writeln!(file, "{}={}", record.headword, record.meanings.join("/"))?; }
            }
            Ok(())
        })?;
        Ok(ExportResult { entries, directory_sync_confirmed: result.directory_sync_confirmed })
    }

    pub fn preview_shortcuts(&self, request: ShortcutImportRequest) -> AppResult<ShortcutPreview> {
        let decoded = encoding::read_text(Path::new(&request.path), request.encoding.as_deref())?;
        let (entries, issues, duplicates) = import::parse_shortcuts(&decoded.text);
        let preview = ShortcutPreview { preview_id: Uuid::new_v4().to_string(), encoding: decoded.encoding, encoding_detected: decoded.detected,
            decoded_text: import::preview_text(&decoded.text), accepted: entries.len(), duplicates, malformed: issues.iter().filter(|issue| issue.code != "duplicate").count(), issues,
            sample: entries.iter().take(30).map(|(key, value)| ShortcutRecord { key: key.clone(), value: value.clone() }).collect() };
        self.lock()?.shortcut_pending = Some((preview.preview_id.clone(), entries));
        Ok(preview)
    }

    pub fn commit_shortcuts(&self, preview_id: &str, state: &AppState) -> AppResult<u64> {
        let mut inner = self.lock()?;
        let pending = inner.shortcut_pending.take().filter(|(id, _)| id == preview_id).ok_or_else(invalid_preview)?;
        let result = self.commit(&mut inner, state, |transaction| {
            transaction.execute("DELETE FROM shortcut_imported", []).map_err(db_error)?;
            let mut statement = transaction.prepare("INSERT INTO shortcut_imported(key,value) VALUES(?1,?2)").map_err(db_error)?;
            for (key, value) in &pending.1 { statement.execute(params![key, value]).map_err(db_error)?; }
            Ok(())
        });
        if result.is_err() { inner.shortcut_pending = Some(pending); }
        result
    }

    pub fn shortcuts(&self) -> AppResult<Vec<ShortcutRecord>> {
        let inner = self.lock()?;
        let connection = available_connection(&inner)?;
        let mut statement = connection.prepare("SELECT key,value FROM (SELECT key,value FROM shortcut_overrides UNION ALL SELECT key,value FROM shortcut_imported WHERE key NOT IN (SELECT key FROM shortcut_overrides)) WHERE key NOT IN (SELECT key FROM shortcut_tombstones)").map_err(db_error)?;
        let mut records = statement.query_map([], |row| Ok(ShortcutRecord { key: row.get(0)?, value: row.get(1)? })).map_err(db_error)?.collect::<Result<Vec<_>, _>>().map_err(db_error)?;
        records.sort_by(|left, right| right.key.chars().count().cmp(&left.key.chars().count()).then(left.key.cmp(&right.key)));
        Ok(records)
    }

    pub fn save_shortcut(&self, mut record: ShortcutRecord, state: &AppState) -> AppResult<u64> {
        if record.key.trim().is_empty() || record.key.contains(['=', '\n', '\r']) || record.value.contains(['=', '\n', '\r']) { return Err(AppError::new("invalidDictionaryEntry", "A shortcut needs a nonempty key and no equals signs or newlines.")); }
        record.key = record.key.to_lowercase();
        let mut inner = self.lock()?;
        self.commit(&mut inner, state, |transaction| {
            transaction.execute("INSERT INTO shortcut_overrides(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![record.key, record.value]).map_err(db_error)?;
            transaction.execute("DELETE FROM shortcut_tombstones WHERE key=?1", [&record.key]).map_err(db_error)?;
            Ok(())
        })
    }

    pub fn delete_shortcut(&self, key: &str, state: &AppState) -> AppResult<u64> {
        let key = key.to_lowercase();
        let mut inner = self.lock()?;
        self.commit(&mut inner, state, |transaction| {
            transaction.execute("DELETE FROM shortcut_overrides WHERE key=?1", [&key]).map_err(db_error)?;
            transaction.execute("INSERT OR IGNORE INTO shortcut_tombstones(key) VALUES(?1)", [&key]).map_err(db_error)?;
            Ok(())
        })
    }

    pub fn export_shortcuts(&self, destination: &str) -> AppResult<ExportResult> {
        if destination.trim().is_empty() { return Err(AppError::new("invalidDestination", "Choose a destination for shortcuts.")); }
        { let inner = self.lock()?; reject_import_destination(&inner, destination)?; }
        let shortcuts = self.shortcuts()?;
        let result = storage::atomic_replace_with(Path::new(destination), |file| { for shortcut in &shortcuts { writeln!(file, "{}={}", shortcut.key, shortcut.value)?; } Ok(()) })?;
        Ok(ExportResult { entries: shortcuts.len(), directory_sync_confirmed: result.directory_sync_confirmed })
    }

    pub fn repair_database(&self, request: RepairDatabaseRequest, state: &AppState) -> AppResult<u64> {
        let mut inner = self.lock()?;
        if inner.connection.is_some() { return Err(AppError::new("dictionaryDatabaseReady", "The database is healthy; no repair replacement is necessary.")); }
        let destination = Path::new(&request.backup_destination);
        if request.backup_destination.trim().is_empty() || !inner.path.is_file() { return Err(AppError::new("dictionaryDatabaseUnavailable", "The original database must be available for a preserved backup before a new database can be created.")); }
        let mut copies = Vec::new();
        for suffix in ["", "-wal", "-shm"] {
            let source = PathBuf::from(format!("{}{}", inner.path.to_string_lossy(), suffix));
            if !source.is_file() { continue; }
            let target = PathBuf::from(format!("{}{}", destination.to_string_lossy(), suffix));
            if target.exists() { return Err(AppError::new("backupExists", "Choose an unused backup destination; existing files will not be overwritten.")); }
            copies.push((source, target));
        }
        let selection = self.app_data.join("dictionary-store.json");
        if selection.is_file() {
            let target = PathBuf::from(format!("{}.selection.json", destination.to_string_lossy()));
            if target.exists() { return Err(AppError::new("backupExists", "Choose an unused backup destination; existing files will not be overwritten.")); }
            copies.push((selection, target));
        }
        for (source, target) in &copies { preserved_copy(source, target)?; }
        let path = self.app_data.join(format!("user-data-{}.sqlite3", Uuid::new_v4()));
        let (mut connection, effective, _, algorithm) = open_database(&path, &self.bundled)?;
        let next = inner.revision.checked_add(1).filter(|revision| *revision <= i64::MAX as u64).ok_or_else(database_unavailable)?;
        let transaction = connection.transaction().map_err(db_error)?;
        transaction.execute("UPDATE store_settings SET value=?1 WHERE key='revision'", [next as i64]).map_err(db_error)?;
        let snapshot = build_snapshot(&effective, &self.bundled, next, algorithm)?;
        transaction.commit().map_err(db_error)?;
        storage::atomic_replace_json(&self.app_data.join("dictionary-store.json"), &DatabaseSelection { version: 1, active_database: path.clone() })?;
        inner.connection = Some(connection); inner.path = path; inner.error = None; inner.revision = next;
        inner.effective = effective; inner.rule_algorithm = algorithm;
        state.replace_dictionaries(snapshot)
    }
}

struct ResolvedRecord { record: Arc<EntryRecord>, layer: DictionaryLayer }
struct EffectiveDictionary { metadata: DictionaryMetadata, records: BTreeMap<String, ResolvedRecord>, tombstones: HashSet<String> }

fn effective_dictionaries(connection: Option<&Connection>, bundled: &BundledData) -> AppResult<Vec<EffectiveDictionary>> {
    let metadata = if let Some(connection) = connection { all_metadata(connection)? } else { bundled.dictionaries.iter().map(|dictionary| dictionary.metadata.clone()).collect() };
    if bundled.dictionaries.iter().any(|base| !metadata.iter().any(|metadata| metadata.id == base.metadata.id)) { return Err(database_corrupt()); }
    let mut dictionaries = Vec::new();
    for mut metadata in metadata {
        validate_metadata(&metadata.name, metadata.language, metadata.kind).map_err(|_| database_corrupt())?;
        let base = bundled.dictionaries.iter().find(|dictionary| dictionary.metadata.id == metadata.id);
        if let Some(base) = base {
            if !metadata.bundled || metadata.language != base.metadata.language || metadata.kind != base.metadata.kind { return Err(database_corrupt()); }
        } else if metadata.bundled { return Err(database_corrupt()); }
        let mut records: BTreeMap<String, ResolvedRecord> = bundled.dictionaries.iter().find(|dictionary| dictionary.metadata.id == metadata.id)
            .map(|dictionary| dictionary.records.iter().map(|(key, record)| (key.clone(), ResolvedRecord { record: Arc::clone(record), layer: DictionaryLayer::Bundled })).collect()).unwrap_or_default();
        let mut tombstones = HashSet::new();
        if let Some(connection) = connection {
            let imported = read_records(connection, "imported_entries", &metadata.id)?;
            metadata.imported_count = imported.len();
            for (key, record) in imported { import::validate_entry(metadata.kind, &record).map_err(|_| database_corrupt())?; records.insert(key, ResolvedRecord { record: Arc::new(record), layer: DictionaryLayer::Imported }); }
            let edited = read_records(connection, "entry_overrides", &metadata.id)?;
            metadata.edited_count = edited.len();
            for (key, record) in edited { import::validate_entry(metadata.kind, &record).map_err(|_| database_corrupt())?; records.insert(key, ResolvedRecord { record: Arc::new(record), layer: DictionaryLayer::Edited }); }
            let mut statement = connection.prepare("SELECT headword FROM entry_tombstones WHERE dictionary_id=?1").map_err(db_error)?;
            tombstones = statement.query_map([&metadata.id], |row| row.get::<_, String>(0)).map_err(db_error)?.collect::<Result<_, _>>().map_err(db_error)?;
            metadata.tombstone_count = tombstones.len();
            for key in &tombstones { records.remove(key); }
        }
        metadata.entry_count = records.len();
        metadata.alias_count = records.values().flat_map(|resolved| resolved.record.aliases.iter())
            .filter(|alias| !records.contains_key(*alias)).collect::<HashSet<_>>().len();
        dictionaries.push(EffectiveDictionary { metadata, records, tombstones });
    }
    Ok(dictionaries)
}

type CanonicalIndex = BTreeMap<String, (Arc<DictionaryEntry>, Arc<EntryRecord>)>;

fn build_snapshot(dictionaries: &[EffectiveDictionary], bundled: &BundledData, revision: u64, algorithm: u8) -> AppResult<DictionarySnapshot> {
    let changed: HashSet<_> = dictionaries.iter().filter(|dictionary| !dictionary.tombstones.is_empty()
        || dictionary.records.values().any(|resolved| resolved.layer != DictionaryLayer::Bundled))
        .map(|dictionary| (dictionary.metadata.language, dictionary.metadata.kind)).collect();
    let mut index: DictionaryIndex = HashMap::new();
    for base in &bundled.dictionaries {
        if !changed.contains(&(base.metadata.language, base.metadata.kind)) {
            index.entry(base.metadata.language).or_default().insert(base.metadata.kind, base.index.clone());
        }
    }
    let mut canonical: HashMap<(SourceLanguage, DictionaryKind), CanonicalIndex> = HashMap::new();
    let mut blocked: HashMap<(SourceLanguage, DictionaryKind), HashSet<String>> = HashMap::new();
    // Layer precedence is global within each matching kind; dictionary row order
    // breaks equal-layer duplicates deterministically (latest user dictionary wins).
    for layer in [DictionaryLayer::Bundled, DictionaryLayer::Imported, DictionaryLayer::Edited] {
        for dictionary in dictionaries {
            if !changed.contains(&(dictionary.metadata.language, dictionary.metadata.kind)) { continue; }
            let entries = canonical.entry((dictionary.metadata.language, dictionary.metadata.kind)).or_default();
            let blocked_keys = blocked.entry((dictionary.metadata.language, dictionary.metadata.kind)).or_default();
            for (headword, resolved) in &dictionary.records {
                if resolved.layer != layer { continue; }
                let record = &resolved.record;
                let entry = if layer == DictionaryLayer::Bundled {
                    Arc::clone(bundled.dictionaries.iter().find(|base| base.metadata.id == dictionary.metadata.id)
                        .and_then(|base| base.resolved_entries.get(headword)).ok_or_else(database_corrupt)?)
                } else {
                    Arc::new(DictionaryEntry { headword: record.headword.clone(), meanings: record.meanings.clone(), reading: record.reading.clone(), part_of_speech: record.pos.clone(), provenance: vec![provenance(&dictionary.metadata, layer, record)] })
                };
                entries.insert(headword.clone(), (entry, Arc::clone(record)));
                blocked_keys.remove(headword);
            }
            if layer == DictionaryLayer::Edited {
                for key in &dictionary.tombstones { entries.remove(key); blocked_keys.insert(key.clone()); }
            }
        }
    }
    for ((language, kind), entries) in canonical {
        let base = bundled.dictionaries.iter().find(|base| base.metadata.language == language && base.metadata.kind == kind).map(|base| &base.records);
        index.entry(language).or_default().insert(kind, index_canonical(&entries, blocked.get(&(language, kind)), base));
    }
    let snapshot = DictionarySnapshot::new(revision, index, algorithm);
    snapshot.validate_engine()?;
    Ok(snapshot)
}

fn index_canonical(entries: &CanonicalIndex, blocked: Option<&HashSet<String>>, base: Option<&BTreeMap<String, Arc<EntryRecord>>>) -> HashMap<String, Arc<DictionaryEntry>> {
    let mut aliases: BTreeMap<&str, Vec<Arc<DictionaryEntry>>> = BTreeMap::new();
    let mut index = HashMap::with_capacity(entries.len() + entries.values().map(|(_, record)| record.aliases.len()).sum::<usize>());
    for (headword, (entry, record)) in entries {
        index.insert(headword.clone(), Arc::clone(entry));
        // Immutable genuine forms still resolve to a headword after its meanings
        // are overridden by a legacy import that cannot describe aliases.
        let inherited = base.and_then(|base| base.get(headword)).filter(|base| !Arc::ptr_eq(base, record));
        for alias in record.aliases.iter().chain(inherited.into_iter().flat_map(|record| record.aliases.iter())) {
            if entries.contains_key(alias) { continue; }
            let candidates = aliases.entry(alias).or_default();
            if !candidates.last().is_some_and(|previous| Arc::ptr_eq(previous, entry)) { candidates.push(Arc::clone(entry)); }
        }
    }
    for (alias, candidates) in aliases {
        let mut candidates = candidates.into_iter();
        let first = candidates.next().expect("nonempty alias candidates");
        let Some(second) = candidates.next() else { index.insert(alias.to_owned(), first); continue; };
        let mut aggregate = DictionaryEntry { headword: alias.to_owned(), meanings: first.meanings.clone(), reading: first.reading.clone(), part_of_speech: first.part_of_speech.clone(), provenance: first.provenance.clone() };
        for candidate in std::iter::once(second).chain(candidates) {
            for meaning in &candidate.meanings { if !aggregate.meanings.contains(meaning) { aggregate.meanings.push(meaning.clone()); } }
            for source in &candidate.provenance { if !aggregate.provenance.contains(source) { aggregate.provenance.push(source.clone()); } }
        }
        index.insert(alias.to_owned(), Arc::new(aggregate));
    }
    if let Some(blocked) = blocked { for key in blocked { index.remove(key); } }
    index
}

fn provenance(metadata: &DictionaryMetadata, layer: DictionaryLayer, record: &EntryRecord) -> DictionaryProvenance {
    DictionaryProvenance { dictionary_id: metadata.id.clone(), dictionary_name: metadata.name.clone(), kind: metadata.kind, layer, source_urls: record.source_urls.clone() }
}

fn open_database(path: &Path, bundled: &BundledData) -> AppResult<(Connection, Vec<EffectiveDictionary>, u64, u8)> {
    let existed = path.exists();
    let connection = Connection::open(path).map_err(db_error)?;
    let check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0)).map_err(db_error)?;
    if check != "ok" { return Err(database_corrupt()); }
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).map_err(db_error)?;
    if version > SCHEMA_VERSION || (version == 0 && existed) || version < 0 {
        return Err(AppError::new("dictionaryDatabaseVersion", "This database uses an unsupported schema version. Its bytes were preserved; choose a backup and explicit new database, or open it with a compatible application."));
    }
    connection.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_error)?;
    connection.pragma_update(None, "foreign_keys", "ON").map_err(db_error)?;
    if version == 0 {
        connection.execute_batch("BEGIN IMMEDIATE;").map_err(db_error)?;
        let initialization = (|| {
            connection.execute_batch(SCHEMA).map_err(db_error)?;
            for dictionary in &bundled.dictionaries {
                connection.execute("INSERT INTO dictionary_metadata(id,name,language,kind,bundled) VALUES(?1,?2,?3,?4,1)", params![dictionary.metadata.id, dictionary.metadata.name, enum_string(&dictionary.metadata.language)?, enum_string(&dictionary.metadata.kind)?]).map_err(db_error)?;
            }
            Ok::<_, AppError>(())
        })();
        if let Err(error) = initialization { let _ = connection.execute_batch("ROLLBACK;"); return Err(error); }
        connection.execute_batch("COMMIT;").map_err(db_error)?;
    }
    // Validate all schema-backed data before changing the existing journal mode.
    validate_schema(&connection)?;
    let existing_revision = revision(&connection)?;
    let algorithm = rule_algorithm(&connection)?;
    let effective = effective_dictionaries(Some(&connection), bundled)?;
    connection.pragma_update(None, "journal_mode", "WAL").map_err(db_error)?;
    Ok((connection, effective, existing_revision, algorithm))
}

fn all_metadata(connection: &Connection) -> AppResult<Vec<DictionaryMetadata>> {
    let mut statement = connection.prepare("SELECT id,name,language,kind,bundled,source_path,encoding,format FROM dictionary_metadata ORDER BY rowid").map_err(db_error)?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, bool>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, Option<String>>(7)?))).map_err(db_error)?;
    rows.map(|row| {
        let (id, name, language, kind, bundled, source_path, encoding, format) = row.map_err(db_error)?;
        Ok(DictionaryMetadata { id, name, language: decode_enum(&language)?, kind: decode_enum(&kind)?, bundled, source_path, encoding, format: format.map(|value| decode_enum(&value)).transpose()?, entry_count: 0, alias_count: 0, imported_count: 0, edited_count: 0, tombstone_count: 0 })
    }).collect()
}

fn find_metadata(connection: &Connection, id: &str) -> AppResult<Option<DictionaryMetadata>> { Ok(all_metadata(connection)?.into_iter().find(|metadata| metadata.id == id)) }

fn read_records(connection: &Connection, table: &str, id: &str) -> AppResult<BTreeMap<String, EntryRecord>> {
    let sql = match table { "imported_entries" => "SELECT headword,record FROM imported_entries WHERE dictionary_id=?1 ORDER BY headword", "entry_overrides" => "SELECT headword,record FROM entry_overrides WHERE dictionary_id=?1 ORDER BY headword", _ => return Err(database_corrupt()) };
    let mut statement = connection.prepare(sql).map_err(db_error)?;
    let rows = statement.query_map([id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(db_error)?;
    rows.map(|row| { let (key, json) = row.map_err(db_error)?; let record = decode_record(&json)?; if key != record.headword { return Err(database_corrupt()); } Ok((key, record)) }).collect()
}

fn effective_record(connection: &Connection, bundled: &BundledData, dictionary_id: &str, headword: &str) -> AppResult<Option<Arc<EntryRecord>>> {
    let tombstone: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM entry_tombstones WHERE dictionary_id=?1 AND headword=?2)", params![dictionary_id, headword], |row| row.get(0)).map_err(db_error)?;
    if tombstone { return Ok(None); }
    for table in ["entry_overrides", "imported_entries"] {
        let sql = if table == "entry_overrides" { "SELECT record FROM entry_overrides WHERE dictionary_id=?1 AND headword=?2" } else { "SELECT record FROM imported_entries WHERE dictionary_id=?1 AND headword=?2" };
        if let Some(json) = connection.query_row(sql, params![dictionary_id, headword], |row| row.get::<_, String>(0)).optional().map_err(db_error)? {
            return Ok(Some(Arc::new(decode_record(&json)?)));
        }
    }
    Ok(bundled.dictionaries.iter().find(|dictionary| dictionary.metadata.id == dictionary_id).and_then(|dictionary| dictionary.records.get(headword)).cloned())
}

fn history(connection: &Connection, id: &str, headword: &str, action: &str, before: Option<&EntryRecord>, after: Option<&EntryRecord>) -> AppResult<()> {
    connection.execute("INSERT INTO entry_history(dictionary_id,headword,action,before_record,after_record) VALUES(?1,?2,?3,?4,?5)", params![id, headword, action, before.map(record_json).transpose()?, after.map(record_json).transpose()?]).map_err(db_error)?;
    Ok(())
}

fn revision(connection: &Connection) -> AppResult<u64> {
    let value: i64 = connection.query_row("SELECT value FROM store_settings WHERE key='revision'", [], |row| row.get(0)).map_err(db_error)?;
    u64::try_from(value).ok().filter(|value| *value > 0).ok_or_else(database_corrupt)
}
fn rule_algorithm(connection: &Connection) -> AppResult<u8> {
    let value: i64 = connection.query_row("SELECT value FROM store_settings WHERE key='ruleAlgorithm'", [], |row| row.get(0)).map_err(db_error)?;
    u8::try_from(value).ok().filter(|value| (1..=3).contains(value)).ok_or_else(database_corrupt)
}
fn available_connection(inner: &Inner) -> AppResult<&Connection> { inner.connection.as_ref().ok_or_else(|| inner.error.clone().unwrap_or_else(database_unavailable)) }
fn validate_metadata(name: &str, language: SourceLanguage, kind: DictionaryKind) -> AppResult<()> {
    if name.trim().is_empty() { return Err(AppError::new("invalidDictionaryEntry", "Give the dictionary a name.")); }
    if (kind == DictionaryKind::Japanese && language != SourceLanguage::Ja) || (matches!(kind, DictionaryKind::HanViet | DictionaryKind::Rules | DictionaryKind::Pronouns | DictionaryKind::Ignored) && language != SourceLanguage::Zh) {
        return Err(AppError::new("invalidDictionaryEntry", "This dictionary kind is not available for the selected source language."));
    }
    Ok(())
}
fn enum_string<T: Serialize>(value: &T) -> AppResult<String> { serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_owned)).ok_or_else(database_corrupt) }
fn decode_enum<T: for<'de> Deserialize<'de>>(value: &str) -> AppResult<T> { serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(|_| database_corrupt()) }
fn record_json(record: &EntryRecord) -> AppResult<String> { serde_json::to_string(record).map_err(|_| database_corrupt()) }
fn decode_record(json: &str) -> AppResult<EntryRecord> { serde_json::from_str(json).map_err(|_| database_corrupt()) }
fn not_found() -> AppError { AppError::new("dictionaryNotFound", "The selected dictionary does not exist; refresh the dictionary list.") }
fn invalid_preview() -> AppError { AppError::new("invalidImportPreview", "The import preview expired or was replaced; preview the selected files again before committing.") }
fn database_unavailable() -> AppError { AppError::new("dictionaryDatabaseUnavailable", "The local dictionary database is unavailable. Preserve it, choose an explicit backup/new database or restore a known-good backup, then reimport your dictionaries.") }
fn database_corrupt() -> AppError { AppError::new("dictionaryDatabaseCorrupt", "The local dictionary database is damaged or contains invalid records. Its files were preserved. Choose an explicit backup/new database, or restore a known-good backup; no data was deleted.") }
fn db_error(error: rusqlite::Error) -> AppError {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) if matches!(failure.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => database_corrupt(),
        rusqlite::Error::SqliteFailure(failure, _) if matches!(failure.code, rusqlite::ErrorCode::CannotOpen | rusqlite::ErrorCode::ReadOnly | rusqlite::ErrorCode::SystemIoFailure | rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DiskFull) => database_unavailable(),
        _ => database_corrupt(),
    }
}
fn same_path(left: &Path, right: &Path) -> bool {
    if left == right { return true; }
    match (fs::canonicalize(left), fs::canonicalize(right)) { (Ok(left), Ok(right)) => left == right, _ => false }
}
fn reject_import_destination(inner: &Inner, destination: &str) -> AppResult<()> {
    let destination = Path::new(destination);
    if ["", "-wal", "-shm"].iter().any(|suffix| same_path(destination, Path::new(&format!("{}{}", inner.path.to_string_lossy(), suffix)))) {
        return Err(AppError::new("invalidDestination", "A dictionary export cannot overwrite the database or its journal."));
    }
    let selection = inner.path.parent().unwrap_or_else(|| Path::new(".")).join("dictionary-store.json");
    if same_path(destination, &selection) { return Err(AppError::new("invalidDestination", "An export cannot overwrite the database selection settings.")); }
    if let Some(connection) = &inner.connection {
        if all_metadata(connection)?.iter().filter_map(|metadata| metadata.source_path.as_ref()).any(|path| same_path(destination, Path::new(path))) {
            return Err(AppError::new("invalidDestination", "Choose a different export destination; original imports are not overwritten."));
        }
    }
    Ok(())
}

fn preserved_copy(source: &Path, destination: &Path) -> AppResult<()> {
    let directory = destination.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new().prefix(".quicktranslator-backup-").tempfile_in(directory)
        .map_err(|error| AppError::io("saveFailed", "The backup could not be safely created; originals were retained", &error))?;
    let mut original = fs::File::open(source).map_err(|error| AppError::io("dictionaryDatabaseUnavailable", "The original database could not be read; originals were retained", &error))?;
    std::io::copy(&mut original, temporary.as_file_mut()).map_err(|error| AppError::io("saveFailed", "The backup copy could not be completed; originals were retained", &error))?;
    temporary.as_file_mut().flush().map_err(|error| AppError::io("saveFailed", "The backup could not be flushed; originals were retained", &error))?;
    temporary.as_file().sync_all().map_err(|error| AppError::io("saveFailed", "The backup could not be flushed; originals were retained", &error))?;
    temporary.persist_noclobber(destination).map_err(|failure| AppError::io("saveFailed", "The backup destination could not be committed without overwriting; originals were retained", &failure.error))?;
    Ok(())
}

fn validate_schema(connection: &Connection) -> AppResult<()> {
    for sql in [
        "SELECT id,name,language,kind,bundled,source_path,encoding,format FROM dictionary_metadata LIMIT 0",
        "SELECT dictionary_id,headword,record FROM imported_entries LIMIT 0",
        "SELECT dictionary_id,headword,record FROM entry_overrides LIMIT 0",
        "SELECT dictionary_id,headword FROM entry_tombstones LIMIT 0",
        "SELECT id,dictionary_id,headword,action,timestamp,before_record,after_record FROM entry_history LIMIT 0",
        "SELECT key,value FROM store_settings LIMIT 0",
        "SELECT key,value FROM shortcut_imported LIMIT 0",
        "SELECT key,value FROM shortcut_overrides LIMIT 0",
        "SELECT key FROM shortcut_tombstones LIMIT 0",
    ] { connection.prepare(sql).map_err(db_error)?; }
    let orphaned: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)", [], |row| row.get(0)).map_err(db_error)?;
    if orphaned { return Err(database_corrupt()); }
    Ok(())
}
