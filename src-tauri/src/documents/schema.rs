use serde_json::Value;

use crate::models::{AppError, AppResult};

fn invalid() -> AppError {
    AppError::new("invalidTargetDocument", "The Vietnamese document contains invalid or unsupported editor nodes, marks or attributes. It was not opened or saved.")
}

pub fn validate_target(document: &Value) -> AppResult<()> {
    if document.get("type").and_then(Value::as_str) != Some("doc") { return Err(invalid()); }
    validate_node(document, None, 0)
}

fn validate_node(node: &Value, parent: Option<&str>, depth: usize) -> AppResult<()> {
    if depth > 128 { return Err(invalid()); }
    let object = node.as_object().ok_or_else(invalid)?;
    if object.keys().any(|key| !matches!(key.as_str(), "type" | "attrs" | "content" | "marks" | "text")) { return Err(invalid()); }
    let kind = node.get("type").and_then(Value::as_str).ok_or_else(invalid)?;
    let blocks = matches!(kind, "paragraph" | "heading" | "blockquote" | "bulletList" | "orderedList" | "codeBlock" | "horizontalRule");
    match parent {
        None if kind != "doc" => return Err(invalid()),
        Some("doc" | "blockquote" | "listItem") if !blocks => return Err(invalid()),
        Some("paragraph" | "heading") if !matches!(kind, "text" | "hardBreak") => return Err(invalid()),
        Some("codeBlock") if kind != "text" => return Err(invalid()),
        Some("bulletList" | "orderedList") if kind != "listItem" => return Err(invalid()),
        Some("text" | "hardBreak" | "horizontalRule") => return Err(invalid()),
        _ => {}
    }
    if kind == "doc" && parent.is_some() { return Err(invalid()); }
    if !matches!(kind, "doc" | "paragraph" | "heading" | "blockquote" | "bulletList" | "orderedList" | "listItem" | "codeBlock" | "horizontalRule" | "hardBreak" | "text") { return Err(invalid()); }
    let children = match node.get("content") {
        None => &[][..],
        Some(value) => value.as_array().ok_or_else(invalid)?.as_slice(),
    };
    if matches!(kind, "doc" | "blockquote" | "bulletList" | "orderedList" | "listItem") && children.is_empty() { return Err(invalid()); }
    if kind == "listItem" && children.first().and_then(|child| child.get("type")).and_then(Value::as_str) != Some("paragraph") { return Err(invalid()); }
    if matches!(kind, "text" | "hardBreak" | "horizontalRule") && node.get("content").is_some() { return Err(invalid()); }
    if kind == "text" {
        if node.get("text").and_then(Value::as_str).is_none_or(str::is_empty) { return Err(invalid()); }
    } else if node.get("text").is_some() { return Err(invalid()); }
    if let Some(attrs) = node.get("attrs") {
        let attrs = attrs.as_object().ok_or_else(invalid)?;
        for (key, value) in attrs {
            match (kind, key.as_str()) {
                ("paragraph" | "heading", "textAlign") if value.is_null() || matches!(value.as_str(), Some("left" | "right" | "center" | "justify")) => {},
                ("heading", "level") if value.as_u64().is_some_and(|level| (1..=6).contains(&level)) => {},
                ("orderedList", "start") if value.as_i64().is_some_and(|start| i32::try_from(start).is_ok()) => {},
                ("orderedList", "type") if value.is_null() || matches!(value.as_str(), Some("1" | "a" | "A" | "i" | "I")) => {},
                ("codeBlock", "language") if value.is_null() || value.as_str().is_some_and(|text| safe_string(text, 128)) => {},
                _ => return Err(invalid()),
            }
        }
    }
    if let Some(marks) = node.get("marks") {
        let marks = marks.as_array().ok_or_else(invalid)?;
        if !matches!(kind, "text" | "hardBreak") || (parent == Some("codeBlock") && !marks.is_empty()) { return Err(invalid()); }
        let mut seen = std::collections::HashSet::new();
        for mark in marks {
            let mark = mark.as_object().ok_or_else(invalid)?;
            if mark.keys().any(|key| !matches!(key.as_str(), "type" | "attrs")) { return Err(invalid()); }
            let kind = mark.get("type").and_then(Value::as_str).ok_or_else(invalid)?;
            if !seen.insert(kind) { return Err(invalid()); }
            match kind {
                "bold" | "italic" | "underline" | "strike" | "code" => {
                    if let Some(attrs) = mark.get("attrs") { if !attrs.as_object().is_some_and(|attrs| attrs.is_empty()) { return Err(invalid()); } }
                },
                "textStyle" => {
                    if let Some(attrs) = mark.get("attrs") {
                        for (key, value) in attrs.as_object().ok_or_else(invalid)? {
                            if !matches!(key.as_str(), "fontFamily" | "fontSize" | "color" | "backgroundColor" | "lineHeight") { return Err(invalid()); }
                            if value.is_null() { continue; }
                            let text = value.as_str().ok_or_else(invalid)?;
                            let safe = match key.as_str() {
                                "fontFamily" => safe_string(text, 256) && !text.contains([';', '{', '}', '<', '>', '\\', '(', ')']),
                                "fontSize" => css_size(text, false),
                                "lineHeight" => text == "normal" || css_size(text, true),
                                _ => css_color(text),
                            };
                            if !safe { return Err(invalid()); }
                        }
                    }
                },
                _ => return Err(invalid()),
            }
        }
        if seen.contains("code") && seen.len() != 1 { return Err(invalid()); }
    }
    for child in children { validate_node(child, Some(kind), depth + 1)?; }
    Ok(())
}

fn safe_string(text: &str, limit: usize) -> bool {
    text.len() <= limit && !text.chars().any(char::is_control)
}

fn css_size(text: &str, allow_unitless: bool) -> bool {
    let text = text.trim();
    let number = ["rem", "px", "pt", "em", "%"].into_iter().find_map(|suffix| text.strip_suffix(suffix));
    let number = match number { Some(number) => number, None if allow_unitless => text, _ => return false };
    !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && number.parse::<f64>().is_ok_and(|value| value.is_finite() && value > 0.0 && value <= 10000.0)
}

fn css_color(text: &str) -> bool {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') { return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|ch| ch.is_ascii_hexdigit()); }
    if !text.is_empty() && text.len() <= 32 && text.bytes().all(|ch| ch.is_ascii_alphabetic()) { return true; }
    ["rgb(", "rgba(", "hsl(", "hsla("].into_iter().any(|prefix| {
        text.strip_prefix(prefix).and_then(|value| value.strip_suffix(')')).is_some_and(|body| {
            !body.is_empty() && body.len() < 128 && body.bytes().all(|ch| ch.is_ascii_digit() || matches!(ch, b' ' | b',' | b'.' | b'%' | b'/' | b'+' | b'-'))
        })
    })
}

pub fn empty_target() -> Value { serde_json::json!({"type":"doc","content":[{"type":"paragraph"}]}) }

pub fn plain_text(document: &Value) -> String {
    fn visit(node: &Value, output: &mut String) {
        let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "text" { if let Some(text) = node.get("text").and_then(Value::as_str) { output.push_str(text); } }
        if kind == "hardBreak" { output.push('\n'); }
        if let Some(children) = node.get("content").and_then(Value::as_array) {
            for (index, child) in children.iter().enumerate() {
                if index > 0 && matches!(kind, "doc" | "blockquote" | "listItem" | "bulletList" | "orderedList") { output.push('\n'); }
                visit(child, output);
            }
        }
    }
    let mut output = String::new();
    visit(document, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn preserves_formatting_and_rejects_links_unknown_attributes_and_invalid_tree() {
        let valid = json!({"type":"doc","content":[{"type":"paragraph","attrs":{"textAlign":"center"},"content":[{"type":"text","text":"Việt 🙂","marks":[{"type":"bold"},{"type":"textStyle","attrs":{"fontFamily":"Noto Sans CJK","fontSize":"12.5pt","color":"#ff0033"}}]}]}]});
        validate_target(&valid).unwrap();
        assert_eq!(plain_text(&valid), "Việt 🙂");
        for invalid in [json!({"type":"doc","content":[{"type":"text","text":"wrong"}]}), json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"link","marks":[{"type":"link","attrs":{"href":"https://example.test"}}]}]}]}), json!({"type":"doc","content":[{"type":"paragraph","attrs":{"onclick":"evil"}}]})] { assert!(validate_target(&invalid).is_err()); }
    }
    #[test]
    fn rejects_css_injection_and_empty_text_but_accepts_empty_paragraph() {
        validate_target(&empty_target()).unwrap();
        assert!(!css_size("12px;color:red", false));
        assert!(!css_color("url(https://example.test)"));
        assert!(validate_target(&serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":""}]}]})).is_err());
    }
}
