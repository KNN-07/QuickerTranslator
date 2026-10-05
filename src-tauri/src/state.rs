use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use lindera::{dictionary::load_dictionary, mode::Mode, segmenter::Segmenter};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    engine::alignment::OffsetMap,
    models::{
        AppError, AppResult, DictionaryKind, DictionaryProvenance, DocumentWindow,
        FoundationHealth, FoundationStatus, SourceLanguage, TranslationOptions,
        TranslationRequest, TranslationResult, TextRange,
    },
};

/// Resolved entries are constructed by the dictionary layer, then shared read-only.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryEntry {
    pub headword: String,
    pub meanings: Vec<String>,
    pub reading: Option<String>,
    pub part_of_speech: Option<String>,
    pub provenance: Vec<DictionaryProvenance>,
}

/// Aliases can point at the same Arc as their headword, without duplicating data.
pub type DictionaryIndex =
    HashMap<SourceLanguage, HashMap<DictionaryKind, HashMap<String, Arc<DictionaryEntry>>>>;

#[derive(Debug)]
pub struct DictionarySnapshot {
    revision: u64,
    entries: DictionaryIndex,
    rule_algorithm: u8,
    chinese_index: AppResult<crate::engine::chinese::ChineseIndex>,
    japanese_index: crate::engine::japanese::JapaneseIndex,
}

impl DictionarySnapshot {
    pub fn new(revision: u64, entries: DictionaryIndex, rule_algorithm: u8) -> Self {
        let chinese_index = crate::engine::chinese::ChineseIndex::new(&entries, rule_algorithm);
        let japanese_index = crate::engine::japanese::JapaneseIndex::new(&entries);
        Self { revision, entries, rule_algorithm, chinese_index, japanese_index }
    }

    pub fn validate_engine(&self) -> AppResult<()> {
        self.chinese_index.as_ref().map(|_| ()).map_err(Clone::clone)
    }

    pub fn chinese_index(&self) -> AppResult<&crate::engine::chinese::ChineseIndex> {
        self.chinese_index.as_ref().map_err(Clone::clone)
    }

    pub fn japanese_index(&self) -> &crate::engine::japanese::JapaneseIndex {
        &self.japanese_index
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn rule_algorithm(&self) -> u8 {
        self.rule_algorithm
    }

    pub fn entries(&self) -> &DictionaryIndex {
        &self.entries
    }

    pub fn lookup(
        &self,
        language: SourceLanguage,
        kind: DictionaryKind,
        key: &str,
    ) -> Option<&Arc<DictionaryEntry>> {
        self.entries.get(&language)?.get(&kind)?.get(key)
    }
}

/// Owns text and its offset map together so revisions cannot accidentally share a map.
#[derive(Debug)]
pub struct SourceSnapshot {
    pub revision: u64,
    pub language: SourceLanguage,
    pub text: String,
    pub offsets: OffsetMap,
}

struct ActiveTranslation {
    generation: u64,
    cancellation: CancellationToken,
}

struct WindowDocument {
    identity: DocumentWindow,
    source: Arc<SourceSnapshot>,
    target_revision: u64,
    generation: u64,
    active_translation: Option<ActiveTranslation>,
    ai_epoch: CancellationToken,
    options: TranslationOptions,
}

#[derive(Default)]
struct StateInner {
    // None means genuinely unavailable, not an empty dictionary pretending to be loaded.
    dictionaries: Option<Arc<DictionarySnapshot>>,
    windows: HashMap<String, WindowDocument>,
}

#[derive(Default)]
pub struct AppState {
    inner: Mutex<StateInner>,
    tokenizer: OnceLock<AppResult<Arc<Segmenter>>>,
}

/// An engine receives only immutable source/index/tokenizer snapshots plus cancellation.
/// The window identity is captured by the command, never supplied in TranslationRequest.
pub struct TranslationWork {
    pub document_id: String,
    pub source: Arc<SourceSnapshot>,
    pub options: TranslationOptions,
    pub dictionaries: Arc<DictionarySnapshot>,
    pub tokenizer: Option<Arc<Segmenter>>,
    pub cancellation: CancellationToken,
    window_label: String,
    generation: u64,
}

impl TranslationWork {
    /// Isolate glossary matching to the approved source span, with local scalar-safe offsets.
    pub fn fragment(&self, range: TextRange) -> AppResult<Self> {
        let bytes = self.source.offsets.utf16_range_to_byte(range)?;
        let source = if bytes.start == 0 && bytes.end == self.source.text.len() {
            Arc::clone(&self.source)
        } else {
            let text = self.source.text[bytes].to_owned();
            let offsets = OffsetMap::new(&text)?;
            Arc::new(SourceSnapshot {
                revision: self.source.revision, language: self.source.language, text, offsets,
            })
        };
        Ok(Self {
            document_id: self.document_id.clone(), source, options: self.options.clone(),
            dictionaries: Arc::clone(&self.dictionaries), tokenizer: self.tokenizer.clone(),
            cancellation: self.cancellation.clone(), window_label: self.window_label.clone(),
            generation: self.generation,
        })
    }
}

impl AppState {
    fn lock(&self) -> AppResult<MutexGuard<'_, StateInner>> {
        self.inner.lock().map_err(|_| {
            AppError::new("stateUnavailable", "Application state is unavailable; restart safely.")
        })
    }

    pub fn initialize_tokenizer(&self) -> AppResult<Arc<Segmenter>> {
        self.tokenizer
            .get_or_init(|| {
                let dictionary = load_dictionary("embedded://ipadic").map_err(|_| {
                    AppError::new("tokenizerUnavailable", "The embedded Japanese IPADIC dictionary could not be loaded.")
                })?;
                Ok(Arc::new(Segmenter::new(Mode::Normal, dictionary, None)))
            })
            .clone()
    }

    pub fn register_window(&self, window_label: &str) -> AppResult<DocumentWindow> {
        let mut inner = self.lock()?;
        if inner.windows.contains_key(window_label) {
            return Err(AppError::new("windowAlreadyRegistered", "This document window already has a session."));
        }
        let identity = DocumentWindow {
            document_id: Uuid::new_v4().to_string(),
            window_label: window_label.to_owned(),
        };
        inner.windows.insert(window_label.to_owned(), WindowDocument {
            identity: identity.clone(),
            source: Arc::new(SourceSnapshot {
                revision: 0,
                language: SourceLanguage::Zh,
                text: String::new(),
                offsets: OffsetMap::new("")?,
            }),
            target_revision: 0,
            generation: 0,
            active_translation: None,
            ai_epoch: CancellationToken::new(),
            options: TranslationOptions::default(),
        });
        Ok(identity)
    }

    pub fn remove_window(&self, window_label: &str) -> AppResult<()> {
        if let Some(document) = self.lock()?.windows.remove(window_label) {
            document.ai_epoch.cancel();
            if let Some(job) = document.active_translation {
                job.cancellation.cancel();
            }
        }
        Ok(())
    }

    pub fn document_window(&self, window_label: &str) -> AppResult<DocumentWindow> {
        self.lock()?.windows.get(window_label)
            .map(|document| document.identity.clone())
            .ok_or_else(window_unavailable)
    }

    pub fn health(&self, window_label: &str) -> AppResult<FoundationHealth> {
        let inner = self.lock()?;
        let document = inner.windows.get(window_label).ok_or_else(window_unavailable)?;
        Ok(FoundationHealth {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            status: FoundationStatus::Ready,
            document_window: document.identity.clone(),
            source_revision: document.source.revision,
            target_revision: document.target_revision,
            dictionary_revision: inner.dictionaries.as_ref().map(|snapshot| snapshot.revision()),
            dictionary_ready: inner.dictionaries.is_some(),
            tokenizer_ready: matches!(self.tokenizer.get(), Some(Ok(_))),
        })
    }

    /// Atomically publish a committed dictionary revision and invalidate old jobs in all windows.
    pub fn replace_dictionaries(&self, snapshot: DictionarySnapshot) -> AppResult<u64> {
        snapshot.validate_engine()?;
        let mut inner = self.lock()?;
        if inner.dictionaries.as_ref().is_some_and(|old| snapshot.revision() <= old.revision()) {
            return Err(AppError::new("staleDictionaryRevision", "A newer dictionary revision is already active."));
        }
        let revision = snapshot.revision();
        inner.dictionaries = Some(Arc::new(snapshot));
        for document in inner.windows.values_mut() {
            cancel_active(document);
            invalidate_ai(document);
        }
        Ok(revision)
    }

    pub fn dictionaries(&self) -> AppResult<Arc<DictionarySnapshot>> {
        self.lock()?.dictionaries.as_ref().map(Arc::clone).ok_or_else(|| {
            AppError::new("dictionariesUnavailable", "Offline dictionaries have not been loaded.")
        })
    }

    /// Capture consent data atomically without registering or cancelling an offline translation.
    pub fn snapshot_for_ai(
        &self, window_label: &str, document_id: &str, source_revision: u64,
        target_revision: u64, source_language: SourceLanguage,
    ) -> AppResult<TranslationWork> {
        let tokenizer = match source_language {
            SourceLanguage::Ja => Some(self.initialize_tokenizer()?),
            SourceLanguage::Zh => None,
        };
        let inner = self.lock()?;
        let document = checked_document(&inner, window_label, document_id)?;
        if document.source.revision != source_revision || document.source.language != source_language {
            return Err(AppError::new("staleSourceRevision", "The source changed; review the AI request again."));
        }
        if document.target_revision != target_revision {
            return Err(AppError::new("staleTargetRevision", "The Vietnamese editor changed; review the AI request again."));
        }
        let dictionaries = inner.dictionaries.as_ref().map(Arc::clone).ok_or_else(|| {
            AppError::new("dictionariesUnavailable", "Offline dictionaries have not been loaded.")
        })?;
        Ok(TranslationWork {
            document_id: document_id.to_owned(), source: Arc::clone(&document.source),
            options: document.options.clone(), dictionaries, tokenizer,
            cancellation: document.ai_epoch.child_token(),
            window_label: window_label.to_owned(), generation: document.generation,
        })
    }

    /// Called for source edits/language switches before translating. Never changes target content.
    pub fn observe_source(
        &self,
        window_label: &str,
        document_id: &str,
        revision: u64,
        language: SourceLanguage,
        text: String,
    ) -> AppResult<Arc<SourceSnapshot>> {
        {
            let inner = self.lock()?;
            let document = checked_document(&inner, window_label, document_id)?;
            if let Some(existing) = existing_source(document, revision, language, &text)? {
                return Ok(existing);
            }
        }
        // Build outside the global lock; a different document can keep editing meanwhile.
        let offsets = OffsetMap::new(&text)?;
        let snapshot = Arc::new(SourceSnapshot { revision, language, text, offsets });
        let mut inner = self.lock()?;
        let document = checked_document_mut(&mut inner, window_label, document_id)?;
        if let Some(existing) = existing_source(document, revision, language, &snapshot.text)? {
            return Ok(existing);
        }
        cancel_active(document);
        invalidate_ai(document);
        document.source = Arc::clone(&snapshot);
        Ok(snapshot)
    }

    pub fn observe_options(&self, window_label: &str, document_id: &str, source_revision: u64, options: TranslationOptions) -> AppResult<()> {
        let mut inner = self.lock()?;
        let document = checked_document_mut(&mut inner, window_label, document_id)?;
        if document.source.revision != source_revision {
            return Err(stale_translation());
        }
        if document.options != options {
            cancel_active(document);
            invalidate_ai(document);
            document.options = options;
        }
        Ok(())
    }

    pub fn observe_target(&self, window_label: &str, document_id: &str, revision: u64) -> AppResult<()> {
        let mut inner = self.lock()?;
        let document = checked_document_mut(&mut inner, window_label, document_id)?;
        if revision < document.target_revision {
            return Err(AppError::new("staleTargetRevision", "The Vietnamese editor has a newer revision."));
        }
        document.target_revision = revision;
        Ok(())
    }

    pub fn cancel_translation(&self, window_label: &str) -> AppResult<()> {
        let mut inner = self.lock()?;
        let document = inner.windows.get_mut(window_label).ok_or_else(window_unavailable)?;
        cancel_active(document);
        Ok(())
    }

    pub fn prepare_translation(&self, window_label: &str, request: TranslationRequest) -> AppResult<TranslationWork> {
        let TranslationRequest { document_id, source_revision, source_language, source_text, options } = request;
        let source = self.observe_source(window_label, &document_id, source_revision, source_language, source_text)?;
        let tokenizer = match source_language {
            SourceLanguage::Zh => None,
            SourceLanguage::Ja => Some(self.initialize_tokenizer()?),
        };
        let mut inner = self.lock()?;
        let dictionaries = inner.dictionaries.as_ref().map(Arc::clone).ok_or_else(|| {
            AppError::new("dictionariesUnavailable", "Offline dictionaries have not been loaded.")
        })?;
        let document = checked_document_mut(&mut inner, window_label, &document_id)?;
        if !Arc::ptr_eq(&source, &document.source) {
            return Err(stale_translation());
        }
        let generation = document.generation.checked_add(1).ok_or_else(|| {
            AppError::new("revisionOverflow", "This document must be reopened before another translation.")
        })?;
        cancel_active(document);
        let cancellation = CancellationToken::new();
        document.generation = generation;
        document.active_translation = Some(ActiveTranslation { generation, cancellation: cancellation.clone() });
        Ok(TranslationWork {
            document_id,
            source,
            options,
            dictionaries,
            tokenizer,
            cancellation,
            window_label: window_label.to_owned(),
            generation,
        })
    }

    /// Only the still-current window/source/dictionary generation can publish a result.
    pub fn finish_translation(&self, work: &TranslationWork, result: AppResult<TranslationResult>) -> AppResult<TranslationResult> {
        let mut inner = self.lock()?;
        let dictionary_current = inner.dictionaries.as_ref()
            .is_some_and(|snapshot| Arc::ptr_eq(snapshot, &work.dictionaries));
        let document = checked_document_mut(&mut inner, &work.window_label, &work.document_id)?;
        let job_current = document.active_translation.as_ref()
            .is_some_and(|job| job.generation == work.generation);
        if !dictionary_current || !job_current || !Arc::ptr_eq(&document.source, &work.source) {
            return Err(stale_translation());
        }
        document.active_translation = None;
        if work.cancellation.is_cancelled() {
            return Err(AppError::new("translationCancelled", "Offline translation was cancelled."));
        }
        let result = result?;
        if result.document_id != work.document_id
            || result.source_revision != work.source.revision
            || result.dictionary_revision != work.dictionaries.revision()
        {
            return Err(AppError::new("invalidTranslationResult", "The engine returned a result for a different revision."));
        }
        Ok(result)
    }
}

/// Dispatch plumbing, not a mock engine. A real engine must be explicitly supplied
/// at integration; until then no translation command is exposed to the frontend.
pub async fn dispatch_translation<F>(
    state: Arc<AppState>,
    window_label: String,
    request: TranslationRequest,
    engine: F,
) -> AppResult<TranslationResult>
where
    F: FnOnce(&TranslationWork) -> AppResult<TranslationResult> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let work = state.prepare_translation(&window_label, request)?;
        let result = if work.cancellation.is_cancelled() {
            Err(AppError::new("translationCancelled", "Offline translation was cancelled."))
        } else {
            engine(&work)
        };
        state.finish_translation(&work, result)
    }).await.map_err(|_| AppError::new("translationWorkerFailed", "The offline translation worker failed."))?
}

fn invalidate_ai(document: &mut WindowDocument) {
    document.ai_epoch.cancel();
    document.ai_epoch = CancellationToken::new();
}

fn cancel_active(document: &mut WindowDocument) {
    if let Some(job) = document.active_translation.take() {
        job.cancellation.cancel();
    }
}

fn checked_document<'a>(inner: &'a StateInner, window: &str, id: &str) -> AppResult<&'a WindowDocument> {
    let document = inner.windows.get(window).ok_or_else(window_unavailable)?;
    if document.identity.document_id != id {
        return Err(AppError::new("documentMismatch", "This document does not belong to the calling window."));
    }
    Ok(document)
}

fn checked_document_mut<'a>(inner: &'a mut StateInner, window: &str, id: &str) -> AppResult<&'a mut WindowDocument> {
    let document = inner.windows.get_mut(window).ok_or_else(window_unavailable)?;
    if document.identity.document_id != id {
        return Err(AppError::new("documentMismatch", "This document does not belong to the calling window."));
    }
    Ok(document)
}

fn existing_source(document: &WindowDocument, revision: u64, language: SourceLanguage, text: &str) -> AppResult<Option<Arc<SourceSnapshot>>> {
    if revision < document.source.revision {
        return Err(stale_translation());
    }
    if revision == document.source.revision {
        if language != document.source.language || text != document.source.text {
            return Err(AppError::new("sourceRevisionConflict", "Source text or language changed without a new revision."));
        }
        return Ok(Some(Arc::clone(&document.source)));
    }
    Ok(None)
}

fn window_unavailable() -> AppError {
    AppError::new("windowUnavailable", "The calling document window is no longer available.")
}

fn stale_translation() -> AppError {
    AppError::new("staleTranslation", "A newer source or dictionary revision superseded this translation.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(document_id: &str, revision: u64, text: &str) -> TranslationRequest {
        TranslationRequest { document_id: document_id.to_owned(), source_revision: revision, source_language: SourceLanguage::Zh, source_text: text.to_owned(), options: TranslationOptions::default() }
    }

    #[test]
    fn source_revision_cannot_reuse_different_text_or_language() {
        let state = AppState::default();
        let document = state.register_window("main").unwrap();
        let first = state.observe_source("main", &document.document_id, 1, SourceLanguage::Zh, "你🙂".into()).unwrap();
        let second = state.observe_source("main", &document.document_id, 1, SourceLanguage::Zh, "你🙂".into()).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(state.observe_source("main", &document.document_id, 1, SourceLanguage::Zh, "好".into()).is_err());
        assert!(state.observe_source("main", &document.document_id, 1, SourceLanguage::Ja, "你🙂".into()).is_err());
        assert!(state.observe_source("main", &document.document_id, 0, SourceLanguage::Zh, "".into()).is_err());
    }

    #[test]
    fn unavailable_dictionaries_never_produce_translation() {
        let state = AppState::default();
        let document = state.register_window("main").unwrap();
        assert_eq!(state.prepare_translation("main", request(&document.document_id, 1, "你好")).err().unwrap().code, "dictionariesUnavailable");
    }

    #[test]
    fn source_and_dictionary_edits_cancel_only_the_appropriate_work() {
        let state = AppState::default();
        let first = state.register_window("main").unwrap();
        let second = state.register_window("document-second").unwrap();
        state.replace_dictionaries(DictionarySnapshot::new(1, HashMap::new(), 1)).unwrap();
        let first_work = state.prepare_translation("main", request(&first.document_id, 1, "你好")).unwrap();
        let second_work = state.prepare_translation("document-second", request(&second.document_id, 1, "学校")).unwrap();
        state.observe_target("main", &first.document_id, 9).unwrap();
        state.observe_source("main", &first.document_id, 2, SourceLanguage::Zh, "好".into()).unwrap();
        assert!(first_work.cancellation.is_cancelled());
        assert!(!second_work.cancellation.is_cancelled());
        assert_eq!(state.finish_translation(&first_work, Err(AppError::new("engineError", "failure"))).unwrap_err().code, "staleTranslation");
        state.replace_dictionaries(DictionarySnapshot::new(2, HashMap::new(), 1)).unwrap();
        assert!(second_work.cancellation.is_cancelled());
        assert_eq!(state.health("main").unwrap().target_revision, 9);
        assert!(state.replace_dictionaries(DictionarySnapshot::new(1, HashMap::new(), 1)).is_err());
    }

    #[test]
    fn window_identity_cannot_be_spoofed_and_close_cancels_work() {
        let state = AppState::default();
        let first = state.register_window("main").unwrap();
        let second = state.register_window("document-second").unwrap();
        state.replace_dictionaries(DictionarySnapshot::new(1, HashMap::new(), 1)).unwrap();
        assert_eq!(state.prepare_translation("main", request(&second.document_id, 1, "你好")).err().unwrap().code, "documentMismatch");
        let work = state.prepare_translation("main", request(&first.document_id, 1, "你好")).unwrap();
        state.remove_window("main").unwrap();
        assert!(work.cancellation.is_cancelled());
        assert!(state.health("main").is_err());
        assert!(state.health("document-second").is_ok());
    }
}
