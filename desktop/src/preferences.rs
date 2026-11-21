use super::*;
use quickertranslator_lib::settings::Theme;

pub struct PreferencesChanged;
impl Global for PreferencesChanged {}
pub struct DictionariesChanged;
impl Global for DictionariesChanged {}
impl Workspace {
    fn change_preference(
        &mut self,
        w: &mut Window,
        c: &mut Context<Self>,
        edit: impl FnOnce(&mut Settings),
    ) -> Result<()> {
        let mut settings = self.services.settings.load()?.settings;
        edit(&mut settings);
        self.save_preferences(settings, w, c)
    }
    fn save_preferences(
        &mut self,
        settings: Settings,
        w: &mut Window,
        c: &mut Context<Self>,
    ) -> Result<()> {
        self.services.settings.save(&settings, false)?;
        apply_theme(&settings, c);
        set(
            &self.preferences,
            serde_json::to_string_pretty(&settings)?,
            w,
            c,
        );

        c.set_global(PreferencesChanged);
        self.status = "Preferences saved".into();
        Ok(())
    }
    pub fn preference_page(&self, c: &Context<Self>) -> Div {
        let options = &self.settings.translation_options;
        let vi = self.settings.locale == UiLocale::Vi;
        let line = |label: &str, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(label.to_owned())
                .child(control)
        };
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(line(
                if vi {
                    "Ngôn ngữ giao diện"
                } else {
                    "Interface language"
                },
                button(
                    if vi {
                        "Tiếng Việt  ▾"
                    } else {
                        "English  ▾"
                    },
                    c,
                    |s, w, c| {
                        s.change_preference(w, c, |p| {
                            p.locale = if p.locale == UiLocale::Vi {
                                UiLocale::En
                            } else {
                                UiLocale::Vi
                            }
                        })
                    },
                )
                .into_any_element(),
            ))
            .child(line(
                if vi { "Giao diện" } else { "Appearance" },
                button(
                    match (self.settings.theme, vi) {
                        (Theme::Dark, true) => "Tối  ▾",
                        (Theme::Light, true) => "Sáng  ▾",
                        (Theme::Dark, false) => "Dark  ▾",
                        _ => "Light  ▾",
                    },
                    c,
                    |s, w, c| {
                        s.change_preference(w, c, |p| {
                            p.theme = match p.theme {
                                Theme::Dark => Theme::Light,
                                Theme::Light => Theme::Dark,
                            }
                        })
                    },
                )
                .into_any_element(),
            ))
            .child(
                div()
                    .border_t_1()
                    .border_color(c.theme().border)
                    .pt_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if vi {
                        "Biên tập / dịch ngoại tuyến"
                    } else {
                        "Editing / offline translation"
                    }),
            )
            .child(
                row()
                    .child(button(
                        format!(
                            "{} · {:?}",
                            if vi { "Thuật toán" } else { "Algorithm" },
                            options.algorithm
                        ),
                        c,
                        |s, w, c| {
                            s.change_preference(w, c, |p| {
                                p.translation_options.algorithm =
                                    match p.translation_options.algorithm {
                                        TranslationAlgorithm::Longest => {
                                            TranslationAlgorithm::LeftToRight
                                        }
                                        TranslationAlgorithm::LeftToRight => {
                                            TranslationAlgorithm::LongestConditional
                                        }
                                        TranslationAlgorithm::LongestConditional => {
                                            TranslationAlgorithm::Longest
                                        }
                                    }
                            })
                        },
                    ))
                    .child(button(
                        format!(
                            "{} {}",
                            if options.prioritize_names {
                                "☑"
                            } else {
                                "☐"
                            },
                            if vi {
                                "Ưu tiên tên"
                            } else {
                                "Prioritize names"
                            }
                        ),
                        c,
                        |s, w, c| {
                            s.change_preference(w, c, |p| {
                                p.translation_options.prioritize_names =
                                    !p.translation_options.prioritize_names
                            })
                        },
                    ))
                    .child(button(
                        format!(
                            "{} · {:?}",
                            if vi {
                                "Ngoặc VietPhrase"
                            } else {
                                "Phrase brackets"
                            },
                            options.full_wrap
                        ),
                        c,
                        |s, w, c| {
                            s.change_preference(w, c, |p| {
                                p.translation_options.full_wrap =
                                    match p.translation_options.full_wrap {
                                        FullWrap::None => FullWrap::All,
                                        FullWrap::All => FullWrap::Ambiguous,
                                        FullWrap::Ambiguous => FullWrap::None,
                                    }
                            })
                        },
                    ))
                    .child(button(
                        format!(
                            "{} · {:?}",
                            if vi {
                                "Ngoặc một nghĩa"
                            } else {
                                "First-meaning brackets"
                            },
                            options.single_wrap
                        ),
                        c,
                        |s, w, c| {
                            s.change_preference(w, c, |p| {
                                p.translation_options.single_wrap =
                                    if p.translation_options.single_wrap == SingleWrap::None {
                                        SingleWrap::All
                                    } else {
                                        SingleWrap::None
                                    }
                            })
                        },
                    )),
            )
            .child(
                row()
                    .child(button(
                        if self.settings.locale == UiLocale::Vi {
                            "Hồ sơ AI"
                        } else {
                            "AI profiles"
                        },
                        c,
                        |s, _, _| {
                            s.page = Page::Ai;
                            Ok(())
                        },
                    ))
                    .child(button(
                        if self.settings.locale == UiLocale::Vi {
                            "Từ điển"
                        } else {
                            "Dictionaries"
                        },
                        c,
                        |s, _, _| {
                            s.page = Page::Dictionaries;
                            Ok(())
                        },
                    )),
            )
            .child(button("Advanced preferences", c, |s, _, _| {
                s.advanced = !s.advanced;
                Ok(())
            }))
            .when(self.advanced, |d| {
                d.child(Input::new(&self.preferences).h(px(330.)))
                    .child(button("Save preferences", c, |s, w, c| {
                        let settings = serde_json::from_str(&value(&s.preferences, c))?;
                        s.save_preferences(settings, w, c)
                    }))
            });
        div().flex_1().min_h_0().child(scroll("preferences", form))
    }
}
