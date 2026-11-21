use super::*;
use gpui_component::resizable::{h_resizable, resizable_panel, v_resizable};

#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Pane {
    Readings,
    Source,
    Draft,
    Phrases,
    Ai,
    Meanings,
    Target,
}
use Pane::*;
const PANES: [Pane; 7] = [Readings, Source, Draft, Phrases, Ai, Meanings, Target];
const GROUPS: [&[Pane]; 4] = [
    &[Readings, Source],
    &[Draft, Phrases, Ai],
    &[Meanings],
    &[Target],
];
#[derive(Clone, Copy, PartialEq)]
pub enum Menu {
    File,
    Layout,
    Language,
    Size,
    Color,
    Align,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct DockState {
    groups: [Vec<Pane>; 4],
    visible: [bool; 7],
    active: [Pane; 4],
    ratios: [f32; 3],
}
impl Default for DockState {
    fn default() -> Self {
        Self {
            groups: GROUPS.map(|g| g.to_vec()),
            visible: [true; 7],
            active: [Source, Draft, Meanings, Target],
            ratios: [0.30, 0.58, 0.60],
        }
    }
}
#[derive(Clone)]
struct DragTab(Pane, &'static str);
impl Render for DragTab {
    fn render(&mut self, _: &mut Window, c: &mut Context<Self>) -> impl IntoElement {
        div().px_2().py_1().bg(c.theme().secondary).child(self.1)
    }
}
pub struct Layout {
    dock: DockState,
    generation: usize,
    pub menu: Option<Menu>,
    pub selected: bool,
    pub source_edit: bool,
    anchor: Point<Pixels>,
    font: u16,
    align: &'static str,
    color: &'static str,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            dock: DockState::default(),
            generation: 0,
            menu: None,
            selected: false,
            source_edit: false,
            anchor: point(px(0.), px(0.)),
            font: 16,
            align: "left",
            color: "#24282d",
        }
    }
}
impl Layout {
    pub fn load(path: &std::path::Path) -> Self {
        let mut s = Self::default();
        if let Some(d) = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<DockState>(&b).ok())
        {
            let all = d.groups.iter().flatten().copied().collect::<Vec<_>>();
            if all.len() == 7
                && PANES.iter().all(|p| all.contains(p))
                && d.ratios
                    .iter()
                    .all(|r| r.is_finite() && (0.02..=0.98).contains(r))
                && d.groups.iter().enumerate().all(|(i, g)| {
                    !g.iter().any(|p| d.visible[*p as usize])
                        || (g.contains(&d.active[i]) && d.visible[d.active[i] as usize])
                })
            {
                s.dock = d;
            }
        }
        s
    }
    fn group_of(&self, p: Pane) -> usize {
        self.dock
            .groups
            .iter()
            .position(|g| g.contains(&p))
            .unwrap()
    }
    pub fn show(&mut self, p: Pane) {
        self.dock.visible[p as usize] = true;
        let g = self.group_of(p);
        self.dock.active[g] = p;
    }

    fn toggle(&mut self, p: Pane) {
        self.dock.visible[p as usize] = !self.dock.visible[p as usize];
        let g = self.group_of(p);
        if self.dock.visible[p as usize] {
            self.dock.active[g] = p;
        } else if self.dock.active[g] == p {
            if let Some(next) = self.dock.groups[g]
                .iter()
                .find(|v| self.dock.visible[**v as usize])
            {
                self.dock.active[g] = *next;
            }
        }
    }
}
#[derive(Clone, Copy)]
struct Palette {
    chrome: Hsla,
    surface: Hsla,
    edge: Hsla,
    muted: Hsla,
    ink: Hsla,
    focus: Hsla,
}
impl Workspace {
    fn save_layout(&mut self) {
        if let Err(e) = quickertranslator_lib::storage::atomic_replace_json(
            &self.services.layout_path,
            &self.layout.dock,
        ) {
            self.status = e.message;
        }
    }
    fn resized(&mut self, index: usize, sizes: &[Pixels]) {
        if let [a, b] = sizes {
            let total = f32::from(*a + *b);
            if total > 0. {
                self.layout.dock.ratios[index] = (f32::from(*a) / total).clamp(0.02, 0.98);
                self.save_layout();
            }
        }
    }

    fn palette(&self) -> Palette {
        let dark = matches!(
            self.settings.theme,
            quickertranslator_lib::settings::Theme::Dark
        );
        let color = |light, d| rgb(if dark { d } else { light }).into();
        Palette {
            chrome: color(0xeeeeee, 0x292c31),
            surface: color(0xffffff, 0x1f2227),
            edge: color(0xb8bdc3, 0x505660),
            muted: color(0x616872, 0xadb4bf),
            ink: color(0x24282d, 0xe8eaf0),
            focus: color(0x27629d, 0x87b9f1),
        }
    }
    fn label(&self, p: Pane) -> &'static str {
        let vi = self.settings.locale == UiLocale::Vi;
        let ja = self.project.source_language == SourceLanguage::Ja;
        match p {
            Readings if !ja => "Hán Việt",
            Readings if vi => "Cách đọc",
            Readings => "Readings",
            Source => match (ja, vi) {
                (true, true) => "Nhật",
                (true, false) => "Japanese",
                (false, true) => "Trung",
                _ => "Chinese",
            },
            Draft => match (ja, vi) {
                (false, true) => "VietPhrase một nghĩa",
                (false, false) => "VietPhrase — first meaning",
                (true, true) => "Nghĩa từ một nghĩa",
                _ => "Word glosses — first meaning",
            },
            Phrases if !ja => "VietPhrase",
            Phrases if vi => "Nghĩa từ",
            Phrases => "Word glosses",
            Ai => "AI",
            Meanings if vi => "Nghĩa",
            Meanings => "Meanings",
            Target if vi => "Việt",
            Target => "Vietnamese",
        }
    }
    fn tool(
        &self,
        label: impl Into<SharedString>,
        boxed: bool,
        c: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) -> Result<()> + 'static,
    ) -> Stateful<Div> {
        let p = self.palette();
        let label = label.into();
        div()
            .id(label.clone())
            .flex()
            .items_center()
            .h(px(if boxed { 24. } else { 26. }))
            .px(px(if boxed { 5. } else { 8. }))
            .rounded(px(2.))
            .text_size(px(if boxed { 12. } else { 13. }))
            .cursor_pointer()
            .when(boxed, |d| d.border_1().border_color(p.edge).bg(p.surface))
            .hover(|d| d.bg(p.edge.opacity(0.42)))
            .child(label)
            .on_click(c.listener(move |s, _, w, c| {
                s.layout.menu = None;
                if let Err(e) = action(s, w, c) {
                    s.status = e.to_string()
                }
                c.notify();
            }))
    }
    fn menu_button(
        &self,
        title: impl Into<SharedString>,
        menu: Menu,
        boxed: bool,
        c: &Context<Self>,
    ) -> Stateful<Div> {
        let was = self.layout.menu == Some(menu);
        self.tool(title, boxed, c, move |s, w, _| {
            s.layout.anchor = w.mouse_position();
            s.layout.menu = (!was).then_some(menu);
            Ok(())
        })
    }
    fn go(&self, next: bool, c: &mut App) -> Result<()> {
        let path = self.origin.as_ref().context("Open a document first")?;
        let sib = quickertranslator_lib::documents::sibling_documents(path)?;
        let path = if next { sib.next } else { sib.previous };
        if let Some(path) = path {
            files::open_path(self.services.clone(), self.project.source_language, path, c)?;
        }
        Ok(())
    }
    fn toolbar(&self, c: &Context<Self>) -> Div {
        let p = self.palette();
        let vi = self.settings.locale == UiLocale::Vi;
        let lang = match (self.project.source_language, vi) {
            (SourceLanguage::Zh, true) => "Tiếng Trung",
            (SourceLanguage::Ja, true) => "Tiếng Nhật",
            (SourceLanguage::Zh, false) => "Chinese",
            _ => "Japanese",
        };
        let mut bar = div()
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap(px(2.))
            .px(px(5.))
            .py(px(3.))
            .min_h(px(34.))
            .border_b_1()
            .border_color(p.edge)
            .bg(p.chrome)
            .child(self.menu_button("File", Menu::File, false, c))
            .child(self.menu_button("Layout", Menu::Layout, false, c))
            .child(self.tool("Translate From Clipboard", false, c, |s, w, c| {
                if let Some(text) = c.read_from_clipboard().and_then(|item| item.text()) {
                    if s.draft_custom
                        && !confirm(
                            "Replace the current draft?",
                            "Translate the clipboard using a new draft.",
                        )
                    {
                        return Ok(());
                    }
                    s.draft_custom = false;
                    set(&s.source, text, w, c);
                    s.layout.show(Source);
                }
                Ok(())
            }))
            .child(self.tool("Re-Translate", false, c, Self::translate))
            .child(self.tool("Reload Dicts", false, c, |s, _, _| {
                s.page = Page::Dictionaries;
                Ok(())
            }))
            .child(self.tool("Config", false, c, |s, _, _| {
                s.page = Page::Preferences;
                Ok(())
            }))
            .child(div().mx(px(5.)).h(px(20.)).w(px(1.)).bg(p.edge))
            .child(self.menu_button(format!("{lang}  ▾"), Menu::Language, true, c))
            .child(self.tool("AI Translate", false, c, |s, _, _| {
                s.layout.show(Ai);
                Ok(())
            }))
            .child(div().flex_1());
        for (label, next) in [("← Back", false), ("Next →", true)] {
            if self.origin.is_some() {
                bar = bar.child(self.tool(label, false, c, move |s, _, c| s.go(next, c)));
            } else {
                bar = bar.child(
                    div()
                        .px(px(8.))
                        .text_size(px(13.))
                        .text_color(p.muted)
                        .child(label),
                );
            }
        }
        bar
    }
    fn group(&self, g: usize, w: &mut Window, c: &mut Context<Self>) -> Div {
        let p = self.palette();
        let vi = self.settings.locale == UiLocale::Vi;
        let active = self.layout.dock.active[g];
        let mut tabs = div()
            .flex()
            .items_center()
            .h(px(29.))
            .flex_shrink_0()
            .bg(p.chrome);
        for &pane in self.layout.dock.groups[g]
            .iter()
            .filter(|v| self.layout.dock.visible[**v as usize])
        {
            let chosen = pane == active;
            let tab = div()
                .id(("tab", pane as usize))
                .flex()
                .items_center()
                .h_full()
                .px(px(9.))
                .gap(px(5.))
                .border_t_2()
                .border_color(if chosen { p.focus } else { p.chrome })
                .bg(if chosen { p.surface } else { p.chrome })
                .text_size(px(12.))
                .text_color(if chosen { p.ink } else { p.muted })
                .cursor_pointer()
                .child(self.label(pane))
                .on_drag(DragTab(pane, self.label(pane)), |info, _, _, c| {
                    c.new(|_| DragTab(info.0, info.1))
                })
                .on_click(c.listener(move |s, _, _, c| {
                    s.layout.dock.active[g] = pane;
                    s.save_layout();
                    c.notify()
                }))
                .when(!chosen, |d| d.child(div().w(px(15.))))
                .when(chosen, |d| {
                    d.child(
                        div()
                            .id(("close-tab", pane as usize))
                            .w(px(13.))
                            .text_size(px(17.))
                            .child("×")
                            .on_click(c.listener(move |s, _, _, c| {
                                c.stop_propagation();
                                s.layout.toggle(pane);
                                s.save_layout();
                                c.notify()
                            })),
                    )
                });
            tabs = tabs.child(tab);
        }
        let input = |f: &Field| {
            Input::new(f)
                .appearance(false)
                .bordered(false)
                .focus_bordered(false)
                .text_size(px(if active == Source {
                    self.project.view_state.fonts.source
                } else if active == Target {
                    self.project.view_state.fonts.target
                } else {
                    self.project.view_state.fonts.single_meaning
                } as f32))
                .px(px(8.))
                .py(px(6.))
                .h_full()
        };
        let body = match active {
            Source => {
                let mut editor = div().relative().size_full().child(input(&self.source));
                if self.result.is_some() && !self.layout.source_edit {
                    editor = editor.child(
                        div()
                            .id("source-review")
                            .absolute()
                            .inset_0()
                            .bg(p.surface)
                            .text_size(px(self.project.view_state.fonts.source as f32))
                            .pl(px(5.))
                            .child(scroll("source-aligned", self.aligned(Source, c)))
                            .on_click(c.listener(|s, _, w, c| {
                                s.layout.source_edit = true;
                                s.source.update(c, |f, c| f.focus(w, c));
                                c.notify();
                            })),
                    );
                }
                editor.into_any_element()
            }
            Readings | Phrases | Draft => {
                let main = if active == Draft && self.draft_custom {
                    input(&self.draft).into_any_element()
                } else {
                    scroll("aligned-text", self.aligned(active, c)).into_any_element()
                };
                let fonts = &self.project.view_state.fonts;
                let size = match active {
                    Draft => fonts.single_meaning,
                    Readings => fonts.readings,
                    _ => fonts.phrases,
                };
                let note = if vi {
                    "Hỗ trợ cách đọc và tra nghĩa ngoại tuyến, không phải bản dịch máy hoàn chỉnh."
                } else {
                    "Offline readings and dictionary glosses are not a complete machine translation."
                };
                let hint = if active == Draft {
                    " · Alt+Shift+←/→"
                } else {
                    ""
                };
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .text_size(px(size as f32))
                            .child(main),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .h(px(20.))
                            .px(px(7.))
                            .bg(p.chrome.opacity(0.6))
                            .border_t_1()
                            .border_color(p.edge)
                            .text_color(p.muted)
                            .text_size(px(11.))
                            .child(format!("{note}{hint}")),
                    )
                    .into_any_element()
            }
            Meanings if !self.layout.selected => {
                let hint = if vi {
                    "Chọn từ trong nguồn hoặc bản nháp để xem nghĩa và nguồn từ điển."
                } else {
                    "Select a source or generated token to review meanings and dictionary provenance."
                };
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(hint)
                    .into_any_element()
            }
            Meanings => scroll("meanings", self.meanings(c)).into_any_element(),
            Target => {
                let target = if self.preview {
                    let html = self
                        .snapshot(c)
                        .and_then(|project| {
                            Ok(quickertranslator_lib::documents::export::target_html(
                                &project.target_document,
                            )?)
                        })
                        .unwrap_or_else(|e| e.to_string());
                    scroll(
                        "rich",
                        gpui_component::text::TextView::html("target-html", html, w, c),
                    )
                    .into_any_element()
                } else {
                    input(&self.target).into_any_element()
                };
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(self.format_toolbar(c))
                    .child(div().flex_1().min_h_0().child(target))
                    .into_any_element()
            }
            Ai => self.ai_request(c).into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .border_1()
            .border_color(p.edge)
            .bg(p.surface)
            .on_drop(c.listener(move |s, drag: &DragTab, _, c| {
                let from = s.layout.group_of(drag.0);
                if from != g {
                    s.layout.dock.groups[from].retain(|p| *p != drag.0);
                    if s.layout.dock.active[from] == drag.0 {
                        if let Some(&next) = s.layout.dock.groups[from]
                            .iter()
                            .find(|p| s.layout.dock.visible[**p as usize])
                        {
                            s.layout.dock.active[from] = next;
                        }
                    }
                    s.layout.dock.groups[g].push(drag.0);
                    s.layout.show(drag.0);
                    s.save_layout();
                    c.notify();
                }
            }))
            .child(tabs)
            .child(div().flex_1().min_h_0().overflow_hidden().child(body))
    }
    fn format_toolbar(&self, c: &Context<Self>) -> Div {
        let p = self.palette();
        let vi = self.settings.locale == UiLocale::Vi;
        let mut bar = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(4.))
            .h(px(35.))
            .flex_shrink_0()
            .bg(p.chrome)
            .border_b_1()
            .border_color(p.edge);
        for (label, kind) in [("B", "bold"), ("𝑰", "italic"), ("U̲", "underline")] {
            bar = bar.child(self.tool(label, true, c, move |s, w, c| {
                s.apply_format(kind, None, w, c)
            }));
        }
        bar.child(self.menu_button("■", Menu::Color, true, c))
            .child(self.menu_button(
                if vi {
                    "Cỡ chữ   ▾"
                } else {
                    "Font size   ▾"
                },
                Menu::Size,
                true,
                c,
            ))
            .child(self.menu_button(
                format!(
                    "{}   ▾",
                    match (self.layout.align, vi) {
                        ("left", true) => "Trái",
                        ("left", false) => "Left",
                        (v, _) => v,
                    }
                ),
                Menu::Align,
                true,
                c,
            ))
            .child(self.tool("↶", true, c, |s, w, c| {
                s.target.update(c, |f, c| f.focus(w, c));
                w.dispatch_action(Box::new(gpui_component::input::Undo), c);
                Ok(())
            }))
            .child(self.tool("↷", true, c, |s, w, c| {
                s.target.update(c, |f, c| f.focus(w, c));
                w.dispatch_action(Box::new(gpui_component::input::Redo), c);
                Ok(())
            }))
            .when(self.structured, |d| {
                d.child(self.tool("‹ ¶", true, c, |s, w, c| s.change_paragraph(-1, w, c)))
                    .child(self.tool("¶ ›", true, c, |s, w, c| s.change_paragraph(1, w, c)))
                    .child(self.tool("+ ¶", true, c, Self::add_paragraph))
                    .child(self.tool(
                        if self.preview { "Edit" } else { "Preview" },
                        true,
                        c,
                        |s, _, _| {
                            s.preview = !s.preview;
                            Ok(())
                        },
                    ))
            })
    }
    fn workspace_dock(&self, w: &mut Window, c: &mut Context<Self>) -> Div {
        let size = w.viewport_size();
        let width = size.width - px(6.);
        let height = size.height - px(64.);
        let weak = c.entity().downgrade();
        let mut columns =
            h_resizable(("columns", self.layout.generation)).on_resize(move |state, _, c| {
                let sizes = state.read(c).sizes().clone();
                let _ = weak.update(c, |s, _| s.resized(0, &sizes));
            });
        let [split, left, right] = self.layout.dock.ratios;
        for (col, groups, ratio, vertical) in
            [(0, [0, 2], split, left), (1, [1, 3], 1. - split, right)]
        {
            let mut column = v_resizable((
                if col == 0 {
                    "left-column"
                } else {
                    "right-column"
                },
                self.layout.generation,
            ));
            let weak = c.entity().downgrade();
            column = column.on_resize(move |state, _, c| {
                let sizes = state.read(c).sizes().clone();
                let _ = weak.update(c, |s, _| s.resized(col + 1, &sizes));
            });
            let mut count = 0;
            for (i, g) in groups.into_iter().enumerate() {
                if self.layout.dock.groups[g]
                    .iter()
                    .any(|v| self.layout.dock.visible[*v as usize])
                {
                    count += 1;
                    column = column.child(
                        resizable_panel()
                            .size(height * if i == 0 { vertical } else { 1. - vertical })
                            .child(
                                div()
                                    .size_full()
                                    .when(col == 0, |d| d.pr(px(3.)))
                                    .when(i == 0, |d| d.pb(px(3.)))
                                    .child(self.group(g, w, c)),
                            ),
                    );
                }
            }
            if count > 0 {
                columns = columns.child(
                    resizable_panel()
                        .size(width * ratio + if col == 0 { px(1.) } else { -px(1.) })
                        .child(column),
                );
            }
        }
        div().flex_1().min_h_0().p(px(3.)).child(columns)
    }
    fn popup(&self, menu: Menu, w: &Window, c: &Context<Self>) -> Div {
        let p = self.palette();
        let vi = self.settings.locale == UiLocale::Vi;
        let mut list = div()
            .flex()
            .flex_col()
            .p(px(4.))
            .min_w(px(match menu {
                Menu::File => 270.,
                Menu::Layout => 225.,
                _ => 120.,
            }))
            .border_1()
            .border_color(p.edge)
            .shadow_md()
            .bg(p.surface);
        match menu {
            Menu::File => {
                for (en, local, handler) in [
                    (
                        "New document in another window",
                        "Tài liệu mới trong cửa sổ khác",
                        Self::new_file as UiHandler,
                    ),
                    ("Open…", "Mở…", Self::open),
                    ("Save", "Lưu", Self::save),
                    ("Save As…", "Lưu thành…", Self::save_as),
                    ("Export…", "Xuất…", Self::export),
                    ("Recover…", "Khôi phục…", Self::restore),
                ] {
                    list = list.child(self.tool(if vi { local } else { en }, false, c, handler));
                }
                list = list.child(self.tool(
                    if vi {
                        "Tìm / thay thế…"
                    } else {
                        "Find / replace…"
                    },
                    false,
                    c,
                    Self::find,
                ));
                list = list.child(self.tool(
                    if vi {
                        "Đóng tài liệu"
                    } else {
                        "Close document"
                    },
                    false,
                    c,
                    |s, w, c| {
                        if !s.dirty
                            || confirm(
                                "Close document?",
                                "Unsaved changes will be available under Recover.",
                            )
                        {
                            if s.dirty {
                                s.recover(c)?;
                            }
                            w.remove_window();
                        }
                        Ok(())
                    },
                ));
            }
            Menu::Layout => {
                list = list.child(self.tool(
                    if vi {
                        "Khôi phục bố cục"
                    } else {
                        "Reset layout"
                    },
                    false,
                    c,
                    |s, _, _| {
                        let generation = s.layout.generation + 1;
                        s.layout = Layout::default();
                        s.layout.generation = generation;
                        s.save_layout();
                        Ok(())
                    },
                ));
                for pane in PANES {
                    list = list.child(self.tool(
                        format!(
                            "{}   {}",
                            if self.layout.dock.visible[pane as usize] {
                                "☑"
                            } else {
                                "☐"
                            },
                            self.label(pane)
                        ),
                        false,
                        c,
                        move |s, _, _| {
                            s.layout.toggle(pane);
                            s.save_layout();
                            Ok(())
                        },
                    ));
                }
            }
            Menu::Language => {
                for (v, en, local) in [
                    (SourceLanguage::Zh, "Chinese", "Tiếng Trung"),
                    (SourceLanguage::Ja, "Japanese", "Tiếng Nhật"),
                ] {
                    list = list.child(self.tool(
                        if vi { local } else { en },
                        false,
                        c,
                        move |s, w, c| {
                            if s.project.source_language != v {
                                s.project.source_language = v;
                                s.dirty = true;
                                s.invalidate();
                                s.recover(c)?;
                                s.translate(w, c)?;
                            }
                            Ok(())
                        },
                    ));
                }
            }
            Menu::Size => {
                for size in [12, 14, 16, 18, 20, 24, 28, 32] {
                    list = list.child(self.tool(format!("{size} px"), false, c, move |s, w, c| {
                        s.layout.font = size;
                        s.apply_format("fontSize", Some(format!("{size}px")), w, c)
                    }));
                }
            }
            Menu::Color => {
                for color in ["#24282d", "#a82626", "#27629d", "#156744", "#805622"] {
                    list = list.child(self.tool(color, false, c, move |s, w, c| {
                        s.layout.color = color;
                        s.apply_format("color", Some(color.into()), w, c)
                    }));
                }
            }
            Menu::Align => {
                for align in ["left", "center", "right", "justify"] {
                    list = list.child(self.tool(align, false, c, move |s, w, c| {
                        s.layout.align = align;
                        s.apply_format("textAlign", Some(align.into()), w, c)
                    }));
                }
            }
        }
        let (x, y) = match menu {
            Menu::File => (px(4.), px(32.)),
            Menu::Layout => (px(45.), px(32.)),
            Menu::Language => (px(530.), px(32.)),
            _ => {
                let mouse = self.layout.anchor;
                (mouse.x, mouse.y.min(w.viewport_size().height - px(220.)))
            }
        };
        div()
            .absolute()
            .inset_0()
            .child(div().id("dismiss-menu").absolute().inset_0().on_mouse_down(
                MouseButton::Left,
                c.listener(|s, _, _, c| {
                    s.layout.menu = None;
                    c.notify()
                }),
            ))
            .child(list.absolute().left(x).top(y).occlude())
    }
    pub fn shell(&mut self, w: &mut Window, c: &mut Context<Self>) -> Div {
        let vi = self.settings.locale == UiLocale::Vi;
        let p = self.palette();
        let doc_name = self
            .path
            .as_ref()
            .and_then(|v| std::path::Path::new(v).file_name())
            .and_then(|v| v.to_str())
            .unwrap_or(if vi { "Chưa đặt tên" } else { "Untitled" });
        w.set_window_title(&format!(
            "{}{} · QuickTranslator",
            if self.dirty { "● " } else { "" },
            doc_name
        ));
        let cell = |s: String| div().px(px(8.)).border_r_1().border_color(p.edge).child(s);
        let status = div()
            .h(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .text_size(px(11.))
            .text_color(p.muted)
            .border_t_1()
            .border_color(p.edge)
            .child(cell(format!(
                "{} · {}",
                doc_name,
                match (vi, self.dirty) {
                    (true, true) => "Đã sửa",
                    (true, false) => "Đã lưu",
                    (false, true) => "Modified",
                    _ => "Saved",
                }
            )))
            .child(cell(format!(
                "{} {}",
                self.source.read(c).text().chars().count(),
                if vi {
                    "ký tự nguồn"
                } else {
                    "source characters"
                }
            )))
            .child(cell(format!(
                "{}:{}",
                self.source.read(c).cursor_position().line + 1,
                self.source.read(c).cursor_position().character + 1
            )))
            .child(cell(self.status.clone()))
            .child(div().flex_1())
            .child(cell(if vi {
                "Từ điển đã nạp".into()
            } else {
                "Dictionaries loaded".into()
            }))
            .child(cell(if vi {
                "Bộ tách từ Nhật đã nạp".into()
            } else {
                "Japanese tokenizer loaded".into()
            }))
            .child(cell(format!(
                "{} · {}",
                if vi {
                    "Đã kết nối ứng dụng gốc"
                } else {
                    "Native application connected"
                },
                env!("CARGO_PKG_VERSION")
            )));
        let mut view = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font_family(if cfg!(target_os = "linux") {
                "Noto Sans"
            } else if cfg!(target_os = "macos") {
                ".SystemUIFont"
            } else {
                "Segoe UI"
            })
            .text_size(px(13.))
            .bg(p.chrome)
            .text_color(p.ink)
            .child(self.toolbar(c))
            .child(self.workspace_dock(w, c))
            .child(status);
        if self.page != Page::Workspace {
            let (title, body) = match self.page {
                Page::Preferences => (
                    if vi {
                        "Cấu hình giao diện"
                    } else {
                        "Interface settings"
                    },
                    self.preference_page(c),
                ),
                Page::Export => (
                    if vi {
                        "Xuất tài liệu"
                    } else {
                        "Export document"
                    },
                    self.export_page(c),
                ),
                Page::Dictionaries => (
                    if vi { "Từ điển" } else { "Dictionaries" },
                    self.dictionary_page(c),
                ),
                _ => (
                    if vi { "Hồ sơ AI" } else { "AI profiles" },
                    self.profile_settings(c),
                ),
            };
            view = view.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(0.32))
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .p_3()
                            .gap_2()
                            .w(if matches!(self.page, Page::Preferences | Page::Export) {
                                px(430.)
                            } else {
                                px(1040.).min(w.viewport_size().width - px(28.))
                            })
                            .h(relative(0.80))
                            .border_1()
                            .border_color(p.edge)
                            .bg(p.surface)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(title)
                                    .child(self.tool("×", true, c, |s, _, _| {
                                        s.page = Page::Workspace;
                                        Ok(())
                                    })),
                            )
                            .child(body)
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(p.muted)
                                    .child(self.status.clone()),
                            ),
                    ),
            );
        }
        if let Some(menu) = self.layout.menu {
            view = view.child(self.popup(menu, w, c));
        }
        view
    }
}
