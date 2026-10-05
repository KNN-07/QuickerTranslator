use std::{fmt, io};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceLanguage {
    #[default]
    Zh,
    Ja,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetLanguage {
    #[default]
    Vi,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLocale {
    #[default]
    Vi,
    En,
}

/// Half-open UTF-16 code-unit offsets. OffsetMap validates scalar boundaries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    pub fn len(self) -> Option<u32> {
        self.end.checked_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.start < self.end && other.start < other.end
            && self.start < other.end && other.start < self.end
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranslationAlgorithm {
    #[default]
    Longest,
    LeftToRight,
    LongestConditional,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FullWrap {
    #[default]
    None,
    All,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SingleWrap {
    #[default]
    None,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranslationOptions {
    pub algorithm: TranslationAlgorithm,
    pub prioritize_names: bool,
    pub full_wrap: FullWrap,
    pub single_wrap: SingleWrap,
}

impl Default for TranslationOptions {
    fn default() -> Self {
        Self {
            algorithm: TranslationAlgorithm::Longest,
            prioritize_names: true,
            full_wrap: FullWrap::None,
            single_wrap: SingleWrap::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    pub document_id: String,
    pub source_revision: u64,
    pub source_language: SourceLanguage,
    pub source_text: String,
    #[serde(default)]
    pub options: TranslationOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DictionaryKind {
    PrimaryNames,
    SecondaryNames,
    VietPhrase,
    HanViet,
    Japanese,
    Pronouns,
    Rules,
    Ignored,
    Cedict,
    Babylon,
    LacViet,
    ThieuChuu,
    Auxiliary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DictionaryLayer {
    Bundled,
    Imported,
    Edited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryProvenance {
    pub dictionary_id: String,
    pub dictionary_name: String,
    pub kind: DictionaryKind,
    pub layer: DictionaryLayer,
    pub source_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSegment {
    pub id: String,
    pub source_range: TextRange,
    pub readings_range: TextRange,
    pub phrases_range: TextRange,
    pub single_meaning_range: TextRange,
    pub surface: String,
    pub lemma: Option<String>,
    pub reading: Option<String>,
    pub part_of_speech: Option<String>,
    pub meanings: Vec<String>,
    pub provenance: Vec<DictionaryProvenance>,
    pub unknown: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationResult {
    pub document_id: String,
    pub source_revision: u64,
    pub dictionary_revision: u64,
    pub readings: String,
    pub phrases: String,
    pub single_meaning: String,
    pub segments: Vec<TranslationSegment>,
}

/// A deliberately small, secret-free error contract for native commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn io(code: &str, context: &str, error: &io::Error) -> Self {
        // An OS error's Display value can include caller-controlled paths or data.
        Self::new(code, format!("{context} ({:?}).", error.kind()))
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentWindow {
    pub document_id: String,
    pub window_label: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FoundationStatus {
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationHealth {
    pub version: String,
    pub status: FoundationStatus,
    pub document_window: DocumentWindow,
    pub source_revision: u64,
    pub target_revision: u64,
    pub dictionary_revision: Option<u64>,
    pub dictionary_ready: bool,
    pub tokenizer_ready: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_meaning_wrap_rejects_an_ambiguous_only_mode() {
        assert!(serde_json::from_value::<SingleWrap>(serde_json::json!("ambiguous")).is_err());
    }

    #[test]
    fn range_overlap_is_half_open() {
        assert!(TextRange { start: 0, end: 2 }.overlaps(TextRange { start: 1, end: 3 }));
        assert!(!TextRange { start: 0, end: 2 }.overlaps(TextRange { start: 2, end: 4 }));
        assert!(!TextRange { start: 1, end: 1 }.overlaps(TextRange { start: 0, end: 2 }));
        assert_eq!(TextRange { start: 3, end: 1 }.len(), None);
    }
}
