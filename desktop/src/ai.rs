use super::*;
use quickertranslator_lib::ai::{
    AiEvent, AiMode, AiProfile, AiScope, AiTranslationRequest,
    profiles::{CredentialStorage, SecretString, endpoint_identity, official_presets},
};
use quickertranslator_lib::documents::schema::plain_text;

pub struct Panel {
    profile: Field,
    id: Field,
    name: Field,
    model: Field,
    url: Field,
    advanced: bool,
    secret: Field,
    instructions: Field,
    profile_id: String,
    preset: usize,
    scope: AiScope,
    preparing: bool,
    mode: AiMode,
    pub review: Option<AiTranslationRequest>,
    pub completed: Option<(u64, u64, String)>,
    pub active: Option<String>,
    display: String,
}
impl Panel {
    fn load(&mut self, p: &AiProfile, w: &mut Window, c: &mut App) -> Result<()> {
        set(&self.id, p.id.clone(), w, c);
        set(&self.name, p.name.clone(), w, c);
        set(&self.model, p.model.clone(), w, c);
        set(&self.url, p.base_url.clone(), w, c);
        set(&self.profile, serde_json::to_string_pretty(p)?, w, c);
        set(&self.secret, "", w, c);
        self.profile_id = p.id.clone();
        self.review = None;
        Ok(())
    }
    fn edited(&self, c: &App) -> Result<AiProfile> {
        let mut p: AiProfile = serde_json::from_str(&value(&self.profile, c))?;
        if !self.advanced {
            p.id = value(&self.id, c);
            p.name = value(&self.name, c);
            p.model = value(&self.model, c);
            p.base_url = value(&self.url, c);
        }
        Ok(p)
    }
    pub fn new(w: &mut Window, c: &mut App) -> Self {
        let profile = &official_presets()[0];
        Self {
            id: field(&profile.id, false, w, c),
            name: field(&profile.name, false, w, c),
            model: field(&profile.model, false, w, c),
            url: field(&profile.base_url, false, w, c),
            advanced: false,
            profile: field(&serde_json::to_string_pretty(profile).unwrap(), true, w, c),
            secret: c.new(|c| InputState::new(w, c).masked(true)),
            instructions: field("", true, w, c),
            profile_id: profile.id.clone(),
            preset: 0,
            scope: AiScope::Document,
            preparing: false,
            mode: AiMode::Translate,
            review: None,
            completed: None,
            active: None,
            display: String::new(),
        }
    }
}
impl Workspace {
    fn preview_ai(&mut self, w: &mut Window, c: &mut Context<Self>) -> Result<()> {
        anyhow::ensure!(self.ai.active.is_none(), "Cancel the active request first");
        self.ai.review = None;
        self.ai.completed = None;
        let src = value(&self.source, c);
        let label = self.identity.window_label.clone();
        let state = self.services.state.clone();
        let doc = self.identity.document_id.clone();
        state.observe_source(
            &label,
            &doc,
            self.revision,
            self.project.source_language,
            src.clone(),
        )?;
        state.observe_options(
            &label,
            &doc,
            self.revision,
            self.settings.translation_options.clone(),
        )?;
        anyhow::ensure!(!self.ai.preparing, "Payload preview is already running");
        let bytes = if self.ai.scope == AiScope::Selection {
            selection(&self.source, w, c)?
        } else {
            0..src.len()
        };
        let source_range =
            engine::alignment::OffsetMap::new(&src)?.byte_range_to_utf16(bytes.clone())?;
        let request = AiTranslationRequest {
            job_id: Uuid::new_v4().to_string(),
            document_id: doc,
            source_revision: self.revision,
            target_revision: self.target_revision,
            profile_id: self.ai.profile_id.clone(),
            mode: self.ai.mode,
            scope: self.ai.scope,
            source_language: self.project.source_language,
            source_range,
            source_text: src[bytes].to_string(),
            target_text: if self.ai.mode == AiMode::Improve {
                Some(if self.ai.scope == AiScope::Selection {
                    let range = selection(&self.target, w, c)?;
                    value(&self.target, c)[range].to_owned()
                } else {
                    plain_text(&self.snapshot(c)?.target_document)
                })
            } else {
                None
            },
            instructions: value(&self.ai.instructions, c),
        };
        let work = self
            .services
            .runtime
            .spawn(self.services.jobs.clone().preview(
                state,
                self.services.profiles.clone(),
                label,
                request.clone(),
            ));
        self.ai.preparing = true;
        self.ai.display = "Preparing payload…".into();
        c.spawn(async move |this, c| {
            let result = work.await;
            let _ = this.update(c, |s, c| {
                s.ai.preparing = false;
                if s.ai.profile_id == request.profile_id
                    && s.ai.mode == request.mode
                    && s.ai.scope == request.scope
                    && value(&s.ai.instructions, c) == request.instructions
                    && s.revision == request.source_revision
                    && s.target_revision == request.target_revision
                {
                    match result {
                        Ok(Ok(preview)) => {
                            s.ai.display = serde_json::to_string_pretty(&preview).unwrap();
                            s.ai.review = Some(request);
                        }
                        Ok(Err(e)) => s.ai.display = e.message,
                        Err(e) => s.ai.display = e.to_string(),
                    }
                }
                c.notify();
            });
        })
        .detach();
        Ok(())
    }
    fn send_ai(&mut self, _: &mut Window, c: &mut Context<Self>) -> Result<()> {
        let r = self.ai.review.take().context("Preview the request first")?;
        anyhow::ensure!(
            r.source_revision == self.revision && r.target_revision == self.target_revision,
            "Document changed. Preview again."
        );
        anyhow::ensure!(
            value(&self.ai.instructions, c) == r.instructions && self.ai.profile_id == r.profile_id,
            "Request changed. Preview again."
        );
        self.ai.active = Some(r.job_id.clone());
        self.ai.completed = None;
        self.ai.display.clear();
        let (events, rx) = async_channel::unbounded();
        let state = self.services.state.clone();
        let jobs = self.services.jobs.clone();
        let profiles = self.services.profiles.clone();
        let label = self.identity.window_label.clone();
        let rev = r.source_revision;
        let target = r.target_revision;
        self.services.runtime.spawn(async move {
            let output = events.clone();
            let id = r.job_id.clone();
            if let Err(e) = jobs
                .start(
                    state,
                    profiles,
                    label,
                    r,
                    Arc::new(move |event| {
                        let _ = output.try_send(event);
                        Ok(())
                    }),
                )
                .await
            {
                let _ = events.try_send(AiEvent::Error {
                    job_id: id,
                    message: e.message,
                    code: e.code,
                    chunk_index: None,
                });
            }
        });
        c.spawn(async move |this, c| {
            while let Ok(event) = rx.recv().await {
                if this
                    .update(c, |s, c| {
                        match event {
                            AiEvent::Delta { text, .. } => s.ai.display.push_str(&text),
                            AiEvent::Completed { text, .. } => {
                                s.ai.active = None;
                                s.ai.display = text.clone();
                                s.ai.completed = Some((rev, target, text));
                            }
                            AiEvent::Cancelled { .. } => {
                                s.ai.active = None;
                                s.ai.display = "Cancelled".into();
                            }
                            AiEvent::Error { message, .. } => {
                                s.ai.active = None;
                                s.ai.display = message;
                            }
                            _ => {}
                        }
                        c.notify()
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Ok(())
    }
    pub fn profile_settings(&self, c: &Context<Self>) -> Div {
        let profiles = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                row()
                    .child(button("Next preset", c, |s, w, c| {
                        let presets = official_presets();
                        s.ai.preset = (s.ai.preset + 1) % presets.len();
                        let p = &presets[s.ai.preset];
                        s.ai.profile_id = p.id.clone();
                        s.ai.review = None;
                        s.ai.load(p, w, c)?;
                        Ok(())
                    }))
                    .child(button("Load saved profile", c, |s, w, c| {
                        let profiles = s.services.profiles.export_profiles()?;
                        anyhow::ensure!(!profiles.is_empty(), "No saved profiles");
                        let i = profiles
                            .iter()
                            .position(|p| p.id == s.ai.profile_id)
                            .map_or(0, |i| (i + 1) % profiles.len());
                        let p = &profiles[i];
                        s.ai.profile_id = p.id.clone();
                        s.ai.review = None;
                        s.ai.load(p, w, c)?;
                        Ok(())
                    })),
            )
            .child(button(
                if self.ai.advanced {
                    "Profile fields"
                } else {
                    "Advanced profile"
                },
                c,
                |s, w, c| {
                    let p = s.ai.edited(c)?;
                    s.ai.advanced = !s.ai.advanced;
                    s.ai.load(&p, w, c)
                },
            ))
            .when(self.ai.advanced, |d| {
                d.child(Input::new(&self.ai.profile).h(px(260.)))
            })
            .when(!self.ai.advanced, |d| {
                d.child("Profile ID")
                    .child(Input::new(&self.ai.id))
                    .child("Name")
                    .child(Input::new(&self.ai.name))
                    .child("Base URL")
                    .child(Input::new(&self.ai.url))
                    .child("Model")
                    .child(Input::new(&self.ai.model))
            })
            .child(button("Save profile", c, |s, _, c| {
                let profile = s.ai.edited(c)?;
                let saved = s.services.profiles.save(profile)?;
                s.ai.profile_id = saved.profile.id;
                s.ai.review = None;
                s.status = format!("Saved · {}", saved.request_url);
                Ok(())
            }))
            .child(button("Delete profile", c, |s, _, _| {
                if confirm("Delete saved profile?", &s.ai.profile_id) {
                    s.services.profiles.delete(&s.ai.profile_id)?;
                    s.ai.review = None;
                }
                Ok(())
            }))
            .child("Credential")
            .child(Input::new(&self.ai.secret))
            .child(
                row().children(
                    [
                        ("Session key", CredentialStorage::Session),
                        ("Store in keychain", CredentialStorage::Keychain),
                    ]
                    .into_iter()
                    .map(|(label, storage)| {
                        button(label, c, move |s, w, c| {
                            let p = s.ai.edited(c)?;
                            let endpoint = endpoint_identity(&p)?;
                            let secret = SecretString::new(value(&s.ai.secret, c));
                            s.services
                                .profiles
                                .set_credential(&p.id, &endpoint, secret, storage)?;
                            set(&s.ai.secret, "", w, c);
                            s.status = format!("Credential set for {endpoint}");
                            Ok(())
                        })
                    }),
                ),
            )
            .child(button("Remove credential", c, |s, _, _| {
                if confirm("Remove credential?", &s.ai.profile_id) {
                    s.services.profiles.delete_credential(&s.ai.profile_id)?;
                }
                Ok(())
            }));
        div().size_full().child(scroll("profiles", profiles))
    }
    pub fn ai_request(&self, c: &Context<Self>) -> Div {
        let vi = self.settings.locale == UiLocale::Vi;
        let request = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                row()
                    .child(format!(
                        "{} · {}",
                        if vi { "Hồ sơ" } else { "Profile" },
                        self.ai.profile_id
                    ))
                    .child(button(
                        if vi {
                            "Quản lý hồ sơ"
                        } else {
                            "Manage profiles"
                        },
                        c,
                        |s, _, _| {
                            s.page = Page::Ai;
                            Ok(())
                        },
                    )),
            )
            .child("Instructions")
            .child(Input::new(&self.ai.instructions).h(px(80.)))
            .child(
                row()
                    .child(button(format!("{:?}", self.ai.mode), c, |s, _, _| {
                        s.ai.mode = if s.ai.mode == AiMode::Translate {
                            AiMode::Improve
                        } else {
                            AiMode::Translate
                        };
                        s.ai.review = None;
                        Ok(())
                    }))
                    .child(button(format!("{:?}", self.ai.scope), c, |s, _, _| {
                        s.ai.scope = if s.ai.scope == AiScope::Document {
                            AiScope::Selection
                        } else {
                            AiScope::Document
                        };
                        s.ai.review = None;
                        Ok(())
                    }))
                    .child(button("Preview payload", c, Self::preview_ai))
                    .when(self.ai.review.is_some(), |d| {
                        d.child(button("Send reviewed payload", c, Self::send_ai))
                    })
                    .when(self.ai.active.is_some(), |d| {
                        d.child(button("Cancel", c, |s, _, _| {
                            if let Some(id) = &s.ai.active {
                                s.services.jobs.cancel(&s.identity.window_label, id)?;
                            }
                            Ok(())
                        }))
                    }),
            )
            .when(self.ai.completed.is_some(), |d| {
                d.child(button("Insert AI result", c, |s, w, c| {
                    let (rev, target, text) =
                        s.ai.completed
                            .as_ref()
                            .context("No completed translation")?;
                    anyhow::ensure!(
                        *rev == s.revision && *target == s.target_revision,
                        "Document changed. Preview again."
                    );
                    s.insert_text(text.clone(), w, c)?;
                    s.ai.completed = None;
                    Ok(())
                }))
            })
            .child(self.ai.display.clone());
        div().size_full().child(scroll("ai-request", request))
    }
}
