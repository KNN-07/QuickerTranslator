use std::{collections::{BTreeMap, HashMap}, sync::{Arc, LazyLock}};


use crate::{models::{AppError, AppResult, DictionaryKind, DictionaryLayer, DictionaryProvenance, SourceLanguage}, state::DictionaryEntry};
use super::types::{DictionaryMetadata, EntryRecord};

pub struct BundledDictionary {
    pub metadata: DictionaryMetadata,
    pub records: BTreeMap<String, Arc<EntryRecord>>,
    pub resolved_entries: BTreeMap<String, Arc<DictionaryEntry>>,
    pub index: HashMap<String, Arc<DictionaryEntry>>,
}

pub struct BundledData {
    pub dictionaries: Vec<BundledDictionary>,
    pub manifest: serde_json::Value,
    pub attribution: String,
    pub licenses: HashMap<String, String>,
}

static BUNDLED: LazyLock<AppResult<Arc<BundledData>>> = LazyLock::new(load);

pub fn bundled_data() -> AppResult<Arc<BundledData>> { BUNDLED.clone() }

#[cfg(feature = "bundled-dictionaries")]
fn load() -> AppResult<Arc<BundledData>> {
    let manifest: serde_json::Value = serde_json::from_str(include_str!("../../resources/dictionaries/manifest.json")).map_err(|_| resource_error())?;
    if manifest["format"] != "quicktranslator-dictionaries" || manifest["version"] != 1 { return Err(resource_error()); }
    let datasets = [
        ("zh-vi.jsonl", "bundled:zh", "Vietnamese Wiktionary · Chinese", SourceLanguage::Zh, DictionaryKind::VietPhrase, include_bytes!("../../resources/dictionaries/zh-vi.jsonl").as_slice()),
        ("ja-vi.jsonl", "bundled:ja", "Vietnamese Wiktionary · Japanese", SourceLanguage::Ja, DictionaryKind::Japanese, include_bytes!("../../resources/dictionaries/ja-vi.jsonl").as_slice()),
        ("han-viet.jsonl", "bundled:han", "Unicode Unihan · Hán Việt", SourceLanguage::Zh, DictionaryKind::HanViet, include_bytes!("../../resources/dictionaries/han-viet.jsonl").as_slice()),
    ];
    let mut dictionaries = Vec::new();
    let resources = manifest["resources"].as_array().ok_or_else(resource_error)?;
    for (file, id, name, language, kind, bytes) in datasets {
        let expected = resources.iter().find(|entry| entry["file"] == file).ok_or_else(resource_error)?;
        verify(bytes, expected)?;
        let text = std::str::from_utf8(bytes).map_err(|_| resource_error())?;
        let mut records = BTreeMap::new();
        let mut aliases = 0;
        for line in text.lines() {
            let entry: EntryRecord = serde_json::from_str(line).map_err(|_| resource_error())?;
            if entry.headword.is_empty() || entry.meanings.is_empty() { return Err(resource_error()); }
            aliases += entry.aliases.len();
            if records.insert(entry.headword.clone(), Arc::new(entry)).is_some() { return Err(resource_error()); }
        }
        if expected["entryCount"].as_u64() != Some(records.len() as u64) || expected["aliasCount"].as_u64() != Some(aliases as u64) { return Err(resource_error()); }
        let resolved_entries: BTreeMap<String, Arc<DictionaryEntry>> = records.iter().map(|(key, record)| (key.clone(), Arc::new(DictionaryEntry {
            headword: record.headword.clone(), meanings: record.meanings.clone(), reading: record.reading.clone(), part_of_speech: record.pos.clone(),
            provenance: vec![DictionaryProvenance { dictionary_id: id.to_owned(), dictionary_name: name.to_owned(), kind, layer: DictionaryLayer::Bundled, source_urls: record.source_urls.clone() }],
        }))).collect();
        let canonical = resolved_entries.iter().map(|(key, entry)| (key.clone(), (Arc::clone(entry), Arc::clone(&records[key])))).collect();
        let index = super::index_canonical(&canonical, None, None);
        dictionaries.push(BundledDictionary { metadata: DictionaryMetadata {
            id: id.to_owned(), name: name.to_owned(), language, kind, bundled: true, source_path: None, encoding: None, format: None,
            entry_count: records.len(), alias_count: aliases, imported_count: 0, edited_count: 0, tombstone_count: 0,
        }, records, resolved_entries, index });
    }
    let notices = [
        ("licenses/IPADIC.txt", include_bytes!("../../resources/dictionaries/licenses/IPADIC.txt").as_slice()),
        ("licenses/IPADIC-original.COPYING", include_bytes!("../../resources/dictionaries/licenses/IPADIC-original.COPYING").as_slice()),
        ("licenses/Lindera-MIT.txt", include_bytes!("../../resources/dictionaries/licenses/Lindera-MIT.txt").as_slice()),
        ("licenses/Unicode-V3.txt", include_bytes!("../../resources/dictionaries/licenses/Unicode-V3.txt").as_slice()),
        ("licenses/CC-BY-SA-4.0.txt", include_bytes!("../../resources/dictionaries/licenses/CC-BY-SA-4.0.txt").as_slice()),
        ("ATTRIBUTION.txt", include_bytes!("../../resources/dictionaries/ATTRIBUTION.txt").as_slice()),
    ];
    let expected_notices = manifest["notices"].as_array().ok_or_else(resource_error)?;
    let mut licenses = HashMap::new();
    let mut attribution = String::new();
    for (file, bytes) in notices {
        let expected = expected_notices.iter().find(|entry| entry["file"] == file).ok_or_else(resource_error)?;
        verify(bytes, expected)?;
        if file.ends_with("original.COPYING") { continue; }
        let text = std::str::from_utf8(bytes).map_err(|_| resource_error())?.to_owned();
        if file == "ATTRIBUTION.txt" { attribution = text; } else { licenses.insert(file.to_owned(), text); }
    }
    Ok(Arc::new(BundledData { dictionaries, manifest, attribution, licenses }))
}

#[cfg(not(feature = "bundled-dictionaries"))]
fn load() -> AppResult<Arc<BundledData>> { Err(resource_error()) }

fn verify(bytes: &[u8], expected: &serde_json::Value) -> AppResult<()> {
    // Embedded bytes and every recorded SHA-256 are checked by build.rs before
    // compilation. Runtime parsing verifies record sizes/counts without rehashing
    // the same immutable bytes.
    if expected["bytes"].as_u64() != Some(bytes.len() as u64) { return Err(resource_error()); }
    Ok(())
}

fn resource_error() -> AppError {
    AppError::new("dictionaryResourcesUnavailable", "Bundled dictionaries or license notices are missing or differ from the verified manifest. Reinstall the application, or run npm run data:prepare before building.")
}
