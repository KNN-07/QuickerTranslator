use std::{collections::{BTreeMap, HashSet}, path::{Path, PathBuf}, sync::LazyLock};

use regex::{Regex, RegexBuilder};

use crate::models::{AppError, AppResult, DictionaryKind};
use super::{encoding::read_text, types::*};

pub struct ParsedImport {
    pub entries: BTreeMap<String, EntryRecord>,
    pub issues: Vec<ImportIssue>,
    pub duplicates: usize,
}

pub fn format_for(kind: DictionaryKind) -> ImportFormat {
    match kind { DictionaryKind::Cedict => ImportFormat::Cedict, DictionaryKind::Ignored => ImportFormat::Ignored, _ => ImportFormat::Legacy }
}

pub fn is_auxiliary(kind: DictionaryKind) -> bool {
    matches!(kind, DictionaryKind::Cedict | DictionaryKind::Babylon | DictionaryKind::LacViet | DictionaryKind::ThieuChuu | DictionaryKind::Auxiliary)
}

pub fn compile_rule(pattern: &str) -> AppResult<Regex> {
    if pattern.matches("{0}").count() != 1 {
        return Err(AppError::new("invalidRule", "A rule must contain exactly one {0} nonempty capture."));
    }
    let pattern = format!("\\A(?:{})\\z", pattern.replacen("{0}", "(?P<qt_capture>.+)", 1));
    let compiled = RegexBuilder::new(&pattern).size_limit(1 << 20).dfa_size_limit(1 << 20).build()
        .map_err(|_| AppError::new("invalidRule", "The rule uses an invalid, unsupported or excessively complex regular expression."))?;
    Ok(compiled)
}

pub fn is_han(character: char) -> bool {
    static HAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A\p{Han}\z").expect("static Han scalar grammar"));
    let mut scalar = [0; 4];
    HAN.is_match(character.encode_utf8(&mut scalar))
}

pub fn validate_entry(kind: DictionaryKind, entry: &EntryRecord) -> AppResult<()> {
    if entry.headword.trim().is_empty() || entry.headword.contains(['\n', '\r']) {
        return Err(AppError::new("invalidDictionaryEntry", "A headword must be nonempty and contain no newline."));
    }
    if kind != DictionaryKind::Ignored && (entry.meanings.is_empty() || entry.meanings.iter().any(|meaning| meaning.trim().is_empty())) {
        return Err(AppError::new("invalidDictionaryEntry", "Provide at least one nonempty meaning."));
    }
    if kind == DictionaryKind::HanViet && (entry.headword.chars().count() != 1 || !entry.headword.chars().next().is_some_and(is_han)) {
        return Err(AppError::new("invalidHanCharacter", "Hán Việt entries require one Han character."));
    }
    if kind == DictionaryKind::Rules { compile_rule(&entry.headword)?; }
    if entry.aliases.iter().any(|alias| alias.trim().is_empty() || alias.contains(['\n', '\r'])) {
        return Err(AppError::new("invalidDictionaryEntry", "Aliases must be nonempty text without newlines."));
    }
    Ok(())
}

pub fn split_meanings(value: &str) -> Vec<String> {
    value.split(['/', '|']).map(str::trim).filter(|meaning| !meaning.is_empty()).map(str::to_owned).collect()
}

pub fn parse_dictionary(text: &str, kind: DictionaryKind, format: ImportFormat) -> ParsedImport {
    let mut parsed = ParsedImport { entries: BTreeMap::new(), issues: Vec::new(), duplicates: 0 };
    let mut indexed_keys = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() || (format == ImportFormat::Cedict && line.trim_start().starts_with('#')) { continue; }
        let candidate = match format {
            ImportFormat::Ignored => Some(EntryRecord { headword: line.trim().to_owned(), meanings: Vec::new(), reading: None, pos: None, aliases: Vec::new(), source_urls: Vec::new(), payload: None }),
            ImportFormat::Legacy => {
                match line.split_once('=') {
                    Some((key, value)) if !value.contains('=') => {
                        if key.trim().is_empty() {
                            issue(&mut parsed.issues, line_number, "emptyKey", "A dictionary key is empty."); None
                        } else if value.trim().is_empty() && kind != DictionaryKind::Ignored {
                            issue(&mut parsed.issues, line_number, "emptyMeaning", "A dictionary value is empty."); None
                        } else {
                            let meanings = if is_auxiliary(kind) { vec![value.to_owned()] } else { split_meanings(value) };
                            let reading = if kind == DictionaryKind::HanViet { meanings.first().cloned() } else { None };
                            Some(EntryRecord { headword: key.to_owned(), meanings, reading, pos: None, aliases: Vec::new(), source_urls: Vec::new(), payload: Some(value.to_owned()) })
                        }
                    },
                    _ => { issue(&mut parsed.issues, line_number, "malformedLine", "A legacy entry must contain exactly one equals sign."); None }
                }
            },
            ImportFormat::Cedict => parse_cedict(line).or_else(|| {
                issue(&mut parsed.issues, line_number, "invalidCedict", "Expected traditional simplified [pinyin] /definitions/."); None
            }),
        };
        let Some(entry) = candidate else { continue; };
        if let Err(error) = validate_entry(kind, &entry) {
            issue(&mut parsed.issues, line_number, &error.code, &error.message); continue;
        }
        if indexed_keys.contains(&entry.headword) {
            parsed.duplicates += 1;
            issue(&mut parsed.issues, line_number, "duplicate", "The first entry with this exact key is retained.");
            continue;
        }
        indexed_keys.insert(entry.headword.clone());
        // CEDict aliases do not steal an earlier record's spelling.
        let mut entry = entry;
        entry.aliases.retain(|alias| indexed_keys.insert(alias.clone()));
        parsed.entries.insert(entry.headword.clone(), entry);
    }
    parsed
}

fn parse_cedict(line: &str) -> Option<EntryRecord> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([^\s]+)\s+([^\s]+)\s+\[([^\]]+)\]\s+/(.+)/\s*$").expect("static CEDict grammar"));
    let capture = PATTERN.captures(line)?;
    let traditional = capture.get(1)?.as_str();
    let simplified = capture.get(2)?.as_str();
    let payload = capture.get(4)?.as_str();
    Some(EntryRecord { headword: traditional.to_owned(), meanings: vec![payload.to_owned()], reading: Some(capture.get(3)?.as_str().to_owned()), pos: None,
        aliases: if traditional != simplified { vec![simplified.to_owned()] } else { Vec::new() }, source_urls: Vec::new(), payload: Some(payload.to_owned()) })
}

pub fn preview_config(request: &ConfigRequest) -> AppResult<ConfigPreview> {
    let decoded = read_text(Path::new(&request.path), request.encoding.as_deref())?;
    let directory = Path::new(&request.path).parent().unwrap_or_else(|| Path::new("."));
    let mut preview = ConfigPreview { encoding: decoded.encoding, encoding_detected: decoded.detected, decoded_text: preview_text(&decoded.text), rule_algorithm: 1, dictionaries: Vec::new(), issues: Vec::new() };
    let mut seen = HashSet::new();
    for (index, raw_line) in decoded.text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) || (line.starts_with('[') && line.ends_with(']')) { continue; }
        let Some((key, value)) = line.split_once('=').filter(|(_, value)| !value.contains('=')) else {
            issue(&mut preview.issues, index + 1, "malformedLine", "A configuration line must contain exactly one equals sign."); continue;
        };
        let key = key.trim();
        if !seen.insert(key.to_owned()) {
            issue(&mut preview.issues, index + 1, "duplicate", "The first configuration value is retained."); continue;
        }
        if key == "ThuatToanNhan" {
            match value.trim().parse::<u8>() { Ok(value @ 1..=3) => preview.rule_algorithm = value, _ => issue(&mut preview.issues, index + 1, "invalidRuleAlgorithm", "ThuatToanNhan must be 1, 2 or 3.") }
            continue;
        }
        let Some(kind) = config_kind(key) else {
            issue(&mut preview.issues, index + 1, "unknownConfigKey", "This configuration key is not a supported dictionary."); continue;
        };
        let selected = request.remappings.get(key).map(String::as_str).unwrap_or(value.trim());
        let resolved = resolve_path(directory, selected);
        let (resolved_path, problem) = match resolved {
            Some(path) if path.is_file() => (Some(path.to_string_lossy().into_owned()), None),
            Some(_) => (None, Some("missingPath".to_owned())),
            None => (None, Some("foreignDrivePath".to_owned())),
        };
        preview.dictionaries.push(ConfigDictionary { key: key.to_owned(), kind, original_path: value.trim().to_owned(), resolved_path, problem });
    }
    Ok(preview)
}

pub fn resolve_path(directory: &Path, value: &str) -> Option<PathBuf> {
    if value.is_empty() { return Some(directory.join("")); }
    let value = value.replace('\\', "/");
    let bytes = value.as_bytes();
    #[cfg(not(windows))]
    if (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':') || value.starts_with("//") { return None; }
    #[cfg(windows)]
    let _ = bytes;
    let path = Path::new(&value);
    Some(if path.is_absolute() { path.to_owned() } else { directory.join(path) })
}

fn config_kind(key: &str) -> Option<DictionaryKind> {
    Some(match key {
        "Names" => DictionaryKind::PrimaryNames, "NamesPhu" => DictionaryKind::SecondaryNames,
        "VietPhrase" => DictionaryKind::VietPhrase, "ChinesePhienAmWords" => DictionaryKind::HanViet,
        "ChinesePhienAmEnglishWords" => DictionaryKind::Auxiliary, "CEDict" => DictionaryKind::Cedict,
        "Babylon" => DictionaryKind::Babylon, "LacViet" => DictionaryKind::LacViet, "ThieuChuu" => DictionaryKind::ThieuChuu,
        "IgnoredChinesePhrases" => DictionaryKind::Ignored, "LuatNhan" => DictionaryKind::Rules, "Pronouns" => DictionaryKind::Pronouns,
        _ => return None,
    })
}

pub fn preview_text(text: &str) -> String { text.chars().take(8000).collect() }

pub fn issue(issues: &mut Vec<ImportIssue>, line: usize, code: &str, message: &str) {
    issues.push(ImportIssue { line, code: code.to_owned(), message: message.to_owned() });
}

pub fn parse_shortcuts(text: &str) -> (BTreeMap<String, String>, Vec<ImportIssue>, usize) {
    let mut entries = BTreeMap::new();
    let mut issues = Vec::new();
    let mut duplicates = 0;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() { continue; }
        let Some((key, value)) = line.split_once('=').filter(|(_, value)| !value.contains('=')) else {
            issue(&mut issues, index + 1, "malformedLine", "A shortcut must contain exactly one equals sign."); continue;
        };
        if key.trim().is_empty() { issue(&mut issues, index + 1, "emptyKey", "A shortcut key is empty."); continue; }
        let key = key.to_lowercase();
        if entries.contains_key(&key) { duplicates += 1; issue(&mut issues, index + 1, "duplicate", "The first lowercased shortcut key is retained."); }
        else { entries.insert(key, value.to_owned()); }
    }
    (entries, issues, duplicates)
}
