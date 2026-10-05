use std::{fmt::Write as _, io, path::Path};

use docx_rs::{
    AbstractNumbering, AlignmentType, BreakType, Docx, IndentLevel, Level, LevelJc,
    LevelText, LineSpacing, LineSpacingType, NumberFormat, Numbering, NumberingId,
    Paragraph, ParagraphBorder, ParagraphBorderPosition, ParagraphBorders, Run,
    RunFonts, Shading, SpecialIndentType, Start, Style, StyleType, Table, TableCell,
    TableRow,
};
use serde_json::Value;

use crate::{models::{AppError, AppResult}, storage::{atomic_replace, atomic_replace_with}};
use super::{ExportColumn, ExportFormat, ExportRequest, SaveResult};

pub fn export_document(request: &ExportRequest) -> AppResult<SaveResult> {
    request.project.validate()?;
    validate_selection(request)?;
    let path = Path::new(&request.path);
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
    let matches_format = match request.format {
        ExportFormat::Txt => extension.eq_ignore_ascii_case("txt"),
        ExportFormat::Html => extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm"),
        ExportFormat::Docx => extension.eq_ignore_ascii_case("docx"),
        ExportFormat::Rtf => extension.eq_ignore_ascii_case("rtf"),
    };
    if !matches_format {
        return Err(AppError::new("invalidExportDestination", "Choose a destination with the selected export format's extension. Project and legacy files cannot be overwritten by an export."));
    }
    let outcome = match request.format {
        ExportFormat::Txt => atomic_replace(path, super::schema::plain_text(&request.project.target_document).as_bytes())?,
        ExportFormat::Html => atomic_replace(path, render_html(request).as_bytes())?,
        ExportFormat::Docx => {
            validate_docx_text(request)?;
            let document = build_docx(request);
            atomic_replace_with(path, move |file| document.build().pack(file).map_err(io::Error::other))?
        }
        ExportFormat::Rtf => {
            let original = request.project.legacy_rtf.as_deref().ok_or_else(|| AppError::new(
                "rtfOriginalUnavailable",
                "RTF export can only preserve an imported original RTF. Export HTML or DOCX to include the current edited Vietnamese document.",
            ))?;
            atomic_replace(path, original.as_bytes())?
        }
    };
    Ok(SaveResult { path: request.path.clone(), directory_sync_confirmed: outcome.directory_sync_confirmed })
}

fn validate_selection(request: &ExportRequest) -> AppResult<()> {
    // Original RTF is an opaque preservation export, not a selected-column export.
    if matches!(request.format, ExportFormat::Rtf) {
        return Ok(());
    }
    if request.columns.is_empty() {
        return Err(AppError::new("invalidExportSelection", "Select at least one column to export."));
    }
    for (index, column) in request.columns.iter().enumerate() {
        if request.columns[..index].contains(column) {
            return Err(AppError::new("invalidExportSelection", "Each export column may be selected only once."));
        }
    }
    if matches!(request.format, ExportFormat::Txt)
        && (request.columns.len() != 1 || !matches!(request.columns[0], ExportColumn::Target)) {
        return Err(AppError::new("invalidExportSelection", "Plain text exports only the manually edited Vietnamese target. Select Target only, or choose HTML or DOCX for multiple columns."));
    }
    Ok(())
}

fn column_label(column: &ExportColumn) -> &'static str {
    match column {
        ExportColumn::Source => "Source", ExportColumn::Readings => "Readings",
        ExportColumn::Phrases => "Phrases", ExportColumn::SingleMeaning => "Single meaning",
        ExportColumn::Target => "Target",
    }
}

fn column_text<'a>(request: &'a ExportRequest, column: &ExportColumn) -> &'a str {
    match column {
        ExportColumn::Source => &request.project.source_text,
        ExportColumn::Readings => &request.readings,
        ExportColumn::Phrases => &request.phrases,
        ExportColumn::SingleMeaning => &request.single_meaning,
        ExportColumn::Target => unreachable!("the rich target is rendered from its editor document"),
    }
}

fn children(node: &Value) -> &[Value] {
    node.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn kind(node: &Value) -> &str { node.get("type").and_then(Value::as_str).unwrap_or_default() }
fn attribute<'a>(node: &'a Value, name: &str) -> Option<&'a str> { node.get("attrs")?.get(name)?.as_str() }
fn marks(node: &Value) -> &[Value] { node.get("marks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() }

fn escape_html(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"), '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"), '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"), _ => output.push(character),
        }
    }
}

fn render_html(request: &ExportRequest) -> String {
    let mut output = String::from("<!doctype html>\n<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>QuickTranslator export</title><style>body{font-family:system-ui,sans-serif;line-height:1.5;margin:2rem}table{border-collapse:collapse;table-layout:fixed;width:100%}th,td{border:1px solid #888;padding:.5rem;vertical-align:top;overflow-wrap:anywhere}th{text-align:left}.content{white-space:pre-wrap;overflow-wrap:anywhere}.content p,.content h1,.content h2,.content h3,.content h4,.content h5,.content h6,.content pre,.content blockquote,.content ul,.content ol{margin-top:0;margin-bottom:0}.content li{margin:0}.content pre{font-family:monospace;white-space:pre-wrap}.content blockquote{margin-left:1.5em;border-left:3px solid #aaa;padding-left:1em}.content p,.content h1,.content h2,.content h3,.content h4,.content h5,.content h6,.content pre,.content hr{");
    // lh measures actual line heights, including the paragraph's inherited font.
    write!(output, "margin-bottom:{}lh", request.blank_lines).expect("writing a String cannot fail");
    output.push_str("}</style></head><body>");
    if request.columns.len() > 1 {
        output.push_str("<table><thead><tr>");
        for column in &request.columns {
            output.push_str("<th scope=\"col\">");
            output.push_str(column_label(column));
            output.push_str("</th>");
        }
        output.push_str("</tr></thead><tbody><tr>");
        for column in &request.columns {
            output.push_str("<td><div class=\"content\">");
            render_html_column(request, column, &mut output);
            output.push_str("</div></td>");
        }
        output.push_str("</tr></tbody></table>");
    } else {
        output.push_str("<main class=\"content\">");
        render_html_column(request, &request.columns[0], &mut output);
        output.push_str("</main>");
    }
    output.push_str("</body></html>\n");
    output
}

fn render_html_column(request: &ExportRequest, column: &ExportColumn, output: &mut String) {
    if matches!(column, ExportColumn::Target) {
        render_html_node(&request.project.target_document, output);
    } else {
        for line in column_text(request, column).split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            output.push_str("<p>");
            escape_html(output, line);
            if line.is_empty() { output.push_str("<br>"); }
            output.push_str("</p>");
        }
    }
}

fn render_html_node(node: &Value, output: &mut String) {
    let tag = match kind(node) {
        "doc" => { for child in children(node) { render_html_node(child, output); } return; }
        "text" => {
            let node_marks = marks(node);
            for mark in node_marks { open_html_mark(mark, output); }
            escape_html(output, node.get("text").and_then(Value::as_str).unwrap_or_default());
            for mark in node_marks.iter().rev() { close_html_mark(mark, output); }
            return;
        }
        "hardBreak" => { output.push_str("<br>"); return; }
        "horizontalRule" => { output.push_str("<hr>"); return; }
        "paragraph" => "p", "blockquote" => "blockquote", "bulletList" => "ul",
        "orderedList" => "ol", "listItem" => "li", "codeBlock" => "pre",
        "heading" => match node.get("attrs").and_then(|attrs| attrs.get("level")).and_then(Value::as_u64).unwrap_or(1) {
            2 => "h2", 3 => "h3", 4 => "h4", 5 => "h5", 6 => "h6", _ => "h1",
        },
        _ => unreachable!("project validation rejects unsupported editor nodes"),
    };
    output.push('<'); output.push_str(tag);
    if let Some(alignment) = attribute(node, "textAlign") {
        output.push_str(" style=\"text-align:"); escape_html(output, alignment); output.push('"');
    }
    if kind(node) == "orderedList" {
        if let Some(start) = node.get("attrs").and_then(|attrs| attrs.get("start")).and_then(Value::as_i64) {
            write!(output, " start=\"{start}\"").expect("writing a String cannot fail");
        }
        if let Some(list_type) = attribute(node, "type") {
            output.push_str(" type=\""); escape_html(output, list_type); output.push('"');
        }
    }
    output.push('>');
    if kind(node) == "codeBlock" { output.push_str("<code>"); }
    for child in children(node) { render_html_node(child, output); }
    if children(node).is_empty() && matches!(kind(node), "paragraph" | "heading") { output.push_str("<br>"); }
    if kind(node) == "codeBlock" { output.push_str("</code>"); }
    output.push_str("</"); output.push_str(tag); output.push('>');
}

fn open_html_mark(mark: &Value, output: &mut String) {
    let tag = match kind(mark) {
        "bold" => "<strong>", "italic" => "<em>", "underline" => "<u>",
        "strike" => "<s>", "code" => "<code>",
        "textStyle" => {
            output.push_str("<span style=\"");
            for (attribute_name, css_name) in [("fontFamily", "font-family"), ("fontSize", "font-size"),
                ("color", "color"), ("backgroundColor", "background-color"), ("lineHeight", "line-height")] {
                // These values have already passed the editor schema's CSS allowlist.
                if let Some(value) = attribute(mark, attribute_name) {
                    output.push_str(css_name); output.push(':'); escape_html(output, value); output.push(';');
                }
            }
            output.push_str("\">");
            return;
        }
        _ => unreachable!("project validation rejects unsupported editor marks"),
    };
    output.push_str(tag);
}

fn close_html_mark(mark: &Value, output: &mut String) {
    output.push_str(match kind(mark) {
        "bold" => "</strong>", "italic" => "</em>", "underline" => "</u>",
        "strike" => "</s>", "code" => "</code>", "textStyle" => "</span>",
        _ => unreachable!("project validation rejects unsupported editor marks"),
    });
}

fn validate_docx_text(request: &ExportRequest) -> AppResult<()> {
    fn valid(text: &str) -> bool {
        text.chars().all(|ch| matches!(ch, '\t' | '\n' | '\r') || (ch >= ' ' && !matches!(ch, '\u{fffe}' | '\u{ffff}')))
    }
    fn valid_node(node: &Value) -> bool {
        node.get("text").and_then(Value::as_str).is_none_or(valid) && children(node).iter().all(valid_node)
    }
    for column in &request.columns {
        let allowed = if matches!(column, ExportColumn::Target) { valid_node(&request.project.target_document) }
            else { valid(column_text(request, column)) };
        if !allowed {
            return Err(AppError::new("invalidExportText", "The selected text contains control characters that cannot be represented in DOCX. Remove those characters or choose a plain-text export."));
        }
    }
    Ok(())
}

const HEADING_STYLES: [&str; 6] = ["Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6"];
const HEADING_SIZES: [usize; 6] = [40, 32, 28, 26, 24, 22];

fn build_docx(request: &ExportRequest) -> Docx {
    let mut document = Docx::new().default_size(24);
    for (index, style_id) in HEADING_STYLES.iter().enumerate() {
        document = document.add_style(Style::new(*style_id, StyleType::Paragraph)
            .name(format!("heading {}", index + 1)).based_on("Normal").bold()
            .size(HEADING_SIZES[index]).outline_lvl(index));
    }
    let mut renderer = DocxRenderer { blank_lines: request.blank_lines, numberings: Vec::new() };
    if request.columns.len() > 1 {
        let headers = request.columns.iter().map(|column| TableCell::new().add_paragraph(
            Paragraph::new().add_run(Run::new().bold().add_text(column_label(column))),
        )).collect();
        let mut cells = Vec::with_capacity(request.columns.len());
        for column in &request.columns {
            let mut cell = TableCell::new();
            for paragraph in renderer.column(request, column) { cell = cell.add_paragraph(paragraph); }
            cells.push(cell);
        }
        document = document.add_table(Table::new(vec![TableRow::new(headers), TableRow::new(cells)])
            .set_grid(vec![9000 / request.columns.len(); request.columns.len()]));
    } else {
        for paragraph in renderer.column(request, &request.columns[0]) { document = document.add_paragraph(paragraph); }
    }
    for (abstract_numbering, numbering) in renderer.numberings {
        document = document.add_abstract_numbering(abstract_numbering).add_numbering(numbering);
    }
    document
}

struct DocxRenderer {
    blank_lines: u8,
    numberings: Vec<(AbstractNumbering, Numbering)>,
}

impl DocxRenderer {
    fn paragraph(&self) -> Paragraph {
        Paragraph::new().line_spacing(LineSpacing::new().before(0).after(0)
            .before_lines(0).after_lines(u32::from(self.blank_lines) * 100))
    }

    fn column(&mut self, request: &ExportRequest, column: &ExportColumn) -> Vec<Paragraph> {
        let mut paragraphs = Vec::new();
        if matches!(column, ExportColumn::Target) {
            self.block(&request.project.target_document, 0, None, &mut paragraphs);
        } else {
            for line in column_text(request, column).split('\n') {
                paragraphs.push(self.paragraph().add_run(add_docx_text(Run::new(), line.strip_suffix('\r').unwrap_or(line))));
            }
        }
        if paragraphs.is_empty() { paragraphs.push(self.paragraph()); }
        paragraphs
    }

    fn block(&mut self, node: &Value, indent: usize, numbering: Option<usize>, output: &mut Vec<Paragraph>) {
        match kind(node) {
            "doc" | "listItem" => for child in children(node) { self.block(child, indent, None, output); },
            "blockquote" => for child in children(node) { self.block(child, indent + 1, None, output); },
            "bulletList" | "orderedList" => {
                let ordered = kind(node) == "orderedList";
                let start = node.get("attrs").and_then(|attrs| attrs.get("start")).and_then(Value::as_i64).unwrap_or(1);
                // docx-rs reserves numbering ID 1. Each list gets a distinct ID so
                // consecutive and nested ordered lists restart independently.
                let id = self.numberings.len() + 2;
                let format = if !ordered { "bullet" } else { match attribute(node, "type") {
                    Some("a") => "lowerLetter", Some("A") => "upperLetter",
                    Some("i") => "lowerRoman", Some("I") => "upperRoman", _ => "decimal",
                }};
                let native_numbering = !ordered || start >= 0;
                if native_numbering {
                    let level = Level::new(0, Start::new(if ordered { start as usize } else { 1 }),
                        NumberFormat::new(format), LevelText::new(if ordered { "%1." } else { "•" }), LevelJc::new("left"))
                        .indent(Some(((indent + 1) * 720) as i32), Some(SpecialIndentType::Hanging(360)), None, None);
                    self.numberings.push((AbstractNumbering::new(id).add_level(level), Numbering::new(id, id)));
                }
                for (item_index, item) in children(node).iter().enumerate() {
                    for (index, child) in children(item).iter().enumerate() {
                        let first = output.len();
                        self.block(child, indent + 1, if index == 0 && native_numbering { Some(id) } else { None }, output);
                        if index == 0 && !native_numbering {
                            // OOXML's builder accepts unsigned starts only; preserve
                            // negative HTML list starts as visible signed markers.
                            if let Some(paragraph) = output.get_mut(first) {
                                paragraph.children.insert(0, docx_rs::ParagraphChild::Run(Box::new(Run::new().add_text(format!("{}. ", start + item_index as i64)))));
                            }
                        }
                    }
                }
            }
            "paragraph" | "heading" | "codeBlock" | "horizontalRule" => {
                let mut paragraph = self.paragraph();
                if indent != 0 { paragraph = paragraph.indent(Some((indent * 720) as i32), None, None, None); }
                if let Some(id) = numbering { paragraph = paragraph.numbering(NumberingId::new(id), IndentLevel::new(0)); }
                if let Some(alignment) = attribute(node, "textAlign") {
                    paragraph = paragraph.align(match alignment { "center" => AlignmentType::Center,
                        "right" => AlignmentType::Right, "justify" => AlignmentType::Both, _ => AlignmentType::Left });
                }
                let heading = if kind(node) == "heading" {
                    Some(node.get("attrs").and_then(|attrs| attrs.get("level")).and_then(Value::as_u64).unwrap_or(1) as usize - 1)
                } else { None };
                if let Some(level) = heading { paragraph = paragraph.style(HEADING_STYLES[level]).outline_lvl(level); }
                if kind(node) == "horizontalRule" {
                    paragraph.property.borders = Some(ParagraphBorders::new().set(ParagraphBorder::new(ParagraphBorderPosition::Bottom)));
                } else {
                    for child in children(node) {
                        paragraph = paragraph.add_run(render_docx_run(child, kind(node) == "codeBlock", heading));
                    }
                    if let Some(value) = children(node).iter().flat_map(marks).find_map(|mark| attribute(mark, "lineHeight")) {
                        if let Some(spacing) = docx_line_spacing(value, self.blank_lines, children(node)) { paragraph = paragraph.line_spacing(spacing); }
                    }
                }
                output.push(paragraph);
            }
            _ => unreachable!("project validation rejects unsupported block nodes"),
        }
    }
}

fn fonts(family: &str) -> RunFonts {
    RunFonts::new().ascii(family).hi_ansi(family).east_asia(family).cs(family)
}

fn render_docx_run(node: &Value, code_block: bool, heading: Option<usize>) -> Run {
    let mut run = Run::new();
    if let Some(level) = heading { run = run.bold().size(HEADING_SIZES[level]); }
    if code_block { run = run.fonts(fonts("Consolas")); }
    for mark in marks(node) {
        run = match kind(mark) {
            "bold" => run.bold(), "italic" => run.italic(), "underline" => run.underline("single"),
            "strike" => run.strike(), "code" => run.fonts(fonts("Consolas")),
            "textStyle" => {
                if let Some(family) = attribute(mark, "fontFamily") {
                    let family = family.split(',').next().unwrap_or(family).trim().trim_matches(['\'', '"']);
                    run = run.fonts(fonts(family));
                }
                if let Some(points) = attribute(mark, "fontSize").and_then(css_points) { run = run.size((points * 2.0).round().max(1.0) as usize); }
                if let Some(color) = attribute(mark, "color").and_then(css_color_hex) { run = run.color(color); }
                if let Some(color) = attribute(mark, "backgroundColor").and_then(css_color_hex) { run = run.shading(Shading::new().fill(color)); }
                run
            }
            _ => unreachable!("project validation rejects unsupported marks"),
        };
    }
    if kind(node) == "hardBreak" { run.add_break(BreakType::TextWrapping) }
    else { add_docx_text(run, node.get("text").and_then(Value::as_str).unwrap_or_default()) }
}

fn add_docx_text(mut run: Run, text: &str) -> Run {
    let mut start = 0;
    let mut previous_cr = false;
    for (index, character) in text.char_indices() {
        if matches!(character, '\r' | '\n' | '\t') {
            if index > start { run = run.add_text(&text[start..index]); }
            if character == '\t' { run = run.add_tab(); }
            else if !(character == '\n' && previous_cr) { run = run.add_break(BreakType::TextWrapping); }
            start = index + character.len_utf8();
        }
        previous_cr = character == '\r';
    }
    if start < text.len() { run = run.add_text(&text[start..]); }
    run
}

fn css_points(value: &str) -> Option<f64> {
    let value = value.trim();
    for (unit, multiplier) in [("rem", 12.0), ("px", 0.75), ("pt", 1.0), ("em", 12.0), ("%", 0.12)] {
        if let Some(number) = value.strip_suffix(unit) {
            return number.trim().parse::<f64>().ok().filter(|value| value.is_finite() && *value > 0.0).map(|value| value * multiplier);
        }
    }
    None
}

fn docx_line_spacing(value: &str, blank_lines: u8, inline: &[Value]) -> Option<LineSpacing> {
    let value = value.trim();
    let spacing = LineSpacing::new().before(0).after(0).before_lines(0).after_lines(u32::from(blank_lines) * 100);
    if value == "normal" { return Some(spacing.line_rule(LineSpacingType::Auto).line(240)); }
    if let Ok(multiplier) = value.parse::<f64>() { return Some(spacing.line_rule(LineSpacingType::Auto).line((multiplier * 240.0).round() as i32)); }
    let font_size = inline.iter().flat_map(marks).find_map(|mark| attribute(mark, "fontSize").and_then(css_points)).unwrap_or(12.0);
    let points = if let Some(percent) = value.strip_suffix('%') { percent.parse::<f64>().ok()? * font_size / 100.0 }
        else if let Some(em) = value.strip_suffix("em").filter(|_| !value.ends_with("rem")) { em.parse::<f64>().ok()? * font_size }
        else { css_points(value)? };
    Some(spacing.line_rule(LineSpacingType::Exact).line((points * 20.0).round().max(1.0) as i32))
}

// CSS alpha is composited on white because Word run colors have no alpha channel.
fn css_color_hex(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let (rgb, alpha) = match hex.len() {
            3 | 4 => {
                let mut components = [255u8; 4];
                for (index, digit) in hex.chars().enumerate() { components[index] = digit.to_digit(16)? as u8 * 17; }
                ([components[0], components[1], components[2]], f64::from(components[3]) / 255.0)
            }
            6 | 8 => {
                let mut components = [255u8; 4];
                for (index, component) in hex.as_bytes().chunks_exact(2).enumerate() {
                    components[index] = u8::from_str_radix(std::str::from_utf8(component).ok()?, 16).ok()?;
                }
                ([components[0], components[1], components[2]], f64::from(components[3]) / 255.0)
            }
            _ => return None,
        };
        return Some(composite_color(rgb, alpha));
    }
    let lowercase;
    let value = if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        lowercase = value.to_ascii_lowercase();
        lowercase.as_str()
    } else { value };
    for (prefix, hsl) in [("rgb(", false), ("rgba(", false), ("hsl(", true), ("hsla(", true)] {
        if let Some(body) = value.strip_prefix(prefix).and_then(|body| body.strip_suffix(')')) {
            let mut parts = body.split(|ch: char| ch == ',' || ch == '/' || ch.is_ascii_whitespace()).filter(|part| !part.is_empty());
            let components = [parts.next()?, parts.next()?, parts.next()?];
            let alpha = match parts.next() { Some(value) => css_component(value, 1.0)?, None => 1.0 };
            if parts.next().is_some() { return None; }
            let rgb = if hsl {
                let hue = components[0].parse::<f64>().ok()?.rem_euclid(360.0) / 60.0;
                let saturation = css_component(components[1], 1.0)?.clamp(0.0, 1.0);
                let lightness = css_component(components[2], 1.0)?.clamp(0.0, 1.0);
                let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
                let x = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
                let (r, g, b) = match hue as u8 { 0 => (chroma, x, 0.0), 1 => (x, chroma, 0.0),
                    2 => (0.0, chroma, x), 3 => (0.0, x, chroma), 4 => (x, 0.0, chroma), _ => (chroma, 0.0, x) };
                let base = lightness - chroma / 2.0;
                [((r + base) * 255.0).round() as u8, ((g + base) * 255.0).round() as u8, ((b + base) * 255.0).round() as u8]
            } else {
                [css_component(components[0], 255.0)?.round().clamp(0.0, 255.0) as u8,
                    css_component(components[1], 255.0)?.round().clamp(0.0, 255.0) as u8,
                    css_component(components[2], 255.0)?.round().clamp(0.0, 255.0) as u8]
            };
            return Some(composite_color(rgb, alpha));
        }
    }
    Some(match value {
        "aliceblue" => "F0F8FF", "antiquewhite" => "FAEBD7", "aqua" | "cyan" => "00FFFF",
        "aquamarine" => "7FFFD4", "azure" => "F0FFFF", "beige" => "F5F5DC", "bisque" => "FFE4C4",
        "black" | "currentcolor" => "000000", "blanchedalmond" => "FFEBCD", "blue" => "0000FF",
        "blueviolet" => "8A2BE2", "brown" => "A52A2A", "burlywood" => "DEB887", "cadetblue" => "5F9EA0",
        "chartreuse" => "7FFF00", "chocolate" => "D2691E", "coral" => "FF7F50", "cornflowerblue" => "6495ED",
        "cornsilk" => "FFF8DC", "crimson" => "DC143C", "darkblue" => "00008B", "darkcyan" => "008B8B",
        "darkgoldenrod" => "B8860B", "darkgray" | "darkgrey" => "A9A9A9", "darkgreen" => "006400",
        "darkkhaki" => "BDB76B", "darkmagenta" => "8B008B", "darkolivegreen" => "556B2F", "darkorange" => "FF8C00",
        "darkorchid" => "9932CC", "darkred" => "8B0000", "darksalmon" => "E9967A", "darkseagreen" => "8FBC8F",
        "darkslateblue" => "483D8B", "darkslategray" | "darkslategrey" => "2F4F4F", "darkturquoise" => "00CED1",
        "darkviolet" => "9400D3", "deeppink" => "FF1493", "deepskyblue" => "00BFFF", "dimgray" | "dimgrey" => "696969",
        "dodgerblue" => "1E90FF", "firebrick" => "B22222", "floralwhite" => "FFFAF0", "forestgreen" => "228B22",
        "fuchsia" | "magenta" => "FF00FF", "gainsboro" => "DCDCDC", "ghostwhite" => "F8F8FF", "gold" => "FFD700",
        "goldenrod" => "DAA520", "gray" | "grey" => "808080", "green" => "008000", "greenyellow" => "ADFF2F",
        "honeydew" => "F0FFF0", "hotpink" => "FF69B4", "indianred" => "CD5C5C", "indigo" => "4B0082",
        "ivory" => "FFFFF0", "khaki" => "F0E68C", "lavender" => "E6E6FA", "lavenderblush" => "FFF0F5",
        "lawngreen" => "7CFC00", "lemonchiffon" => "FFFACD", "lightblue" => "ADD8E6", "lightcoral" => "F08080",
        "lightcyan" => "E0FFFF", "lightgoldenrodyellow" => "FAFAD2", "lightgray" | "lightgrey" => "D3D3D3",
        "lightgreen" => "90EE90", "lightpink" => "FFB6C1", "lightsalmon" => "FFA07A", "lightseagreen" => "20B2AA",
        "lightskyblue" => "87CEFA", "lightslategray" | "lightslategrey" => "778899", "lightsteelblue" => "B0C4DE",
        "lightyellow" => "FFFFE0", "lime" => "00FF00", "limegreen" => "32CD32", "linen" => "FAF0E6",
        "maroon" => "800000", "mediumaquamarine" => "66CDAA", "mediumblue" => "0000CD", "mediumorchid" => "BA55D3",
        "mediumpurple" => "9370DB", "mediumseagreen" => "3CB371", "mediumslateblue" => "7B68EE",
        "mediumspringgreen" => "00FA9A", "mediumturquoise" => "48D1CC", "mediumvioletred" => "C71585",
        "midnightblue" => "191970", "mintcream" => "F5FFFA", "mistyrose" => "FFE4E1", "moccasin" => "FFE4B5",
        "navajowhite" => "FFDEAD", "navy" => "000080", "oldlace" => "FDF5E6", "olive" => "808000",
        "olivedrab" => "6B8E23", "orange" => "FFA500", "orangered" => "FF4500", "orchid" => "DA70D6",
        "palegoldenrod" => "EEE8AA", "palegreen" => "98FB98", "paleturquoise" => "AFEEEE", "palevioletred" => "DB7093",
        "papayawhip" => "FFEFD5", "peachpuff" => "FFDAB9", "peru" => "CD853F", "pink" => "FFC0CB",
        "plum" => "DDA0DD", "powderblue" => "B0E0E6", "purple" => "800080", "rebeccapurple" => "663399",
        "red" => "FF0000", "rosybrown" => "BC8F8F", "royalblue" => "4169E1", "saddlebrown" => "8B4513",
        "salmon" => "FA8072", "sandybrown" => "F4A460", "seagreen" => "2E8B57", "seashell" => "FFF5EE",
        "sienna" => "A0522D", "silver" => "C0C0C0", "skyblue" => "87CEEB", "slateblue" => "6A5ACD",
        "slategray" | "slategrey" => "708090", "snow" => "FFFAFA", "springgreen" => "00FF7F",
        "steelblue" => "4682B4", "tan" => "D2B48C", "teal" => "008080", "thistle" => "D8BFD8",
        "tomato" => "FF6347", "turquoise" => "40E0D0", "violet" => "EE82EE", "wheat" => "F5DEB3",
        "white" | "transparent" => "FFFFFF", "whitesmoke" => "F5F5F5", "yellow" => "FFFF00", "yellowgreen" => "9ACD32",
        _ => return None,
    }.to_owned())
}

fn css_component(value: &str, scale: f64) -> Option<f64> {
    let number = if let Some(percent) = value.strip_suffix('%') { percent.parse::<f64>().ok()? * scale / 100.0 }
        else { value.parse::<f64>().ok()? };
    number.is_finite().then_some(number)
}

fn composite_color(rgb: [u8; 3], alpha: f64) -> String {
    let alpha = alpha.clamp(0.0, 1.0);
    let component = |value: u8| (f64::from(value) * alpha + 255.0 * (1.0 - alpha)).round() as u8;
    format!("{:02X}{:02X}{:02X}", component(rgb[0]), component(rgb[1]), component(rgb[2]))
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Cursor};
    use docx_rs::{DocumentChild, ParagraphChild, RunChild, TableCellContent, TableChild, TableRowChild};
    use scraper::{Html, Selector};
    use serde_json::json;
    use crate::{documents::Project, models::SourceLanguage};
    use super::*;

    fn request(format: ExportFormat, path: &Path) -> ExportRequest {
        let mut project = Project::new(SourceLanguage::Zh, "原文 中文 🙂".to_owned());
        project.target_document = json!({"type":"doc","content":[
            {"type":"paragraph","attrs":{"textAlign":"center"},"content":[
                {"type":"text","text":"Tiếng Việt 日本語 🙂 & <x> \"quoted\"","marks":[
                    {"type":"bold"},{"type":"italic"},{"type":"underline"},{"type":"strike"},
                    {"type":"textStyle","attrs":{"fontFamily":"Noto Sans CJK","fontSize":"12.5pt","color":"#ff0033","backgroundColor":"#ff0","lineHeight":"1.5"}}
                ]},
                {"type":"hardBreak"},{"type":"text","text":"dòng hai"}
            ]},
            {"type":"heading","attrs":{"level":2,"textAlign":"right"},"content":[{"type":"text","text":"Đề mục"}]},
            {"type":"orderedList","attrs":{"start":3,"type":"a"},"content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"Mục ba"}]},
                    {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"子項"}]}]}]}
                ]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"Mục bốn","marks":[{"type":"code"}]}]}]}
            ]},
            {"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"Trích dẫn"}]}]},
            {"type":"codeBlock","content":[{"type":"text","text":"let\tvalue = 1;\n第二行"}]},
            {"type":"horizontalRule"}
        ]});
        ExportRequest {
            path: path.to_string_lossy().into_owned(), format, project,
            columns: vec![ExportColumn::Target], readings: "âm đọc 日本語".into(),
            phrases: "Cụm từ & nghĩa".into(), single_meaning: "Một nghĩa 🙂".into(), blank_lines: 2,
        }
    }

    #[test]
    fn html_is_standalone_escaped_and_preserves_ordered_native_columns() {
        let mut request = request(ExportFormat::Html, Path::new("unused.html"));
        request.project.target_document = json!({"type":"doc","content":[{"type":"paragraph","content":[
            {"type":"text","text":"<script>alert('x')</script> & <img src=\"remote\"> 日本語 🙂","marks":[
                {"type":"bold"},{"type":"textStyle","attrs":{"fontFamily":"Noto \"CJK\"","color":"#123456"}}
            ]}
        ]}]});
        request.columns = vec![ExportColumn::Phrases, ExportColumn::Source, ExportColumn::Target, ExportColumn::Readings, ExportColumn::SingleMeaning];
        request.project.validate().unwrap();
        let rendered = render_html(&request);
        assert!(rendered.starts_with("<!doctype html>"));
        assert!(rendered.contains("<meta charset=\"utf-8\">"));
        assert!(rendered.contains("margin-bottom:2lh"));
        assert!(rendered.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"));
        assert!(rendered.contains("font-family:Noto &quot;CJK&quot;"));
        let document = Html::parse_document(&rendered);
        assert_eq!(document.select(&Selector::parse("script,img,iframe,link").unwrap()).count(), 0);
        let headers: Vec<_> = document.select(&Selector::parse("th").unwrap()).map(|node| node.text().collect::<String>()).collect();
        assert_eq!(headers, ["Phrases", "Source", "Target", "Readings", "Single meaning"]);
        let cells: Vec<_> = document.select(&Selector::parse("td").unwrap()).map(|node| node.text().collect::<String>()).collect();
        assert_eq!(cells[0], request.phrases);
        assert_eq!(cells[1], request.project.source_text);
        assert_eq!(cells[2], "<script>alert('x')</script> & <img src=\"remote\"> 日本語 🙂");
        assert_eq!(cells[3], request.readings);
        assert_eq!(cells[4], request.single_meaning);
    }

    #[test]
    fn html_preserves_rich_blocks_alignment_and_blank_line_settings() {
        let mut request = request(ExportFormat::Html, Path::new("unused.html"));
        request.project.validate().unwrap();
        let rendered = render_html(&request);
        assert!(rendered.contains("<strong><em><u><s><span"));
        assert!(rendered.contains("font-size:12.5pt;color:#ff0033;background-color:#ff0;line-height:1.5"));
        assert!(rendered.contains("<p style=\"text-align:center\">"));
        assert!(rendered.contains("<h2 style=\"text-align:right\">"));
        assert!(rendered.contains("<ol start=\"3\" type=\"a\">"));
        assert!(rendered.contains("<ul><li>"));
        assert!(rendered.contains("<blockquote>"));
        assert!(rendered.contains("<pre><code>let\tvalue = 1;\n第二行</code></pre>"));
        assert!(!rendered.contains("<table>"));
        request.blank_lines = 0;
        assert!(render_html(&request).contains("margin-bottom:0lh"));
        request.blank_lines = 255;
        assert!(render_html(&request).contains("margin-bottom:255lh"));
    }

    fn cell_text(cell: &TableCell) -> String {
        cell.children.iter().filter_map(|child| match child {
            TableCellContent::Paragraph(paragraph) => Some(paragraph.raw_text()),
            _ => None,
        }).collect::<Vec<_>>().join("\n")
    }

    fn table_cells(row: &TableChild) -> Vec<&TableCell> {
        let TableChild::TableRow(row) = row;
        row.cells.iter().map(|child| {
            let TableRowChild::TableCell(cell) = child;
            cell
        }).collect()
    }

    #[test]
    fn docx_xml_and_packed_consumer_preserve_formatting_unicode_and_columns() {
        let mut request = request(ExportFormat::Docx, Path::new("unused.docx"));
        request.columns = vec![ExportColumn::Readings, ExportColumn::Target, ExportColumn::Source, ExportColumn::SingleMeaning, ExportColumn::Phrases];
        request.project.validate().unwrap();
        let package = build_docx(&request).build();
        let xml = std::str::from_utf8(&package.document).unwrap();
        for boundary in ["<w:tbl>", "<w:b", "<w:i", "<w:u", "<w:strike",
            "w:ascii=\"Noto Sans CJK\"", "w:eastAsia=\"Noto Sans CJK\"", "w:val=\"25\"",
            "w:val=\"FF0033\"", "w:fill=\"FFFF00\"", "w:val=\"center\"", "w:val=\"right\"",
            "w:afterLines=\"200\"", "w:line=\"360\"", "w:val=\"Heading2\"", "<w:numPr>",
            "w:ascii=\"Consolas\"", "<w:tab", "<w:br", "<w:pBdr>", "&amp;", "&lt;x&gt;",
            "Tiếng Việt 日本語 🙂"] {
            assert!(xml.contains(boundary), "missing OOXML boundary: {boundary}");
        }
        let numbering = std::str::from_utf8(&package.numberings).unwrap();
        assert!(numbering.contains("w:val=\"lowerLetter\""));
        assert!(numbering.contains("w:val=\"bullet\""));
        assert!(numbering.contains("<w:start w:val=\"3\""));
        let mut archive = Cursor::new(Vec::new());
        package.pack(&mut archive).unwrap();
        let bytes = archive.into_inner();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        let parsed = docx_rs::read_docx(&bytes).unwrap();
        let table = parsed.document.children.iter().find_map(|child| match child {
            DocumentChild::Table(table) => Some(table),
            _ => None,
        }).unwrap();
        assert_eq!(table.rows.len(), 2);
        let headers: Vec<_> = table_cells(&table.rows[0]).into_iter().map(cell_text).collect();
        assert_eq!(headers, ["Readings", "Target", "Source", "Single meaning", "Phrases"]);
        let cells = table_cells(&table.rows[1]);
        assert_eq!(cell_text(cells[0]), request.readings);
        assert!(cell_text(cells[1]).contains("Tiếng Việt 日本語 🙂 & <x> \"quoted\"\ndòng hai"));
        assert!(cell_text(cells[1]).contains("let\tvalue = 1;\n第二行"));
        assert_eq!(cell_text(cells[2]), request.project.source_text);
        assert_eq!(cell_text(cells[3]), request.single_meaning);
        assert_eq!(cell_text(cells[4]), request.phrases);
        let paragraph = cells[1].children.iter().find_map(|child| match child {
            TableCellContent::Paragraph(paragraph) => Some(paragraph), _ => None,
        }).unwrap();
        let run = paragraph.children.iter().find_map(|child| match child {
            ParagraphChild::Run(run) => Some(run), _ => None,
        }).unwrap();
        assert!(run.run_property.bold.is_some());
        assert!(run.run_property.italic.is_some());
        assert!(run.run_property.underline.is_some());
        assert!(run.run_property.strike.is_some());
        assert!(run.children.iter().any(|child| matches!(child, RunChild::Text(text) if text.text.contains("日本語 🙂"))));
    }

    #[test]
    fn standalone_docx_streams_to_atomic_destination_and_is_readable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("translated.docx");
        fs::write(&path, b"old file").unwrap();
        let request = request(ExportFormat::Docx, &path);
        let saved = export_document(&request).unwrap();
        assert_eq!(saved.path, request.path);
        let bytes = fs::read(&path).unwrap();
        let parsed = docx_rs::read_docx(&bytes).unwrap();
        assert!(parsed.document.children.iter().any(|child| matches!(child, DocumentChild::Paragraph(_))));
        assert!(!parsed.document.children.iter().any(|child| matches!(child, DocumentChild::Table(_))));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn txt_exports_only_the_manual_utf8_target() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("target.txt");
        let mut request = request(ExportFormat::Txt, &path);
        request.project.target_document = json!({"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"Tự sửa 日本語 🙂"},{"type":"hardBreak"},{"type":"text","text":"dòng hai"}]},
            {"type":"paragraph","content":[{"type":"text","text":"dòng ba"}]}
        ]});
        request.blank_lines = 255;
        export_document(&request).unwrap();
        assert_eq!(fs::read(&path).unwrap(), "Tự sửa 日本語 🙂\ndòng hai\ndòng ba".as_bytes());
        request.columns.push(ExportColumn::Source);
        assert_eq!(export_document(&request).unwrap_err().code, "invalidExportSelection");
        assert_eq!(fs::read(&path).unwrap(), "Tự sửa 日本語 🙂\ndòng hai\ndòng ba".as_bytes());
    }

    #[test]
    fn invalid_selections_projects_extensions_and_xml_leave_old_files_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retained.docx");
        fs::write(&path, b"previous").unwrap();
        let mut request = request(ExportFormat::Docx, &path);
        request.columns.clear();
        assert_eq!(export_document(&request).unwrap_err().code, "invalidExportSelection");
        request.columns = vec![ExportColumn::Target, ExportColumn::Target];
        assert_eq!(export_document(&request).unwrap_err().code, "invalidExportSelection");
        request.columns = vec![ExportColumn::Target];
        request.project.target_document = json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"bad\u{0000}control"}]}]});
        assert_eq!(export_document(&request).unwrap_err().code, "invalidExportText");
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        let project_path = directory.path().join("future.qtp");
        fs::write(&project_path, b"untouched future project").unwrap();
        request.path = project_path.to_string_lossy().into_owned();
        request.project.target_document = super::super::schema::empty_target();
        assert_eq!(export_document(&request).unwrap_err().code, "invalidExportDestination");
        request.path = path.to_string_lossy().into_owned();
        request.project.version = 999;
        assert_eq!(export_document(&request).unwrap_err().code, "futureProjectVersion");
        assert_eq!(fs::read(&project_path).unwrap(), b"untouched future project");
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn rtf_requires_and_preserves_the_original_not_the_edited_target() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.rtf");
        fs::write(&path, b"previous").unwrap();
        let mut request = request(ExportFormat::Rtf, &path);
        request.columns.clear();
        let error = export_document(&request).unwrap_err();
        assert_eq!(error.code, "rtfOriginalUnavailable");
        assert!(error.message.contains("HTML or DOCX"));
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        let original = "{\\rtf1\\ansi\\uc1 Original \\u26085?\\u26412?\\u35486?}";
        request.project.legacy_rtf = Some(original.to_owned());
        export_document(&request).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    }

    #[test]
    fn css_colors_sizes_and_docx_whitespace_have_deterministic_boundaries() {
        assert_eq!(css_color_hex("#abc").as_deref(), Some("AABBCC"));
        assert_eq!(css_color_hex("#ff000080").as_deref(), Some("FF7F7F"));
        assert_eq!(css_color_hex("rgb(100%, 0%, 0%)").as_deref(), Some("FF0000"));
        assert_eq!(css_color_hex("hsl(120, 100%, 50%)").as_deref(), Some("00FF00"));
        assert_eq!(css_color_hex("rebeccapurple").as_deref(), Some("663399"));
        assert_eq!(css_points("16px"), Some(12.0));
        assert_eq!(css_points("12.5pt"), Some(12.5));
        assert_eq!(css_points("1.5em"), Some(18.0));
        let paragraph = Paragraph::new().add_run(add_docx_text(Run::new(), "a\r\nb\rc\nd\t🙂"));
        let package = Docx::new().add_paragraph(paragraph).build();
        let xml = std::str::from_utf8(&package.document).unwrap();
        assert_eq!(xml.matches("<w:br").count(), 3);
        assert_eq!(xml.matches("<w:tab").count(), 1);
        let mut archive = Cursor::new(Vec::new());
        package.pack(&mut archive).unwrap();
        let parsed = docx_rs::read_docx(&archive.into_inner()).unwrap();
        let paragraph = parsed.document.children.iter().find_map(|child| match child {
            DocumentChild::Paragraph(paragraph) => Some(paragraph), _ => None,
        }).unwrap();
        assert_eq!(paragraph.raw_text(), "a\nb\nc\nd\t🙂");
    }
}
