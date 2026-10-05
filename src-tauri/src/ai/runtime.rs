use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};

use parking_lot::{Mutex, MutexGuard};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::{
    models::{AppError, AppResult, SourceLanguage, TextRange},
    state::{AppState, TranslationWork},
};

use super::{
    AiEvent, AiMode, AiProfile, AiProtocol, AiScope, AiTranslationRequest, AiUsage,
    DeltaSink, ProviderOutput, ProviderPrompt,
    profiles::ProfileService,
    prompt::{self, PromptPlan},
    transport,
};

/// A channel boundary, also usable by native probes without a webview. It never
/// owns or modifies a target editor and must not log event payloads.
pub type EventHandler = Arc<dyn Fn(AiEvent) -> AppResult<()> + Send + Sync>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPayloadChunk {
    pub chunk_index: usize,
    pub source_range: TextRange,
    pub source_text: String,
    pub system: String,
    pub user: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPayloadPreview {
    pub profile_id: String,
    pub request_url: String,
    pub endpoint_identity: String,
    pub host: String,
    pub model: String,
    pub mode: AiMode,
    pub scope: AiScope,
    pub source_language: SourceLanguage,
    pub source_range: TextRange,
    pub source_text: String,
    pub target_text: Option<String>,
    pub chunk_count: usize,
    pub chunks: Vec<AiPayloadChunk>,
}

struct ReviewedPayload {
    request: AiTranslationRequest,
    profile: AiProfile,
    work: TranslationWork,
    plan: PromptPlan,
}

#[derive(Default)]
struct Jobs {
    active: HashMap<String, Arc<JobControl>>,
    pending_cancellations: HashMap<String, HashSet<String>>,
    reviewed: HashMap<String, ReviewedPayload>,
    // Native document labels are unique. A destroyed label must never acquire
    // a late registration after its state and pending cancellations are cleared.
    destroyed_windows: HashSet<String>,
}

/// Managed once for the application: one HTTP client and isolated window jobs.
pub struct JobRuntime {
    client: reqwest::Client,
    jobs: Mutex<Jobs>,
}

#[derive(Default)]
struct EventState {
    terminal: bool,
    chunk_index: Option<usize>,
}

struct JobControl {
    id: String,
    cancellation: CancellationToken,
    invalidation: OnceLock<CancellationToken>,
    events: Mutex<EventState>,
    on_event: EventHandler,
}

impl JobControl {
    fn cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
            || self.invalidation.get().is_some_and(CancellationToken::is_cancelled)
    }

    fn cancel(&self) {
        // Serialize explicit cancellation with channel emission, so a queued
        // callback cannot publish a delta after this cancellation is acknowledged.
        let _events = self.events.lock();
        self.cancellation.cancel();
    }

    fn emit(&self, event: AiEvent) -> AppResult<()> {
        let mut events = self.events.lock();
        if events.terminal || self.cancelled() {
            return Err(cancelled_error());
        }
        if let AiEvent::ChunkStarted { chunk_index, .. } = &event {
            events.chunk_index = Some(*chunk_index);
        }
        if (self.on_event)(event).is_err() {
            self.cancellation.cancel();
            return Err(AppError::new("ai_channel_closed", "The AI preview channel is no longer available."));
        }
        Ok(())
    }

    fn terminal(&self, result: AppResult<ProviderOutput>) {
        let mut events = self.events.lock();
        if events.terminal {
            return;
        }
        // Commit terminal ownership before attempting IPC. A failed/disconnected
        // channel cannot be repaired by sending a second, contradictory terminal.
        events.terminal = true;
        let event = if self.cancelled() || result.as_ref().is_err_and(|error| error.code == "ai_cancelled") {
            AiEvent::Cancelled { job_id: self.id.clone() }
        } else {
            match result {
                Ok(output) => AiEvent::Completed {
                    job_id: self.id.clone(), text: output.text, usage: output.usage,
                },
                Err(error) => AiEvent::Error {
                    job_id: self.id.clone(), code: error.code, message: error.message,
                    chunk_index: events.chunk_index,
                },
            }
        };
        let _ = (self.on_event)(event);
    }
}

/// RAII ownership guarantees one terminal attempt even if a native task is
/// dropped or unwinds. Cleanup cannot remove a different window generation.
struct RegisteredJob {
    runtime: Arc<JobRuntime>,
    window_label: String,
    control: Arc<JobControl>,
}

impl Drop for RegisteredJob {
    fn drop(&mut self) {
        self.control.terminal(Err(AppError::new(
            "ai_interrupted", "The AI operation ended before a complete response was received.",
        )));
        self.control.cancellation.cancel();
        let mut jobs = self.runtime.jobs.lock();
        if jobs.active.get(&self.window_label).is_some_and(|active| Arc::ptr_eq(active, &self.control)) {
            jobs.active.remove(&self.window_label);
        }
    }
}

/// Connection tests reserve the same window slot and use the same cancellation
/// lifecycle as document jobs, without publishing document-preview events.
pub struct TestJob {
    job: RegisteredJob,
}

impl TestJob {
    pub fn cancellation(&self) -> &CancellationToken {
        &self.job.control.cancellation
    }

    pub fn finish(self, result: AppResult<ProviderOutput>) -> AppResult<ProviderOutput> {
        // This reservation has no preview channel; its sole terminal surface is
        // the command result. Keep the real output in that result without a copy.
        let mut events = self.job.control.events.lock();
        events.terminal = true;
        if self.job.control.cancelled() {
            Err(cancelled_error())
        } else {
            result
        }
    }
}


impl JobRuntime {
    pub fn new() -> AppResult<Self> {
        Ok(Self { client: transport::http_client()?, jobs: Mutex::new(Jobs::default()) })
    }

    pub fn begin_test(self: &Arc<Self>, window_label: &str, job_id: &str) -> AppResult<TestJob> {
        let silent_handler: EventHandler = Arc::new(|_| Ok(()));
        Ok(TestJob { job: self.register(window_label, job_id, silent_handler)? })
    }
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn lock(&self) -> MutexGuard<'_, Jobs> {
        self.jobs.lock()
    }

    fn register(self: &Arc<Self>, window_label: &str, job_id: &str, on_event: EventHandler) -> AppResult<RegisteredJob> {
        validate_job_id(job_id)?;
        let mut jobs = self.lock();
        if jobs.destroyed_windows.contains(window_label) {
            return Err(AppError::new("windowUnavailable", "The calling document window is no longer available."));
        }
        if jobs.active.contains_key(window_label) {
            return Err(AppError::new("ai_job_active", "Cancel the active AI operation before starting another one."));
        }
        let control = Arc::new(JobControl {
            id: job_id.to_owned(), cancellation: CancellationToken::new(),
            invalidation: OnceLock::new(), events: Mutex::new(EventState::default()), on_event,
        });
        if let Some(pending) = jobs.pending_cancellations.get_mut(window_label) {
            if pending.remove(job_id) {
                control.cancellation.cancel();
            }
            if pending.is_empty() {
                jobs.pending_cancellations.remove(window_label);
            }
        }
        jobs.active.insert(window_label.to_owned(), Arc::clone(&control));
        Ok(RegisteredJob { runtime: Arc::clone(self), window_label: window_label.to_owned(), control })
    }

    /// Cancellation may arrive before start registration. It is remembered only
    /// for this native window and consumed by that exact job, before key or HTTP IO.
    pub fn cancel(&self, window_label: &str, job_id: &str) -> AppResult<()> {
        validate_job_id(job_id)?;
        let mut jobs = self.lock();
        if jobs.destroyed_windows.contains(window_label) {
            return Ok(());
        }
        if let Some(active) = jobs.active.get(window_label).filter(|active| active.id == job_id) {
            active.cancel();
        } else {
            jobs.pending_cancellations.entry(window_label.to_owned()).or_default().insert(job_id.to_owned());
        }
        Ok(())
    }

    pub fn remove_window(&self, window_label: &str) {
        let mut jobs = self.jobs.lock();
        jobs.destroyed_windows.insert(window_label.to_owned());
        jobs.reviewed.remove(window_label);
        jobs.pending_cancellations.remove(window_label);
        if let Some(job) = jobs.active.remove(window_label) {
            job.cancel();
        }
    }

    /// The preview is the actual immutable payload, not an approximate JS prompt.
    /// One reviewed payload is retained per window and consumed once by start.
    pub async fn preview(
        self: Arc<Self>, state: Arc<AppState>, profiles: Arc<ProfileService>,
        window_label: String, request: AiTranslationRequest,
    ) -> AppResult<AiPayloadPreview> {
        validate_job_id(&request.job_id)?;
        let preparation_label = window_label.clone();
        let (reviewed, preview) = tokio::task::spawn_blocking(move || {
            let work = state.snapshot_for_ai(
                &preparation_label, &request.document_id, request.source_revision,
                request.target_revision, request.source_language,
            )?;
            prompt::validate_request(&request, &work.source)?;
            let profile = profiles.get(&request.profile_id)?;
            let request_url = transport::request_url(&profile)?;
            let plan = prompt::prepare(&request, &work)?;
            let preview = AiPayloadPreview {
                profile_id: profile.id.clone(), request_url: request_url.to_string(),
                endpoint_identity: super::profiles::endpoint_identity(&profile)?,
                host: request_url.host_str().unwrap_or_default().to_owned(),
                model: profile.model.clone(), mode: request.mode, scope: request.scope,
                source_language: request.source_language, source_range: request.source_range,
                source_text: request.source_text.clone(), target_text: request.target_text.clone(),
                chunk_count: plan.chunks.len(),
                chunks: plan.chunks.iter().enumerate().map(|(index, chunk)| AiPayloadChunk {
                    chunk_index: index, source_range: chunk.source_range,
                    source_text: chunk.source_text().to_owned(),
                    system: chunk.prompt.system.clone(), user: chunk.prompt.user.clone(),
                }).collect(),
            };
            Ok::<_, AppError>((ReviewedPayload { request, profile, work, plan }, preview))
        }).await.map_err(|_| AppError::new("ai_interrupted", "The AI payload preview could not be prepared."))??;
        if reviewed.work.cancellation.is_cancelled() {
            return Err(stale_preview());
        }
        let mut jobs = self.lock();
        if jobs.destroyed_windows.contains(&window_label) {
            return Err(AppError::new("windowUnavailable", "The calling document window is no longer available."));
        }
        jobs.reviewed.insert(window_label, reviewed);
        Ok(preview)
    }

    /// Before registration, invocation failures are ordinary command errors.
    /// After registration all outcomes are terminal channel events and Ok(()),
    /// preventing the frontend from presenting a duplicate error/completion.
    pub async fn start(
        self: Arc<Self>, state: Arc<AppState>, profiles: Arc<ProfileService>,
        window_label: String, request: AiTranslationRequest, on_event: EventHandler,
    ) -> AppResult<()> {
        state.document_window(&window_label)?;
        let job = self.register(&window_label, &request.job_id, on_event)?;
        let result = self.run(&job, state, profiles, request).await;
        job.control.terminal(result);
        Ok(())
    }

    async fn run(
        &self, job: &RegisteredJob, state: Arc<AppState>, profiles: Arc<ProfileService>,
        request: AiTranslationRequest,
    ) -> AppResult<ProviderOutput> {
        if job.control.cancelled() {
            // Do not resolve the profile, unlock a keychain, prepare prompts or
            // open HTTP when cancel was acknowledged before registration.
            self.lock().reviewed.remove(&job.window_label);
            return Err(cancelled_error());
        }
        let reviewed = self.lock().reviewed.remove(&job.window_label).ok_or_else(|| {
            AppError::new("ai_preview_required", "Review the actual AI payload before sending it.")
        })?;
        if !same_request(&reviewed.request, &request) {
            return Err(stale_preview());
        }
        let _ = job.control.invalidation.set(reviewed.work.cancellation.clone());
        if job.control.cancelled() {
            return Err(cancelled_error());
        }
        let label = job.window_label.clone();
        let expected_source = Arc::clone(&reviewed.work.source);
        let expected_dictionaries = Arc::clone(&reviewed.work.dictionaries);
        let expected_profile = reviewed.profile;
        let resolution = tokio::task::spawn_blocking(move || {
            let current = state.snapshot_for_ai(
                &label, &request.document_id, request.source_revision,
                request.target_revision, request.source_language,
            )?;
            prompt::validate_request(&request, &current.source)?;
            if !Arc::ptr_eq(&current.source, &expected_source)
                || !Arc::ptr_eq(&current.dictionaries, &expected_dictionaries)
            {
                return Err(stale_preview());
            }
            if !same_profile(&expected_profile, &profiles.get(&request.profile_id)?) {
                return Err(stale_preview());
            }
            let resolved = profiles.resolve_profile_and_credential(&request.profile_id)?;
            if !same_profile(&expected_profile, &resolved.0) {
                return Err(stale_preview());
            }
            Ok(resolved)
        });
        let (profile, credential) = tokio::select! {
            biased;
            _ = job.control.cancellation.cancelled() => return Err(cancelled_error()),
            _ = reviewed.work.cancellation.cancelled() => return Err(cancelled_error()),
            result = resolution => result.map_err(|_| AppError::new("ai_interrupted", "The AI profile could not be resolved."))??,
        };
        job.control.emit(AiEvent::Started {
            job_id: job.control.id.clone(), chunk_count: reviewed.plan.chunks.len(),
        })?;
        let mut text = reviewed.plan.leading_delimiter;
        let mut usage = None;
        for (chunk_index, chunk) in reviewed.plan.chunks.into_iter().enumerate() {
            job.control.emit(AiEvent::ChunkStarted {
                job_id: job.control.id.clone(), chunk_index, source_range: chunk.source_range,
            })?;
            let mut on_delta = |delta: &str| job.control.emit(AiEvent::Delta {
                job_id: job.control.id.clone(), chunk_index, text: delta.to_owned(),
            });
            let output = tokio::select! {
                biased;
                _ = job.control.cancellation.cancelled() => return Err(cancelled_error()),
                _ = reviewed.work.cancellation.cancelled() => return Err(cancelled_error()),
                result = translate_profile(
                    &self.client, &profile, credential.as_ref().map(|secret| secret.expose_secret()),
                    &chunk.prompt, &job.control.cancellation, &mut on_delta,
                ) => result?,
            };
            // Cancellation wins even if the final provider bytes arrived in the
            // same scheduling turn. No stale chunk/full completion is published.
            if job.control.cancelled() {
                return Err(cancelled_error());
            }
            add_usage(&mut usage, output.usage.as_ref());
            text.push_str(&output.text);
            text.push_str(&chunk.delimiter_after);
            job.control.emit(AiEvent::ChunkCompleted {
                job_id: job.control.id.clone(), chunk_index, text: output.text, usage: output.usage,
            })?;
        }
        Ok(ProviderOutput { text, usage })
    }
}

/// All runtime and fixed-cost connection tests dispatch the same real adapters.
/// No provider, model, endpoint, mode fallback or automatic retry is performed.
pub async fn translate_profile(
    client: &reqwest::Client, profile: &AiProfile, credential: Option<&str>,
    prompt: &ProviderPrompt, cancel: &CancellationToken, on_delta: &mut DeltaSink<'_>,
) -> AppResult<ProviderOutput> {
    match profile.protocol {
        AiProtocol::OpenaiResponses => super::openai::translate_responses(client, profile, credential, prompt, cancel, on_delta).await,
        AiProtocol::OpenaiChat => super::openai::translate_chat(client, profile, credential, prompt, cancel, on_delta).await,
        AiProtocol::Gemini => super::gemini::translate(client, profile, credential, prompt, cancel, on_delta).await,
        AiProtocol::Anthropic => super::anthropic::translate(client, profile, credential, prompt, cancel, on_delta).await,
    }
}

fn add_usage(total: &mut Option<AiUsage>, reported: Option<&AiUsage>) {
    let Some(reported) = reported else { return; };
    let total = total.get_or_insert_with(AiUsage::default);
    if let Some(count) = reported.input_tokens {
        total.input_tokens = Some(total.input_tokens.unwrap_or(0).saturating_add(count));
    }
    if let Some(count) = reported.output_tokens {
        total.output_tokens = Some(total.output_tokens.unwrap_or(0).saturating_add(count));
    }
}

fn same_request(left: &AiTranslationRequest, right: &AiTranslationRequest) -> bool {
    left.job_id == right.job_id && left.document_id == right.document_id
        && left.source_revision == right.source_revision && left.target_revision == right.target_revision
        && left.profile_id == right.profile_id && left.mode == right.mode
        && left.source_language == right.source_language && left.scope == right.scope
        && left.source_range == right.source_range && left.source_text == right.source_text
        && left.target_text == right.target_text && left.instructions == right.instructions
}

fn same_profile(left: &AiProfile, right: &AiProfile) -> bool {
    left.id == right.id && left.name == right.name && left.protocol == right.protocol
        && left.base_url == right.base_url && left.model == right.model && left.stream == right.stream
        && left.max_output_tokens == right.max_output_tokens && left.auth_mode == right.auth_mode
        && left.allow_insecure_http == right.allow_insecure_http && left.token_limit_field == right.token_limit_field
}

fn validate_job_id(job_id: &str) -> AppResult<()> {
    if job_id.is_empty() || job_id.len() > 256 || job_id.chars().any(char::is_control) {
        return Err(AppError::new("ai_invalid_request", "A valid AI job identifier is required."));
    }
    Ok(())
}

fn cancelled_error() -> AppError {
    AppError::new("ai_cancelled", "The AI operation was cancelled.")
}

fn stale_preview() -> AppError {
    AppError::new("ai_preview_stale", "The document, profile or payload changed. Review it again before sending.")
}


#[cfg(feature = "native")]
#[tauri::command]
pub async fn preview_ai_translation(
    request: AiTranslationRequest,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    runtime: tauri::State<'_, Arc<JobRuntime>>,
    profiles: tauri::State<'_, Arc<ProfileService>>,
) -> AppResult<AiPayloadPreview> {
    Arc::clone(runtime.inner()).preview(
        Arc::clone(state.inner()), Arc::clone(profiles.inner()), window.label().to_owned(), request,
    ).await
}

#[cfg(feature = "native")]
#[tauri::command]
pub async fn start_ai_translation(
    request: AiTranslationRequest,
    on_event: tauri::ipc::Channel<AiEvent>,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    runtime: tauri::State<'_, Arc<JobRuntime>>,
    profiles: tauri::State<'_, Arc<ProfileService>>,
) -> AppResult<()> {
    let handler: EventHandler = Arc::new(move |event| {
        on_event.send(event).map_err(|_| AppError::new("ai_channel_closed", "The AI preview channel is no longer available."))
    });
    Arc::clone(runtime.inner()).start(
        Arc::clone(state.inner()), Arc::clone(profiles.inner()), window.label().to_owned(), request, handler,
    ).await
}

#[cfg(feature = "native")]
#[tauri::command]
pub fn cancel_ai_translation(
    job_id: String,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    runtime: tauri::State<'_, Arc<JobRuntime>>,
) -> AppResult<()> {
    state.document_window(window.label())?;
    runtime.cancel(window.label(), &job_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{settings::SettingsStore, state::DictionarySnapshot};
    use serde_json::{Value, json};

    fn runtime() -> Arc<JobRuntime> {
        Arc::new(JobRuntime::new().unwrap())
    }

    fn events() -> (EventHandler, Arc<Mutex<Vec<Value>>>) {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        let handler: EventHandler = Arc::new(move |event| {
            sink.lock().push(serde_json::to_value(event).unwrap());
            Ok(())
        });
        (handler, collected)
    }

    fn request(document_id: &str, job_id: &str) -> AiTranslationRequest {
        AiTranslationRequest {
            job_id: job_id.into(), document_id: document_id.into(),
            source_revision: 1, target_revision: 0, profile_id: "not-configured".into(),
            mode: AiMode::Translate, source_language: SourceLanguage::Zh,
            scope: AiScope::Document, source_range: TextRange { start: 0, end: 4 },
            source_text: "你🙂好".into(), target_text: None, instructions: String::new(),
        }
    }

    fn state_with_windows() -> (Arc<AppState>, String, String) {
        let state = Arc::new(AppState::default());
        let first = state.register_window("first").unwrap().document_id;
        let second = state.register_window("second").unwrap().document_id;
        state.replace_dictionaries(DictionarySnapshot::new(1, HashMap::new(), 1)).unwrap();
        state.observe_source("first", &first, 1, SourceLanguage::Zh, "你🙂好".into()).unwrap();
        state.observe_source("second", &second, 1, SourceLanguage::Zh, "你好".into()).unwrap();
        (state, first, second)
    }

    #[test]
    fn pending_cancellation_is_exact_and_window_scoped() {
        let runtime = runtime();
        runtime.cancel("first", "cancel-before-start").unwrap();
        let (handler, collected) = events();
        let other_window = runtime.register("second", "cancel-before-start", Arc::clone(&handler)).unwrap();
        assert!(!other_window.control.cancelled());
        let unrelated = runtime.register("first", "different", Arc::clone(&handler)).unwrap();
        assert!(!unrelated.control.cancelled());
        unrelated.control.terminal(Ok(ProviderOutput { text: "done".into(), usage: None }));
        drop(unrelated);
        let pending = runtime.register("first", "cancel-before-start", handler).unwrap();
        assert!(pending.control.cancelled());
        assert!(!runtime.lock().pending_cancellations.contains_key("first"));
        pending.control.terminal(Ok(ProviderOutput { text: "must not complete".into(), usage: None }));
        drop(pending);
        assert_eq!(collected.lock().last().unwrap()["type"], "cancelled");
        assert!(!other_window.control.cancelled());
    }

    #[test]
    fn one_active_job_per_window_and_one_terminal_per_registration() {
        let runtime = runtime();
        let (handler, collected) = events();
        let first = runtime.register("first", "job-1", Arc::clone(&handler)).unwrap();
        assert_eq!(runtime.register("first", "job-2", Arc::clone(&handler)).err().unwrap().code, "ai_job_active");
        let second = runtime.register("second", "job-1", handler).unwrap();
        first.control.emit(AiEvent::ChunkStarted {
            job_id: "job-1".into(), chunk_index: 2, source_range: TextRange { start: 0, end: 1 },
        }).unwrap();
        first.control.terminal(Err(AppError::new("ai_protocol", "The provider response was invalid.")));
        first.control.terminal(Ok(ProviderOutput { text: "late".into(), usage: None }));
        assert!(first.control.emit(AiEvent::Delta {
            job_id: "job-1".into(), chunk_index: 2, text: "late".into(),
        }).is_err());
        drop(first);
        let values = collected.lock();
        assert_eq!(values.len(), 2);
        assert_eq!(values[1]["type"], "error");
        assert_eq!(values[1]["chunkIndex"], 2);
        drop(values);
        assert!(!second.control.cancelled());
        assert!(!runtime.lock().active.contains_key("first"));
    }

    #[test]
    fn dropping_an_unfinished_job_emits_one_interrupted_terminal() {
        let runtime = runtime();
        let (handler, collected) = events();
        drop(runtime.register("first", "abandoned", Arc::clone(&handler)).unwrap());
        assert_eq!(collected.lock().as_slice(), &[json!({
            "type": "error", "jobId": "abandoned", "code": "ai_interrupted",
            "message": "The AI operation ended before a complete response was received.", "chunkIndex": null,
        })]);
        let replacement = runtime.register("first", "replacement", handler).unwrap();
        replacement.control.terminal(Ok(ProviderOutput { text: "complete".into(), usage: None }));
        drop(replacement);
        assert_eq!(collected.lock().len(), 2);
    }

    #[test]
    fn cancellation_prevents_late_deltas_and_completion() {
        let runtime = runtime();
        let (handler, collected) = events();
        let job = runtime.register("first", "job", handler).unwrap();
        job.control.emit(AiEvent::Delta { job_id: "job".into(), chunk_index: 0, text: "Xin ".into() }).unwrap();
        runtime.cancel("first", "job").unwrap();
        assert!(job.control.emit(AiEvent::Delta {
            job_id: "job".into(), chunk_index: 0, text: "late".into(),
        }).is_err());
        job.control.terminal(Ok(ProviderOutput { text: "stale completion".into(), usage: None }));
        drop(job);
        let values = collected.lock();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["text"], "Xin ");
        assert_eq!(values[1]["type"], "cancelled");
    }

    #[test]
    fn window_destruction_clears_jobs_and_pending_ids_without_affecting_peers() {
        let runtime = runtime();
        let (handler, collected) = events();
        let first = runtime.register("first", "active", Arc::clone(&handler)).unwrap();
        let second = runtime.register("second", "active", Arc::clone(&handler)).unwrap();
        runtime.cancel("first", "pending").unwrap();
        runtime.remove_window("first");
        assert!(first.control.cancelled());
        assert!(!second.control.cancelled());
        let jobs = runtime.lock();
        assert!(!jobs.active.contains_key("first"));
        assert!(!jobs.pending_cancellations.contains_key("first"));
        drop(jobs);
        assert_eq!(runtime.register("first", "late", handler).err().unwrap().code, "windowUnavailable");
        runtime.cancel("first", "after-close").unwrap();
        assert!(!runtime.lock().pending_cancellations.contains_key("first"));
        drop(first);
        assert_eq!(collected.lock()[0]["type"], "cancelled");
    }

    #[test]
    fn native_snapshot_validates_identity_source_language_and_both_revisions() {
        let (state, first, second) = state_with_windows();
        let work = state.snapshot_for_ai("first", &first, 1, 0, SourceLanguage::Zh).unwrap();
        assert_eq!(work.source.text, "你🙂好");
        assert!(state.snapshot_for_ai("first", &second, 1, 0, SourceLanguage::Zh).is_err());
        assert!(state.snapshot_for_ai("first", &first, 0, 0, SourceLanguage::Zh).is_err());
        assert!(state.snapshot_for_ai("first", &first, 2, 0, SourceLanguage::Zh).is_err());
        assert!(state.snapshot_for_ai("first", &first, 1, 0, SourceLanguage::Ja).is_err());
        state.observe_target("first", &first, 1).unwrap();
        assert!(state.snapshot_for_ai("first", &first, 1, 0, SourceLanguage::Zh).is_err());
        assert!(!work.cancellation.is_cancelled());
        assert_eq!(work.source.text, "你🙂好");
    }

    #[test]
    fn source_language_dictionary_and_close_invalidate_immutable_ai_work() {
        let (state, first, second) = state_with_windows();
        let first_work = state.snapshot_for_ai("first", &first, 1, 0, SourceLanguage::Zh).unwrap();
        let second_work = state.snapshot_for_ai("second", &second, 1, 0, SourceLanguage::Zh).unwrap();
        state.observe_source("first", &first, 2, SourceLanguage::Ja, "学校".into()).unwrap();
        assert!(first_work.cancellation.is_cancelled());
        assert!(!second_work.cancellation.is_cancelled());
        assert_eq!(first_work.source.text, "你🙂好");
        state.replace_dictionaries(DictionarySnapshot::new(2, HashMap::new(), 1)).unwrap();
        assert!(second_work.cancellation.is_cancelled());
        let current = state.snapshot_for_ai("second", &second, 1, 0, SourceLanguage::Zh).unwrap();
        state.remove_window("second").unwrap();
        assert!(current.cancellation.is_cancelled());
        assert!(state.snapshot_for_ai("second", &second, 1, 0, SourceLanguage::Zh).is_err());
    }

    #[test]
    fn external_invalidation_cannot_publish_a_successful_terminal() {
        let runtime = runtime();
        let (handler, collected) = events();
        let job = runtime.register("first", "job", handler).unwrap();
        let source_epoch = CancellationToken::new();
        job.control.invalidation.set(source_epoch.clone()).unwrap();
        source_epoch.cancel();
        assert!(job.control.emit(AiEvent::Started { job_id: "job".into(), chunk_count: 1 }).is_err());
        job.control.terminal(Ok(ProviderOutput { text: "stale".into(), usage: None }));
        drop(job);
        assert_eq!(collected.lock().as_slice(), &[json!({ "type": "cancelled", "jobId": "job" })]);
    }

    #[test]
    fn usage_only_sums_counts_that_were_reported() {
        let mut total = None;
        add_usage(&mut total, None);
        assert_eq!(total, None);
        add_usage(&mut total, Some(&AiUsage { input_tokens: Some(3), output_tokens: None }));
        add_usage(&mut total, Some(&AiUsage { input_tokens: None, output_tokens: Some(7) }));
        add_usage(&mut total, Some(&AiUsage { input_tokens: Some(0), output_tokens: Some(2) }));
        assert_eq!(total, Some(AiUsage { input_tokens: Some(3), output_tokens: Some(9) }));
        let mut unknown = None;
        add_usage(&mut unknown, Some(&AiUsage::default()));
        assert_eq!(unknown, Some(AiUsage::default()));
    }

    #[tokio::test]
    async fn registered_validation_error_uses_channel_not_duplicate_command_error() {
        let (state, first, _) = state_with_windows();
        let directory = tempfile::tempdir().unwrap();
        let settings = Arc::new(SettingsStore::open(directory.path()).unwrap());
        let profiles = Arc::new(ProfileService::open(directory.path(), settings).unwrap());
        let runtime = runtime();
        let (handler, collected) = events();
        runtime.start(state, profiles, "first".into(), request(&first, "without-review"), handler).await.unwrap();
        let values = collected.lock();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["type"], "error");
        assert_eq!(values[0]["code"], "ai_preview_required");
    }

    #[tokio::test]
    async fn pre_registration_cancel_does_not_resolve_profile_or_send_http() {
        let (state, first, _) = state_with_windows();
        let directory = tempfile::tempdir().unwrap();
        let settings = Arc::new(SettingsStore::open(directory.path()).unwrap());
        let profiles = Arc::new(ProfileService::open(directory.path(), settings).unwrap());
        let runtime = runtime();
        runtime.cancel("first", "cancelled-before-start").unwrap();
        let (handler, collected) = events();
        Arc::clone(&runtime).start(
            state, profiles, "first".into(), request(&first, "cancelled-before-start"), handler,
        ).await.unwrap();
        assert_eq!(collected.lock().as_slice(), &[json!({
            "type": "cancelled", "jobId": "cancelled-before-start",
        })]);
        assert!(!runtime.lock().active.contains_key("first"));
    }

    fn profile_service() -> (tempfile::TempDir, Arc<ProfileService>) {
        let directory = tempfile::tempdir().unwrap();
        let settings = Arc::new(SettingsStore::open(directory.path()).unwrap());
        let profiles = Arc::new(ProfileService::open(directory.path(), settings).unwrap());
        profiles.save(AiProfile {
            id: "not-configured".into(), name: "Local fixture".into(),
            protocol: AiProtocol::OpenaiChat, base_url: "http://127.0.0.1:1/proxy/team/v1".into(),
            model: "fixture-model".into(), stream: true, max_output_tokens: 4096,
            auth_mode: super::super::AuthMode::None, allow_insecure_http: false,
            token_limit_field: Some(super::super::TokenLimitField::MaxTokens),
        }).unwrap();
        (directory, profiles)
    }

    #[tokio::test]
    async fn actual_review_is_frozen_and_changed_request_cannot_send() {
        let (state, first, _) = state_with_windows();
        let (_directory, profiles) = profile_service();
        let runtime = runtime();
        let original = request(&first, "reviewed");
        let preview = Arc::clone(&runtime).preview(
            Arc::clone(&state), Arc::clone(&profiles), "first".into(), original.clone(),
        ).await.unwrap();
        assert_eq!(preview.request_url, "http://127.0.0.1:1/proxy/team/v1/chat/completions");
        assert_eq!(preview.model, "fixture-model");
        assert_eq!(preview.source_text, "你🙂好");
        assert_eq!(preview.chunk_count, 1);
        assert_eq!(preview.chunks[0].source_range, original.source_range);
        assert_eq!(preview.chunks[0].source_text, original.source_text);
        assert!(!preview.chunks[0].system.is_empty());
        assert!(!preview.chunks[0].user.is_empty());
        let mut changed = original;
        changed.instructions = "New unreviewed instructions".into();
        let (handler, collected) = events();
        runtime.start(state, profiles, "first".into(), changed, handler).await.unwrap();
        let values = collected.lock();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["code"], "ai_preview_stale");
    }

    #[tokio::test]
    async fn changed_profile_requires_review_before_any_adapter_is_called() {
        let (state, first, _) = state_with_windows();
        let (_directory, profiles) = profile_service();
        let runtime = runtime();
        let request = request(&first, "changed-profile");
        Arc::clone(&runtime).preview(
            Arc::clone(&state), Arc::clone(&profiles), "first".into(), request.clone(),
        ).await.unwrap();
        let mut changed = profiles.get(&request.profile_id).unwrap();
        changed.model = "unreviewed-model".into();
        profiles.save(changed).unwrap();
        let (handler, collected) = events();
        runtime.start(state, profiles, "first".into(), request, handler).await.unwrap();
        let values = collected.lock();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["code"], "ai_preview_stale");
    }

    #[tokio::test]
    async fn source_edit_after_review_is_cancelled_not_a_new_hidden_payload() {
        let (state, first, _) = state_with_windows();
        let (_directory, profiles) = profile_service();
        let runtime = runtime();
        let request = request(&first, "source-edited");
        Arc::clone(&runtime).preview(
            Arc::clone(&state), Arc::clone(&profiles), "first".into(), request.clone(),
        ).await.unwrap();
        state.observe_source("first", &first, 2, SourceLanguage::Zh, "changed".into()).unwrap();
        let (handler, collected) = events();
        runtime.start(state, profiles, "first".into(), request, handler).await.unwrap();
        assert_eq!(collected.lock().as_slice(), &[json!({ "type": "cancelled", "jobId": "source-edited" })]);
    }

    #[tokio::test]
    async fn target_edit_between_review_and_start_is_rejected_before_http() {
        let (state, first, _) = state_with_windows();
        let (_directory, profiles) = profile_service();
        let runtime = runtime();
        let request = request(&first, "target-edited");
        Arc::clone(&runtime).preview(
            Arc::clone(&state), Arc::clone(&profiles), "first".into(), request.clone(),
        ).await.unwrap();
        state.observe_target("first", &first, 1).unwrap();
        let (handler, collected) = events();
        runtime.start(Arc::clone(&state), profiles, "first".into(), request, handler).await.unwrap();
        let values = collected.lock();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["type"], "error");
        assert_ne!(values[0]["code"], "ai_connection");
        assert_eq!(state.health("first").unwrap().target_revision, 1);
    }

    #[test]
    fn fixed_test_and_document_job_share_one_slot_and_pending_cancellation() {
        let runtime = runtime();
        runtime.cancel("first", "test").unwrap();
        let test = runtime.begin_test("first", "test").unwrap();
        assert!(test.cancellation().is_cancelled());
        let (handler, _) = events();
        assert_eq!(runtime.register("first", "translation", Arc::clone(&handler)).err().unwrap().code, "ai_job_active");
        let second_window = runtime.register("second", "translation", handler).unwrap();
        assert!(!second_window.control.cancelled());
        let result = test.finish(Ok(ProviderOutput { text: "a cancelled response".into(), usage: None }));
        assert_eq!(result.err().unwrap().code, "ai_cancelled");
        assert!(!runtime.lock().active.contains_key("first"));
    }

    #[tokio::test]
    async fn empty_translation_and_improvement_fail_before_profile_or_http_access() {
        let (state, first, _) = state_with_windows();
        state.observe_source("first", &first, 2, SourceLanguage::Zh, " \n".into()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let settings = Arc::new(SettingsStore::open(directory.path()).unwrap());
        let profiles = Arc::new(ProfileService::open(directory.path(), settings).unwrap());
        let runtime = runtime();
        let mut empty = request(&first, "empty");
        empty.source_revision = 2;
        empty.source_text = " \n".into();
        empty.source_range = TextRange { start: 0, end: 2 };
        assert_eq!(Arc::clone(&runtime).preview(
            Arc::clone(&state), Arc::clone(&profiles), "first".into(), empty.clone(),
        ).await.err().unwrap().code, "ai_empty_input");
        empty.mode = AiMode::Improve;
        empty.target_text = Some(" \t".into());
        assert_eq!(runtime.preview(state, profiles, "first".into(), empty).await.err().unwrap().code, "ai_empty_input");
    }
}
