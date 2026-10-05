use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::{DictionaryKind, DictionaryLayer, DictionaryProvenance, SourceLanguage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntryRecord {
    pub headword: String,
    pub meanings: Vec<String>,
    #[serde(default)]
    pub reading: Option<String>,
    #[serde(default)]
    pub pos: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub source_urls: Vec<String>,
    /// Original auxiliary/legacy value, never interpreted as markup.
    #[serde(default)]
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImportFormat { Legacy, Cedict, Ignored }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub path: String,
    pub name: String,
    pub language: SourceLanguage,
    pub kind: DictionaryKind,
    #[serde(default)]
    pub dictionary_id: Option<String>,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub format: Option<ImportFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportIssue { pub line: usize, pub code: String, pub message: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub preview_id: String,
    pub dictionary_id: String,
    pub name: String,
    pub path: String,
    pub language: SourceLanguage,
    pub kind: DictionaryKind,
    pub encoding: String,
    pub encoding_detected: bool,
    pub decoded_text: String,
    pub accepted: usize,
    pub duplicates: usize,
    pub malformed: usize,
    pub issues: Vec<ImportIssue>,
    pub sample: Vec<EntryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitImportRequest {
    pub preview_ids: Vec<String>,
    #[serde(default)]
    pub rule_algorithm: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryMetadata {
    pub id: String,
    pub name: String,
    pub language: SourceLanguage,
    pub kind: DictionaryKind,
    pub bundled: bool,
    pub source_path: Option<String>,
    pub encoding: Option<String>,
    pub format: Option<ImportFormat>,
    pub entry_count: usize,
    pub alias_count: usize,
    pub imported_count: usize,
    pub edited_count: usize,
    pub tombstone_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryCatalog {
    pub revision: u64,
    pub rule_algorithm: u8,
    pub dictionaries: Vec<DictionaryMetadata>,
    pub manifest: serde_json::Value,
    pub attribution: String,
    pub licenses: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataMutation {
    pub id: Option<String>,
    pub name: String,
    pub language: SourceLanguage,
    pub kind: DictionaryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryMutation { pub dictionary_id: String, pub entry: EntryRecord }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryKey { pub dictionary_id: String, pub headword: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub dictionary_id: Option<String>,
    pub language: Option<SourceLanguage>,
    pub kind: Option<DictionaryKind>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_limit() -> usize { 100 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub dictionary_id: String,
    pub language: SourceLanguage,
    pub kind: DictionaryKind,
    pub entry: EntryRecord,
    pub layer: DictionaryLayer,
    pub provenance: Vec<DictionaryProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult { pub total: usize, pub entries: Vec<EntryView> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryHistory {
    pub id: i64,
    pub action: String,
    pub timestamp: String,
    pub before: Option<EntryRecord>,
    pub after: Option<EntryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest { pub dictionary_id: String, pub destination: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult { pub entries: usize, pub directory_sync_confirmed: bool }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequest {
    pub path: String,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub remappings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDictionary {
    pub key: String,
    pub kind: DictionaryKind,
    pub original_path: String,
    pub resolved_path: Option<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPreview {
    pub encoding: String,
    pub encoding_detected: bool,
    pub decoded_text: String,
    pub rule_algorithm: u8,
    pub dictionaries: Vec<ConfigDictionary>,
    pub issues: Vec<ImportIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutRecord { pub key: String, pub value: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutImportRequest { pub path: String, pub encoding: Option<String> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutPreview {
    pub preview_id: String,
    pub encoding: String,
    pub encoding_detected: bool,
    pub decoded_text: String,
    pub accepted: usize,
    pub duplicates: usize,
    pub malformed: usize,
    pub issues: Vec<ImportIssue>,
    pub sample: Vec<ShortcutRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryStorageStatus {
    pub ready: bool,
    pub path: String,
    pub error: Option<crate::models::AppError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairDatabaseRequest { pub backup_destination: String }
