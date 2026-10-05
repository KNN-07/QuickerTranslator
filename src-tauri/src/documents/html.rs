use scraper::{ElementRef, Html};

/// Parse HTML locally; neither the parser nor this traversal fetches resources.
/// Only body text is imported, not HTML markup or executable content.
pub fn extract_visible_text(input: &str) -> String {
    let document = Html::parse_document(input);
    let mut output = VisibleText::default();
    let mut pending = vec![Visit::Element(document.root_element(), false)];
    while let Some(visit) = pending.pop() {
        match visit {
            Visit::Boundary(lines) => output.boundary(lines),
            Visit::Text(text, preformatted) => output.text(text, preformatted),
            Visit::Element(element, inherited_pre) => {
                let name = element.value().name();
                if excluded(element) {
                    continue;
                }
                if name == "br" {
                    output.hard_break();
                    continue;
                }
                let preformatted = inherited_pre || name == "pre"
                    || element.attr("style").is_some_and(preserves_whitespace);
                let lines = match name {
                    "p" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    | "blockquote" => 2,
                    "div" | "section" | "article" | "header" | "footer" | "main"
                    | "aside" | "nav" | "ul" | "ol" | "li" | "dl" | "dt" | "dd"
                    | "table" | "tr" | "hr" | "figure" | "figcaption" | "address" => 1,
                    _ => 0,
                };
                if lines != 0 {
                    output.boundary(lines);
                    pending.push(Visit::Boundary(lines));
                } else if matches!(name, "td" | "th") {
                    output.cell_boundary();
                }
                // A stack avoids overflowing the call stack on deeply nested input.
                for child in element.children().rev() {
                    if let Some(child_element) = ElementRef::wrap(child) {
                        pending.push(Visit::Element(child_element, preformatted));
                    } else if let Some(text) = child.value().as_text() {
                        pending.push(Visit::Text(text, preformatted));
                    }
                }
            }
        }
    }
    output.finish()
}

enum Visit<'a> {
    Element(ElementRef<'a>, bool),
    Text(&'a str, bool),
    Boundary(usize),
}

fn excluded(element: ElementRef<'_>) -> bool {
    matches!(element.value().name(), "head" | "script" | "style" | "template" | "noscript")
        || element.attr("hidden").is_some()
        || element.attr("style").is_some_and(|style| {
            style.split(';').any(|declaration| {
                let Some((property, value)) = declaration.split_once(':') else { return false };
                let value = value.split('!').next().unwrap_or_default().trim();
                (property.trim().eq_ignore_ascii_case("display") && value.eq_ignore_ascii_case("none"))
                    || (property.trim().eq_ignore_ascii_case("visibility")
                        && (value.eq_ignore_ascii_case("hidden") || value.eq_ignore_ascii_case("collapse")))
            })
        })
}

fn preserves_whitespace(style: &str) -> bool {
    style.split(';').any(|declaration| {
        let Some((property, value)) = declaration.split_once(':') else { return false };
        let value = value.split('!').next().unwrap_or_default().trim();
        property.trim().eq_ignore_ascii_case("white-space")
            && ["pre", "pre-wrap", "break-spaces"].iter().any(|expected| value.eq_ignore_ascii_case(expected))
    })
}

#[derive(Default)]
struct VisibleText {
    text: String,
    pending_space: bool,
    pending_lines: usize,
}

impl VisibleText {
    fn boundary(&mut self, lines: usize) {
        self.pending_space = false;
        self.pending_lines = self.pending_lines.max(lines);
    }

    fn flush_boundary(&mut self) {
        if self.pending_lines == 0 {
            return;
        }
        if !self.text.is_empty() {
            let existing = self.text.bytes().rev().take_while(|byte| *byte == b'\n').count();
            for _ in existing..self.pending_lines {
                self.text.push('\n');
            }
        }
        self.pending_lines = 0;
    }

    fn hard_break(&mut self) {
        self.flush_boundary();
        self.pending_space = false;
        self.text.push('\n');
    }

    fn cell_boundary(&mut self) {
        if self.pending_lines == 0 && !self.text.is_empty() && !self.text.ends_with(['\n', '\t']) {
            self.pending_space = false;
            self.text.push('\t');
        }
    }

    fn text(&mut self, text: &str, preformatted: bool) {
        if preformatted {
            if text.is_empty() {
                return;
            }
            self.flush_boundary();
            if self.pending_space && !self.text.is_empty() && !self.text.ends_with(['\n', '\t', ' ']) {
                self.text.push(' ');
            }
            self.pending_space = false;
            self.text.push_str(text);
            return;
        }
        for character in text.chars() {
            // HTML collapsible whitespace excludes non-breaking and ideographic spaces.
            if matches!(character, ' ' | '\t' | '\r' | '\n' | '\u{000c}') {
                self.pending_space = true;
                continue;
            }
            self.flush_boundary();
            if self.pending_space && !self.text.is_empty() && !self.text.ends_with(['\n', '\t', ' ']) {
                self.text.push(' ');
            }
            self.pending_space = false;
            self.text.push(character);
        }
    }

    fn finish(self) -> String {
        // Deferred boundaries avoid adding trailing paragraph separators, without
        // trimming significant leading/trailing whitespace inside <pre>.
        self.text
    }
}

#[cfg(test)]
mod tests {
    use super::extract_visible_text;

    #[test]
    fn entities_paragraphs_and_unicode_are_dom_text() {
        assert_eq!(
            extract_visible_text("<!doctype html><html><head><title>not body</title></head><body><p>Tiếng &amp; Việt&nbsp;日本語 &#x1f642;</p><p>a <b>b</b><br>c</p></body></html>"),
            "Tiếng & Việt\u{00a0}日本語 🙂\n\na b\nc"
        );
    }

    #[test]
    fn hidden_ancestors_and_executable_content_are_excluded() {
        assert_eq!(extract_visible_text(
            "<p>visible<script>bad</script><style>bad</style><template><p>bad</p></template><span hidden=false>bad</span><span style=' DISPLAY : NONE !important'><b>bad</b></span><span style='visibility: HIDDEN'>bad</span> end</p><img src='https://example.invalid/never-loaded'>"
        ), "visible end");
    }

    #[test]
    fn whitespace_collapses_except_preformatted_content() {
        assert_eq!(extract_visible_text(" <div> a\n\t b <i> c </i> d </div><pre>  e\n f  </pre><p>g</p>"),
            "a b c d\n\n  e\n f  \n\ng");
        assert_eq!(extract_visible_text("<span style='white-space:pre-wrap'> a  b\n c </span>"), " a  b\n c ");
    }

    #[test]
    fn malformed_markup_uses_html_tree_repair_and_table_boundaries() {
        assert_eq!(extract_visible_text("<p>one<p>two &lt;x&gt;"), "one\n\ntwo <x>");
        assert_eq!(extract_visible_text("<table><tr><td>中文</td><td>Việt</td></tr><tr><td>二</td><td>hai</td></tr></table>"), "中文\tViệt\n二\thai");
        assert_eq!(extract_visible_text("<p>a<br><br>b</p>"), "a\n\nb");
    }
}
