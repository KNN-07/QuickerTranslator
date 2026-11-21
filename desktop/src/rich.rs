//! Paragraph editing preserves the surrounding project tree, including imported lists.
use anyhow::{Result, ensure};
use quickertranslator_lib::documents::schema::plain_text;
use serde_json::{Value, json};
use std::ops::Range;

pub fn paragraphs(doc: &Value) -> Vec<Vec<usize>> {
    fn visit(node: &Value, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if matches!(
            node["type"].as_str(),
            Some("paragraph" | "heading" | "codeBlock")
        ) {
            out.push(path.clone());
            return;
        }
        if let Some(children) = node["content"].as_array() {
            for (i, child) in children.iter().enumerate() {
                path.push(i);
                visit(child, path, out);
                path.pop();
            }
        }
    }
    let mut out = vec![];
    visit(doc, &mut vec![], &mut out);
    out
}
pub fn block_mut<'a>(mut doc: &'a mut Value, path: &[usize]) -> &'a mut Value {
    for i in path {
        doc = &mut doc["content"][*i];
    }
    doc
}
pub fn block<'a>(mut doc: &'a Value, path: &[usize]) -> &'a Value {
    for i in path {
        doc = &doc["content"][*i];
    }
    doc
}
fn slice(block: &Value, range: Range<usize>) -> Vec<Value> {
    let mut result = vec![];
    let mut pos = 0;
    for node in block["content"].as_array().into_iter().flatten() {
        let text = node["text"].as_str().unwrap_or("\n");
        let end = pos + text.len();
        let a = range.start.max(pos);
        let b = range.end.min(end);
        if a < b {
            let mut node = node.clone();
            if node["type"] == "text" {
                node["text"] = json!(&text[a - pos..b - pos]);
            }
            result.push(node);
        }
        pos = end;
    }
    result
}
pub fn edit(block: &mut Value, new: &str) {
    let old = plain_text(block);
    if old == new {
        return;
    }
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    let mut nodes = slice(block, 0..prefix);
    let marks = nodes
        .last()
        .or_else(|| block["content"].get(0))
        .and_then(|n| n.get("marks"))
        .cloned();
    let insert = &new[prefix..new.len() - suffix];
    if block["type"] == "codeBlock" && !insert.is_empty() {
        nodes.push(json!({"type":"text","text":insert}));
    } else {
        for (i, line) in insert.split('\n').enumerate() {
            if i > 0 {
                nodes.push(json!({"type":"hardBreak"}));
            }
            if !line.is_empty() {
                let mut n = json!({"type":"text","text":line});
                if let Some(m) = &marks {
                    n["marks"] = m.clone();
                }
                nodes.push(n);
            }
        }
    }
    nodes.extend(slice(block, old.len() - suffix..old.len()));
    block["content"] = Value::Array(nodes);
}
pub fn toggle_mark(block: &mut Value, range: Range<usize>, kind: &str) -> Result<()> {
    ensure!(!range.is_empty(), "Select Vietnamese text to format");
    ensure!(
        block["type"] != "codeBlock",
        "Formatting is unavailable inside a code block"
    );
    let mut middle = slice(block, range.clone());
    let marked = |n: &Value| {
        n["marks"]
            .as_array()
            .is_some_and(|m| m.iter().any(|v| v["type"] == kind))
    };
    let remove = middle.iter().all(marked);
    for n in &mut middle {
        let mut marks = n["marks"].as_array().cloned().unwrap_or_default();
        marks.retain(|v| v["type"] != kind && (remove || v["type"] != "code"));
        if !remove {
            marks.push(json!({"type":kind}));
        }
        n["marks"] = Value::Array(marks);
    }
    let mut out = slice(block, 0..range.start);
    out.extend(middle);
    out.extend(slice(block, range.end..usize::MAX));
    block["content"] = Value::Array(out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_keep_other_marks_and_list_structure() {
        let mut doc = json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"Việt ","marks":[{"type":"bold"}]},{"type":"text","text":"語🙂"}]}]}]}]});
        let path = paragraphs(&doc).remove(0);
        let p = block_mut(&mut doc, &path);
        edit(p, "Việt Nam\n語🙂");
        toggle_mark(p, 0..5, "italic").unwrap();
        assert_eq!(plain_text(&doc), "Việt Nam\n語🙂");
        assert_eq!(doc["content"][0]["type"], "bulletList");
        quickertranslator_lib::documents::schema::validate_target(&doc).unwrap();
    }
}

/// Apply one textStyle attribute without removing other marks on the selection.
pub fn set_style(block: &mut Value, range: Range<usize>, key: &str, value: &str) -> Result<()> {
    ensure!(!range.is_empty(), "Select Vietnamese text to format");
    ensure!(
        block["type"] != "codeBlock",
        "Text styles are unavailable in a code block"
    );
    let mut middle = slice(block, range.clone());
    for n in &mut middle {
        let mut marks = n["marks"].as_array().cloned().unwrap_or_default();
        marks.retain(|m| m["type"] != "code");
        if let Some(style) = marks.iter_mut().find(|m| m["type"] == "textStyle") {
            style["attrs"][key] = json!(value);
        } else {
            marks.push(json!({"type":"textStyle","attrs":{(key):value}}));
        }
        n["marks"] = Value::Array(marks);
    }
    let mut nodes = slice(block, 0..range.start);
    nodes.extend(middle);
    nodes.extend(slice(block, range.end..usize::MAX));
    block["content"] = Value::Array(nodes);
    Ok(())
}
