use std::{collections::{BTreeMap, HashMap, HashSet}, fs, path::{Path, PathBuf}, sync::Mutex, time::{SystemTime, UNIX_EPOCH}};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{dictionaries::encoding::decode_bytes, models::{AppError, AppResult, SourceLanguage, TargetLanguage}, storage};

pub mod schema;
pub mod rtf;
pub mod html;
pub mod export;
#[cfg(feature = "native")]
pub mod native;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftEdits {
    pub source_revision: u64,
    pub source_text: String,
    pub overrides: BTreeMap<String, String>,
    pub order: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaneFonts {
    pub source: u16, pub readings: u16, pub phrases: u16,
    pub single_meaning: u16, pub meanings: u16, pub target: u16,
}
impl Default for PaneFonts {
    fn default() -> Self { Self { source: 18, readings: 16, phrases: 16, single_meaning: 16, meanings: 14, target: 16 } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaneWraps {
    pub source: bool, pub readings: bool, pub phrases: bool,
    pub single_meaning: bool, pub meanings: bool, pub target: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewState {
    pub auto_scroll: bool,
    pub wrap: bool,
    pub fonts: PaneFonts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wraps: Option<PaneWraps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_scroll_indices: Option<[u32; 3]>,
}
impl Default for ViewState {
    fn default() -> Self { Self { auto_scroll: false, wrap: true, fonts: PaneFonts::default(), wraps: None, legacy_scroll_indices: None } }
}
impl ViewState {
    pub fn validate(&self) -> AppResult<()> {
        if [self.fonts.source, self.fonts.readings, self.fonts.phrases, self.fonts.single_meaning, self.fonts.meanings, self.fonts.target].iter().any(|size| !(8..=96).contains(size)) {
            return Err(AppError::new("invalidViewState", "Editor font sizes must be between 8 and 96 pixels."));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Project {
    pub format: String,
    pub version: u32,
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
    pub source_text: String,
    pub target_document: Value,
    pub draft_edits: DraftEdits,
    pub view_state: ViewState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_rtf: Option<String>,
}
impl Project {
    pub fn new(source_language: SourceLanguage, source_text: String) -> Self {
        Self { format: "quicktranslator-project".into(), version: 1, source_language, target_language: TargetLanguage::Vi,
            draft_edits: DraftEdits { source_revision: 0, source_text: source_text.clone(), overrides: BTreeMap::new(), order: vec![], preview_text: None },
            source_text, target_document: schema::empty_target(), view_state: ViewState::default(), legacy_rtf: None }
    }
    pub fn validate(&self) -> AppResult<()> {
        if self.format != "quicktranslator-project" { return Err(AppError::new("invalidProject", "This is not a QuickTranslator project.")); }
        if self.version != 1 { return Err(version_error(self.version)); }
        schema::validate_target(&self.target_document)?;
        self.view_state.validate()?;
        if self.draft_edits.order.iter().any(|id| id.is_empty()) || self.draft_edits.overrides.keys().any(|id| id.is_empty())
            || self.draft_edits.order.iter().collect::<HashSet<_>>().len() != self.draft_edits.order.len() {
            return Err(AppError::new("invalidDraft", "Saved draft token identifiers must be nonempty and ordered without duplicates."));
        }
        Ok(())
    }
}
fn version_error(version: u32) -> AppError {
    if version > 1 { AppError::new("futureProjectVersion", "This project was created by a newer QuickTranslator version. Upgrade the application; the original file was not changed.") }
    else { AppError::new("unsupportedProjectVersion", "This project format version is not supported. The original file was not changed.") }
}

pub fn parse_project(bytes: &[u8]) -> AppResult<Project> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value: Value = serde_json::from_slice(bytes).map_err(|_| AppError::new("invalidProject", "The project is not valid UTF-8 JSON. The current document and original file were not changed."))?;
    if let Some(version) = value.get("version").and_then(Value::as_u64) { if version != 1 { return Err(version_error(u32::try_from(version).unwrap_or(u32::MAX))); } }
    let project: Project = serde_json::from_value(value).map_err(|_| AppError::new("invalidProject", "The project has missing, invalid or unsupported fields. It was not opened or saved."))?;
    project.validate()?;
    Ok(project)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportKind { Project, LegacyQt, Text, Html }
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportRequest {
    pub path: String,
    pub source_language: SourceLanguage,
    #[serde(default)] pub encoding: Option<String>,
    #[serde(default)] pub confirm_detected_encoding: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentImportPreview {
    pub path: String,
    pub kind: ImportKind,
    pub encoding: String,
    pub detected_encoding: bool,
    pub preview_text: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ImportOutcome {
    Ready { path: String, project: Project, imported: bool, warnings: Vec<String> },
    EncodingConfirmationRequired { preview: DocumentImportPreview },
}
struct PreparedImport { preview: DocumentImportPreview, project: Project, imported: bool }

pub fn normalize_newlines(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') { chars.next(); }
            output.push('\n');
        } else { output.push(ch); }
    }
    output
}
fn extension(path: &Path) -> String { path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase() }
fn read_file(path: &Path) -> AppResult<Vec<u8>> { fs::read(path).map_err(|error| AppError::io("documentFileUnavailable", "The selected document could not be read", &error)) }
fn prepare_import(request: &ImportRequest) -> AppResult<PreparedImport> {
    let path = Path::new(&request.path);
    let kind = match extension(path).as_str() {
        "qtp" => ImportKind::Project, "qt" => ImportKind::LegacyQt,
        "txt" => ImportKind::Text, "html" | "htm" => ImportKind::Html,
        "doc" => return Err(AppError::new("legacyWordUnsupported", "Binary Word (.doc) input is not supported. Open it in a word processor and convert it to TXT or HTML before importing.")),
        "docx" => return Err(AppError::new("wordImportUnsupported", "Word documents are export-only. Convert this document to TXT or HTML before importing.")),
        _ => return Err(AppError::new("unsupportedDocument", "Choose a .qtp, .qt, .txt, .html or .htm document.")),
    };
    let bytes = read_file(path)?;
    let (project, encoding, detected, warnings, imported) = if matches!(kind, ImportKind::Project) {
        (parse_project(&bytes)?, "UTF-8".into(), false, vec![], false)
    } else {
        let decoded = decode_bytes(&bytes, request.encoding.as_deref())?;
        let (project, warnings) = match kind {
            ImportKind::LegacyQt => import_qt(&decoded.text)?,
            ImportKind::Html => (Project::new(request.source_language, html::extract_visible_text(&decoded.text)), vec![]),
            _ => {
                let text = if decoded.text.contains('\r') { normalize_newlines(&decoded.text) } else { decoded.text };
                (Project::new(request.source_language, text), vec![])
            },
        };
        (project, decoded.encoding, decoded.detected, warnings, true)
    };
    let preview_text = project.source_text.chars().take(1600).collect();
    Ok(PreparedImport { preview: DocumentImportPreview { path: request.path.clone(), kind, encoding, detected_encoding: detected, preview_text, warnings }, project, imported })
}
pub fn preview_import(request: &ImportRequest) -> AppResult<DocumentImportPreview> { Ok(prepare_import(request)?.preview) }
pub fn open_import(request: &ImportRequest) -> AppResult<ImportOutcome> {
    let prepared = prepare_import(request)?;
    if prepared.preview.detected_encoding && !request.confirm_detected_encoding {
        return Ok(ImportOutcome::EncodingConfirmationRequired { preview: prepared.preview });
    }
    Ok(ImportOutcome::Ready { path: prepared.preview.path, project: prepared.project, imported: prepared.imported, warnings: prepared.preview.warnings })
}

pub fn import_qt(input: &str) -> AppResult<(Project, Vec<String>)> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut chinese = None;
    let mut viet = None;
    let mut current = None;
    for (index, line) in lines.iter().enumerate() {
        let slot = match line.trim_end_matches('\n').trim_end_matches('\r') { "[Chinese]" => &mut chinese, "[Viet]" => &mut viet, "[CurrentLines]" => &mut current, _ => continue };
        if slot.replace(index).is_some() { return Err(qt_structure()); }
    }
    let chinese = chinese.ok_or_else(qt_structure)?;
    let viet = viet.ok_or_else(qt_structure)?;
    if viet <= chinese || (current.is_none() && chinese != 0) || (current.is_some() && (current != Some(0) || chinese != 4)) { return Err(qt_structure()); }
    let mut warnings = Vec::new();
    let mut indices = [0; 3];
    if current.is_some() {
        for (index, value) in indices.iter_mut().enumerate() {
            match lines[index + 1].trim().parse::<i32>() {
                Ok(number) if number >= 0 => *value = number as u32,
                _ => warnings.push(format!("Legacy scroll index {} was invalid and was reset to 0.", index + 1)),
            }
        }
    }
    let source_start: usize = lines[..chinese + 1].iter().map(|line| line.len()).sum();
    let viet_start: usize = source_start + lines[chinese + 1..viet].iter().map(|line| line.len()).sum::<usize>();
    let rtf_start = viet_start + lines[viet].len();
    let source = normalize_newlines(&input[source_start..viet_start]);
    let original_rtf = input[rtf_start..].to_owned();
    let parsed = rtf::parse_rtf(&original_rtf)?;
    warnings.extend(parsed.warnings);
    let mut project = Project::new(SourceLanguage::Zh, source);
    project.target_document = parsed.target_document;
    project.view_state.legacy_scroll_indices = current.map(|_| indices);
    project.legacy_rtf = Some(original_rtf);
    project.validate()?;
    Ok((project, warnings))
}
fn qt_structure() -> AppError { AppError::new("ambiguousLegacyDocument", "The .qt file must contain exactly one [Chinese] marker followed by exactly one [Viet] marker, with an optional [CurrentLines] header and three indices. Ambiguous or incomplete files were not imported.") }

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRequest { pub path: String, pub project: Project }
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult { pub path: String, pub directory_sync_confirmed: bool }
pub fn save_project(request: &SaveRequest) -> AppResult<SaveResult> {
    request.project.validate()?;
    let path = Path::new(&request.path);
    if extension(path) != "qtp" { return Err(AppError::new("projectDestinationRequired", "Save projects as .qtp. Imported .qt, text and HTML originals are never overwritten by project saves.")); }
    if path.exists() { parse_project(&read_file(path)?)?; }
    let outcome = storage::atomic_replace_json(path, &request.project)?;
    Ok(SaveResult { path: request.path.clone(), directory_sync_confirmed: outcome.directory_sync_confirmed })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat { Txt, Html, Docx, Rtf }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportColumn { Source, Readings, Phrases, SingleMeaning, Target }
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    pub path: String, pub format: ExportFormat, pub project: Project, pub columns: Vec<ExportColumn>,
    pub readings: String, pub phrases: String, pub single_meaning: String, pub blank_lines: u8,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiblingDocuments { pub previous: Option<String>, pub next: Option<String>, pub paths: Vec<String> }
pub fn sibling_documents(path: &str) -> AppResult<SiblingDocuments> {
    let current = fs::canonicalize(path).map_err(|error| AppError::io("documentFileUnavailable", "The current document path is unavailable", &error))?;
    let directory = current.parent().ok_or_else(|| AppError::new("documentFileUnavailable", "The document has no containing directory."))?;
    let entries = fs::read_dir(directory).map_err(|error| AppError::io("documentDirectoryUnavailable", "Sibling documents could not be listed", &error))?;
    let mut paths = Vec::<PathBuf>::new();
    for entry in entries {
        let entry = entry.map_err(|error| AppError::io("documentDirectoryUnavailable", "Sibling documents could not be listed", &error))?;
        let file = entry.path();
        if matches!(extension(&file).as_str(), "qtp" | "qt" | "txt" | "html" | "htm") && file.is_file() { paths.push(file); }
    }
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let index = paths.iter().position(|candidate| candidate == &current);
    let previous = index.and_then(|index| index.checked_sub(1)).map(|index| paths[index].to_string_lossy().into_owned());
    let next = index.filter(|index| index + 1 < paths.len()).map(|index| paths[index + 1].to_string_lossy().into_owned());
    Ok(SiblingDocuments { previous, next, paths: paths.into_iter().map(|path| path.to_string_lossy().into_owned()).collect() })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryRecord {
    pub document_id: String, pub name: String, pub path: Option<String>, pub updated_at: u64, pub project: Project,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryWrite { pub document_id: String, pub name: String, pub path: Option<String>, pub project: Project }
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySummary { pub document_id: String, pub name: String, pub path: Option<String>, pub updated_at: u64 }
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryList { pub records: Vec<RecoverySummary>, pub warnings: Vec<String> }

pub struct DocumentService { recovery_directory: PathBuf, owners: Mutex<HashMap<String, String>> }
impl DocumentService {
    pub fn open(app_data: &Path) -> AppResult<Self> {
        let recovery_directory = app_data.join("recovery");
        fs::create_dir_all(&recovery_directory).map_err(|error| AppError::io("recoveryUnavailable", "Unsaved-work recovery storage could not be opened", &error))?;
        Ok(Self { recovery_directory, owners: Mutex::new(HashMap::new()) })
    }
    fn recovery_path(&self, document_id: &str) -> AppResult<PathBuf> {
        let id = uuid::Uuid::parse_str(document_id).map_err(|_| AppError::new("invalidDocumentId", "Recovery requires a valid per-document UUID."))?;
        if id.to_string() != document_id { return Err(AppError::new("invalidDocumentId", "Recovery requires a canonical per-document UUID.")); }
        Ok(self.recovery_directory.join(format!("{id}.json")))
    }
    fn owner_lock(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, String>>> {
        self.owners.lock().map_err(|_| AppError::new("recoveryUnavailable", "Recovery ownership could not be accessed."))
    }
    fn check_owner(owners: &HashMap<String, String>, window: &str, id: &str) -> AppResult<()> {
        if owners.get(id).is_some_and(|owner| owner != window) { return Err(AppError::new("recoveryOwnedByAnotherWindow", "This recovery document is already open in another window.")); }
        Ok(())
    }
    fn read_record(&self, id: &str) -> AppResult<RecoveryRecord> {
        let path = self.recovery_path(id)?;
        let record: RecoveryRecord = serde_json::from_slice(&read_file(&path)?).map_err(|_| AppError::new("invalidRecovery", "The recovery file is invalid and was preserved for manual recovery."))?;
        if record.document_id != id { return Err(AppError::new("invalidRecovery", "Recovery document identity does not match its file. The file was preserved.")); }
        record.project.validate()?;
        Ok(record)
    }
    pub fn list_recovery(&self, window: &str) -> AppResult<RecoveryList> {
        let owners = self.owner_lock()?;
        let mut records = Vec::new(); let mut warnings = Vec::new();
        for entry in fs::read_dir(&self.recovery_directory).map_err(|error| AppError::io("recoveryUnavailable", "Recovery files could not be listed", &error))? {
            let entry = entry.map_err(|error| AppError::io("recoveryUnavailable", "Recovery files could not be listed", &error))?;
            let path = entry.path();
            if extension(&path) != "json" || !path.is_file() { continue; }
            let Some(id) = path.file_stem().and_then(|id| id.to_str()) else { warnings.push("An invalid recovery filename was preserved.".into()); continue; };
            if Self::check_owner(&owners, window, id).is_err() { continue; }
            match self.read_record(id) {
                Ok(record) => records.push(RecoverySummary { document_id: record.document_id, name: record.name, path: record.path, updated_at: record.updated_at }),
                Err(_) => warnings.push(format!("Recovery file {id}.json is invalid or unsupported and was preserved for manual recovery.")),
            }
        }
        records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.document_id.cmp(&b.document_id)));
        Ok(RecoveryList { records, warnings })
    }
    pub fn read_recovery(&self, window: &str, id: &str) -> AppResult<RecoveryRecord> {
        let mut owners = self.owner_lock()?;
        Self::check_owner(&owners, window, id)?;
        let record = self.read_record(id)?;
        owners.insert(id.into(), window.into());
        Ok(record)
    }
    pub fn write_recovery(&self, window: &str, request: &RecoveryWrite) -> AppResult<()> {
        request.project.validate()?;
        let path = self.recovery_path(&request.document_id)?;
        let mut owners = self.owner_lock()?;
        Self::check_owner(&owners, window, &request.document_id)?;
        if path.exists() { self.read_record(&request.document_id)?; }
        let updated_at = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| AppError::new("recoveryUnavailable", "System clock is unavailable for recovery."))?.as_secs();
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct BorrowedRecord<'a> { document_id: &'a str, name: &'a str, path: &'a Option<String>, updated_at: u64, project: &'a Project }
        let record = BorrowedRecord { document_id: &request.document_id, name: &request.name, path: &request.path, updated_at, project: &request.project };
        storage::atomic_replace_json(&path, &record)?;
        owners.insert(request.document_id.clone(), window.into());
        Ok(())
    }
    pub fn discard_recovery(&self, window: &str, id: &str) -> AppResult<()> {
        let path = self.recovery_path(id)?;
        let mut owners = self.owner_lock()?;
        Self::check_owner(&owners, window, id)?;
        match fs::remove_file(path) { Ok(()) => {}, Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}, Err(error) => return Err(AppError::io("recoveryDiscardFailed", "This document's recovery record could not be removed", &error)) }
        owners.remove(id);
        Ok(())
    }
    /// Roll back one failed recovery installation without deleting its bytes or other leases.
    pub fn release_recovery(&self, window: &str, id: &str) -> AppResult<()> {
        self.recovery_path(id)?;
        let mut owners = self.owner_lock()?;
        Self::check_owner(&owners, window, id)?;
        owners.remove(id);
        Ok(())
    }
    pub fn release_window(&self, window: &str) -> AppResult<()> { self.owner_lock()?.retain(|_, owner| owner != window); Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn path_string(path: &Path) -> String { path.to_str().unwrap().to_owned() }

    #[test]
    fn legacy_bom_crlf_import_retains_unicode_formatting_source_and_scroll_indices() {
        let fixture = include_str!("../tests/fixtures/legacy-unicode.qt");
        let fixture = format!("\u{feff}{}", fixture.replace('\n', "\r\n"));
        let (project, warnings) = import_qt(&fixture).unwrap();
        assert_eq!(project.source_text, "你好\n");
        assert_eq!(schema::plain_text(&project.target_document), "Tiếng Việt {bold}\n🙂shown");
        assert_eq!(project.view_state.legacy_scroll_indices, Some([0, 1, 2]));
        assert!(warnings.iter().any(|warning| warning.to_ascii_lowercase().contains("picture") || warning.to_ascii_lowercase().contains("pict")));
        let serialized = serde_json::to_string(&project.target_document).unwrap();
        assert!(serialized.contains("\"bold\""));
        assert!(!serialized.contains("https://invalid.test"));
        assert!(project.legacy_rtf.as_ref().unwrap().contains("HYPERLINK"));
        assert!(project.legacy_rtf.as_ref().unwrap().ends_with("\r\n"));
    }

    #[test]
    fn invalid_legacy_indices_clamp_and_ambiguous_or_malformed_imports_fail() {
        let valid_rtf = "{\\rtf1\\ansi text}";
        let (project, warnings) = import_qt(&format!("[CurrentLines]\n-1\nno\n4294967296\n[Chinese]\n你好\n[Viet]\n{valid_rtf}")).unwrap();
        assert_eq!(project.view_state.legacy_scroll_indices, Some([0, 0, 0]));
        assert_eq!(warnings.len(), 3);
        for input in [
            "[Chinese]\na\n[Viet]\n{\\rtf1 text}\n[Chinese]\nb",
            "[Chinese]\na\n[Viet]\n{\\rtf1 text}\n[Viet]\nb",
            "[Chinese]\na",
            "[Chinese]\na\n[Viet]\n{\\rtf1 broken",
            "[Chinese]\na\n[Viet]\n{\\rtf1\\u-10179?}",
        ] { assert!(import_qt(input).is_err()); }
    }

    #[test]
    fn project_roundtrip_preserves_manual_formatting_stale_draft_and_legacy_payload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.qtp");
        let (mut project, _) = import_qt(include_str!("../tests/fixtures/legacy-unicode.qt")).unwrap();
        project.draft_edits.source_revision = 7;
        project.draft_edits.source_text = "older exact source".into();
        project.draft_edits.overrides.insert("token-1".into(), "lựa chọn".into());
        project.draft_edits.order = vec!["token-1".into()];
        project.draft_edits.preview_text = Some("saved stale preview".into());
        save_project(&SaveRequest { path: path_string(&path), project: project.clone() }).unwrap();
        let actual = parse_project(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&actual).unwrap(), serde_json::to_value(&project).unwrap());
        let json = fs::read_to_string(path).unwrap();
        assert!(!json.contains("apiKey"));
        assert!(!json.contains("dictionaryDatabase"));
    }

    #[test]
    fn future_versions_invalid_nodes_and_legacy_destinations_never_overwrite_previous_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.qtp");
        let project = Project::new(SourceLanguage::Ja, "学校🙂".into());
        let request = SaveRequest { path: path_string(&path), project: project.clone() };
        save_project(&request).unwrap();
        let old = fs::read(&path).unwrap();
        let mut invalid = project.clone();
        invalid.target_document = json!({"type":"doc","content":[{"type":"image","attrs":{"src":"https://invalid.test"}}]});
        assert!(save_project(&SaveRequest { path: request.path.clone(), project: invalid }).is_err());
        assert_eq!(fs::read(&path).unwrap(), old);
        let future = br#"{"format":"quicktranslator-project","version":99}"#;
        fs::write(&path, future).unwrap();
        assert_eq!(save_project(&request).unwrap_err().code, "futureProjectVersion");
        assert_eq!(fs::read(&path).unwrap(), future);
        let legacy = directory.path().join("chapter.qt");
        fs::write(&legacy, b"original legacy").unwrap();
        assert!(save_project(&SaveRequest { path: path_string(&legacy), project }).is_err());
        assert_eq!(fs::read(&legacy).unwrap(), b"original legacy");
    }

    #[test]
    fn encoding_preview_bom_and_override_are_strict() {
        let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("source.txt");
        let text = "学校\r\nTiếng Việt 🙂";
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() { bytes.extend(unit.to_le_bytes()); }
        fs::write(&path, &bytes).unwrap();
        let request = ImportRequest { path: path_string(&path), source_language: SourceLanguage::Ja, encoding: Some("utf-8".into()), confirm_detected_encoding: false };
        let preview = preview_import(&request).unwrap();
        assert_eq!(preview.encoding, "UTF-16LE");
        assert!(!preview.detected_encoding);
        match open_import(&request).unwrap() { ImportOutcome::Ready { project, imported, .. } => { assert!(imported); assert_eq!(project.source_text, "学校\nTiếng Việt 🙂"); }, _ => panic!("BOM must not require detected-encoding confirmation") }
        let (gbk, _, had_errors) = encoding_rs::GBK.encode("你好");
        assert!(!had_errors); fs::write(&path, &gbk).unwrap();
        let mut override_request = request.clone(); override_request.encoding = Some("gbk".into());
        assert_eq!(preview_import(&override_request).unwrap().preview_text, "你好");
        override_request.encoding = Some("utf-8".into());
        assert_eq!(preview_import(&override_request).unwrap_err().code, "invalidEncoding");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(preview_import(&request).unwrap_err().code, "invalidEncoding");
    }

    #[test]
    fn recovery_rollback_releases_only_its_lease_and_preserves_both_documents() {
        let directory = tempfile::tempdir().unwrap();
        let service = DocumentService::open(directory.path()).unwrap();
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        let request = RecoveryWrite { document_id: first.into(), name: "first".into(), path: None, project: Project::new(SourceLanguage::Zh, "你好".into()) };
        service.write_recovery("one", &request).unwrap();
        let another = RecoveryWrite { document_id: second.into(), name: "second".into(), ..request };
        service.write_recovery("one", &another).unwrap();
        let bytes = fs::read(service.recovery_path(first).unwrap()).unwrap();
        assert!(service.release_recovery("two", first).is_err());
        service.release_recovery("one", first).unwrap();
        assert_eq!(fs::read(service.recovery_path(first).unwrap()).unwrap(), bytes);
        assert_eq!(service.read_recovery("two", first).unwrap().project.source_text, "你好");
        assert_eq!(service.read_recovery("two", second).unwrap_err().code, "recoveryOwnedByAnotherWindow");
        assert_eq!(service.read_recovery("one", second).unwrap().project.source_text, "你好");
    }

    #[test]
    fn recovery_is_document_scoped_claimed_by_window_and_retains_corrupt_records() {
        let directory = tempfile::tempdir().unwrap(); let service = DocumentService::open(directory.path()).unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        let other = "00000000-0000-4000-8000-000000000002";
        let request = RecoveryWrite { document_id: id.into(), name: "chapter".into(), path: None, project: Project::new(SourceLanguage::Zh, "你好".into()) };
        service.write_recovery("one", &request).unwrap();
        let mut another = request.clone(); another.document_id = other.into();
        service.write_recovery("two", &another).unwrap();
        assert_eq!(service.list_recovery("one").unwrap().records.len(), 1);
        assert!(service.read_recovery("two", id).is_err());
        assert!(service.discard_recovery("two", id).is_err());
        service.discard_recovery("one", id).unwrap();
        assert_eq!(service.read_recovery("two", other).unwrap().project.source_text, "你好");
        service.release_window("two").unwrap();
        assert_eq!(service.read_recovery("one", other).unwrap().document_id, other);
        let corrupt = directory.path().join("recovery/00000000-0000-4000-8000-000000000003.json");
        fs::write(&corrupt, b"broken unsaved work").unwrap();
        let listed = service.list_recovery("one").unwrap();
        assert_eq!(listed.warnings.len(), 1);
        assert_eq!(fs::read(corrupt).unwrap(), b"broken unsaved work");
    }

    #[test]
    fn sibling_navigation_is_sorted_supported_files_only_and_doc_has_actionable_error() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["b.txt", "a.qt", "c.html", "ignore.doc", "ignore.json"] { fs::write(directory.path().join(name), b"").unwrap(); }
        let siblings = sibling_documents(&path_string(&directory.path().join("b.txt"))).unwrap();
        assert!(siblings.previous.unwrap().ends_with("a.qt"));
        assert!(siblings.next.unwrap().ends_with("c.html"));
        assert_eq!(siblings.paths.len(), 3);
        let error = preview_import(&ImportRequest { path: path_string(&directory.path().join("ignore.doc")), source_language: SourceLanguage::Zh, encoding: None, confirm_detected_encoding: false }).unwrap_err();
        assert_eq!(error.code, "legacyWordUnsupported"); assert!(error.message.contains("TXT or HTML"));
    }
}
