use std::{collections::HashMap, sync::Arc};

use regex::{CaptureLocations, Regex};

use crate::{
    dictionaries::import::{compile_rule, is_han},
    models::{AppError, AppResult, DictionaryKind, DictionaryProvenance, FullWrap, SingleWrap, SourceLanguage, TranslationAlgorithm, TranslationOptions, TranslationResult},
    state::{DictionaryEntry, DictionaryIndex, DictionarySnapshot, TranslationWork},
};

use super::{is_closing_punctuation, is_horizontal_space, normalize_punctuation, OutputAssembler, OutputStyle, OutputToken, SegmentInput};

const LOOKUP_LIMIT: usize = 20;

#[derive(Debug, Default)]
struct PrefixNode {
    children: HashMap<char, usize>,
    exact: Option<ExactEntry>,
    capture: Option<Arc<DictionaryEntry>>,
    ignored: Option<Arc<DictionaryEntry>>,
}

#[derive(Debug)]
struct ExactEntry {
    entry: Arc<DictionaryEntry>,
    name: bool,
}

#[derive(Debug)]
struct CompiledRule {
    expression: Regex,
    capture_slot: usize,
    entry: Arc<DictionaryEntry>,
}

/// Constructed once with each immutable resolved dictionary revision. Trie
/// terminals share dictionary entries; translation never allocates candidate keys.
#[derive(Debug)]
pub struct ChineseIndex {
    nodes: Vec<PrefixNode>,
    rules: Vec<CompiledRule>,
    max_ignored_length: usize,
}

impl ChineseIndex {
    pub fn new(entries: &DictionaryIndex, rule_algorithm: u8) -> AppResult<Self> {
        if !(1..=3).contains(&rule_algorithm) {
            return Err(AppError::new("invalidRuleAlgorithm", "ThuatToanNhan must be 1, 2 or 3."));
        }
        let mut index = Self { nodes: vec![PrefixNode::default()], rules: Vec::new(), max_ignored_length: 0 };
        let Some(dictionaries) = entries.get(&SourceLanguage::Zh) else { return Ok(index); };
        // Insert lowest priority first. Equal keys are replaced, never selected by
        // HashMap iteration order or by the provenance of a bundled entry.
        for kind in [DictionaryKind::VietPhrase, DictionaryKind::SecondaryNames, DictionaryKind::PrimaryNames] {
            if let Some(dictionary) = dictionaries.get(&kind) {
                for (key, entry) in dictionary {
                    if key.chars().count() > LOOKUP_LIMIT { continue; }
                    let terminal = index.insert(key);
                    index.nodes[terminal].exact = Some(ExactEntry {
                        entry: Arc::clone(entry),
                        name: matches!(kind, DictionaryKind::PrimaryNames | DictionaryKind::SecondaryNames),
                    });
                }
            }
        }
        let capture_kinds: &[DictionaryKind] = match rule_algorithm {
            1 => &[DictionaryKind::Pronouns],
            2 => &[DictionaryKind::SecondaryNames, DictionaryKind::PrimaryNames, DictionaryKind::Pronouns],
            3 => &[DictionaryKind::VietPhrase, DictionaryKind::SecondaryNames, DictionaryKind::PrimaryNames, DictionaryKind::Pronouns],
            _ => unreachable!("algorithm validated above"),
        };
        for kind in capture_kinds {
            if let Some(dictionary) = dictionaries.get(kind) {
                for (key, entry) in dictionary {
                    if key.chars().count() > LOOKUP_LIMIT { continue; }
                    let terminal = index.insert(key);
                    index.nodes[terminal].capture = Some(Arc::clone(entry));
                }
            }
        }
        if let Some(dictionary) = dictionaries.get(&DictionaryKind::Ignored) {
            for (key, entry) in dictionary {
                let length = key.chars().count();
                if length == 0 { continue; }
                index.max_ignored_length = index.max_ignored_length.max(length);
                let terminal = index.insert(key);
                index.nodes[terminal].ignored = Some(Arc::clone(entry));
            }
        }
        if let Some(dictionary) = dictionaries.get(&DictionaryKind::Rules) {
            let mut rules: Vec<_> = dictionary.iter().map(|(key, entry)| (key, entry, key.chars().count())).collect();
            rules.sort_unstable_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(right.0)));
            for (pattern, entry, _) in rules {
                let expression = compile_rule(pattern)?;
                let capture_slot = expression.capture_names().position(|name| name == Some("qt_capture"))
                    .ok_or_else(|| AppError::new("invalidDictionaryRule", "The rule must capture one nonempty {0} phrase."))?;
                index.rules.push(CompiledRule { expression, capture_slot, entry: Arc::clone(entry) });
            }
        }
        Ok(index)
    }

    fn insert(&mut self, key: &str) -> usize {
        let mut current = 0;
        for scalar in key.chars() {
            current = match self.nodes[current].children.get(&scalar) {
                Some(&child) => child,
                None => {
                    let child = self.nodes.len();
                    self.nodes.push(PrefixNode::default());
                    self.nodes[current].children.insert(scalar, child);
                    child
                }
            };
        }
        current
    }

    fn capture(&self, key: &str) -> Option<&DictionaryEntry> {
        let mut current = 0;
        for scalar in key.chars() { current = *self.nodes[current].children.get(&scalar)?; }
        self.nodes[current].capture.as_deref()
    }

    fn scan<'a>(&'a self, remaining: &str, work: &TranslationWork) -> AppResult<Scan<'a>> {
        let mut scan = Scan { ends: [0; LOOKUP_LIMIT], exact: [None; LOOKUP_LIMIT], length: 0, ignored: None };
        let mut current = Some(0);
        let limit = LOOKUP_LIMIT.max(self.max_ignored_length);
        for (position, (byte, scalar)) in remaining.char_indices().take(limit).enumerate() {
            check_cancelled(work)?;
            current = current.and_then(|node| self.nodes[node].children.get(&scalar).copied());
            let end = byte + scalar.len_utf8();
            if position < LOOKUP_LIMIT {
                scan.ends[position] = end;
                scan.length = position + 1;
                scan.exact[position] = current.and_then(|node| self.nodes[node].exact.as_ref());
            }
            if let Some(node) = current {
                if let Some(ignored) = self.nodes[node].ignored.as_deref() { scan.ignored = Some((end, ignored)); }
            } else if self.rules.is_empty() || position + 1 >= LOOKUP_LIMIT {
                break;
            }
        }
        Ok(scan)
    }

    fn candidate_policy(&self, remaining: &str, end: usize, length: usize, check_names: bool, options: &TranslationOptions, work: &TranslationWork) -> AppResult<CandidatePolicy> {
        if length == 1 { return Ok(CandidatePolicy { algorithm_allows: true, names_allow: true }); }
        let threshold = match options.algorithm {
            TranslationAlgorithm::LeftToRight => usize::MAX,
            TranslationAlgorithm::Longest => length,
            TranslationAlgorithm::LongestConditional => length.max(3),
        };
        if threshold == usize::MAX && !check_names {
            return Ok(CandidatePolicy { algorithm_allows: true, names_allow: true });
        }
        let mut names_allow = true;
        for (inside, _) in remaining[..end].char_indices().skip(1) {
            check_cancelled(work)?;
            let mut current = 0;
            for (position, scalar) in remaining[inside..].chars().take(LOOKUP_LIMIT).enumerate() {
                let Some(&child) = self.nodes[current].children.get(&scalar) else { break; };
                current = child;
                if let Some(candidate) = &self.nodes[current].exact {
                    let candidate_length = position + 1;
                    if candidate_length > threshold {
                        return Ok(CandidatePolicy { algorithm_allows: false, names_allow });
                    }
                    if check_names && candidate.name && candidate_length >= 2 {
                        names_allow = false;
                        if threshold == usize::MAX {
                            return Ok(CandidatePolicy { algorithm_allows: true, names_allow });
                        }
                    }
                }
            }
        }
        Ok(CandidatePolicy { algorithm_allows: true, names_allow })
    }

    fn rule<'a>(&'a self, surface: &str, locations: &mut [CaptureLocations], work: &TranslationWork) -> AppResult<Option<(&'a DictionaryEntry, &'a DictionaryEntry)>> {
        for (rule, locations) in self.rules.iter().zip(locations.iter_mut()) {
            check_cancelled(work)?;
            if rule.expression.captures_read(locations, surface).is_some() {
                if let Some((start, end)) = locations.get(rule.capture_slot) {
                    if let Some(entry) = self.capture(&surface[start..end]) { return Ok(Some((&rule.entry, entry))); }
                }
            }
        }
        Ok(None)
    }
}

struct Scan<'a> {
    ends: [usize; LOOKUP_LIMIT],
    exact: [Option<&'a ExactEntry>; LOOKUP_LIMIT],
    length: usize,
    ignored: Option<(usize, &'a DictionaryEntry)>,
}

struct CandidatePolicy {
    algorithm_allows: bool,
    names_allow: bool,
}

pub fn translate(work: &TranslationWork) -> AppResult<TranslationResult> {
    if work.source.language != SourceLanguage::Zh {
        return Err(AppError::new("invalidSourceLanguage", "The Chinese engine requires Chinese source language."));
    }
    check_cancelled(work)?;
    let index = work.dictionaries.chinese_index()?;
    let mut capture_locations: Vec<_> = index.rules.iter().map(|rule| rule.expression.capture_locations()).collect();
    let source = &work.source.text;
    let mut assembler = OutputAssembler::new(work, OutputStyle::Chinese);
    let mut start = 0;
    while start < source.len() {
        check_cancelled(work)?;
        let remaining = &source[start..];
        let scan = index.scan(remaining, work)?;
        if let Some((end, ignored)) = scan.ignored {
            assembler.push(SegmentInput {
                source_range: work.source.offsets.byte_range_to_utf16(start..start + end)?,
                surface: &remaining[..end],
                readings: OutputToken::literal(""),
                phrases: OutputToken::literal(""),
                single_meaning: OutputToken::literal(""),
                lemma: None, reading: None, part_of_speech: None,
                meanings: Vec::new(), provenance: ignored.provenance.clone(), unknown: false,
            })?;
            start += end;
            continue;
        }
        let mut selected = None;
        for position in (0..scan.length).rev() {
            check_cancelled(work)?;
            let end = scan.ends[position];
            let exact = scan.exact[position];
            if exact.is_none() && index.rules.is_empty() { continue; }
            let check_names = work.options.prioritize_names && exact.is_some_and(|entry| !entry.name);
            let policy = index.candidate_policy(remaining, end, position + 1, check_names, &work.options, work)?;
            if !policy.algorithm_allows { continue; }
            if policy.names_allow {
                if let Some(exact) = exact {
                    selected = Some((end, Selection::Exact(&exact.entry)));
                    break;
                }
            }
            // Rules share algorithm lookahead, but not exact-phrase name
            // rejection: their captured value may itself resolve to a name.
            if !index.rules.is_empty() {
                if let Some((rule, capture)) = index.rule(&remaining[..end], &mut capture_locations, work)? {
                    selected = Some((end, Selection::Rule { rule, capture }));
                    break;
                }
            }
        }
        let (end, selection) = selected.unwrap_or_else(|| {
            let scalar = remaining.chars().next().expect("nonempty remaining source");
            let han = is_han(scalar);
            let mut end = scalar.len_utf8();
            // Coalesce literal Latin words and horizontal whitespace when no
            // rule or dictionary prefix can begin inside them. This preserves
            // exact source text without allocating one segment per ASCII letter.
            if index.rules.is_empty() && !index.nodes[0].children.contains_key(&scalar)
                && (is_horizontal_space(scalar) || (scalar.is_alphanumeric() && !han)) {
                let whitespace = is_horizontal_space(scalar);
                for (byte, next) in remaining.char_indices().skip(1) {
                    if work.cancellation.is_cancelled() { break; }
                    let same_run = if whitespace { is_horizontal_space(next) } else { next.is_alphanumeric() && !is_han(next) };
                    if !same_run || index.nodes[0].children.contains_key(&next) { break; }
                    end = byte + next.len_utf8();
                }
            }
            let reading = if han { work.dictionaries.lookup(SourceLanguage::Zh, DictionaryKind::HanViet, &remaining[..end]) } else { None };
            (end, reading.map_or(Selection::Unchanged { unknown: han }, |entry| Selection::Han(entry)))
        });
        check_cancelled(work)?;
        let surface = &remaining[..end];
        let source_range = work.source.offsets.byte_range_to_utf16(start..start + end)?;
        let han_fallback = matches!(&selection, Selection::Han(_));
        match selection {
            Selection::Unchanged { unknown } => {
                assembler.push(SegmentInput {
                    source_range, surface,
                    readings: OutputToken::literal(surface), phrases: OutputToken::literal(surface), single_meaning: OutputToken::literal(surface),
                    lemma: None, reading: None, part_of_speech: None,
                    meanings: Vec::new(), provenance: Vec::new(), unknown,
                })?;
            }
            Selection::Exact(entry) | Selection::Han(entry) => {
                let meanings = alternatives(&entry.meanings);
                let mut provenance = entry.provenance.clone();
                let readings = readings_for(surface, entry, &work.dictionaries, &mut provenance);
                let reading = readings.known.then(|| readings.text.clone());
                let count = if han_fallback { 1 } else { meanings.len() };
                let phrase = if han_fallback { OutputToken::generated(&readings.text, wrap_full(work.options.full_wrap, count)) }
                    else if meanings.is_empty() { OutputToken::generated(surface, wrap_full(work.options.full_wrap, count)) }
                    else { OutputToken::all_meanings(wrap_full(work.options.full_wrap, count)) };
                let first = if han_fallback { OutputToken::generated(&readings.text, work.options.single_wrap == SingleWrap::All) }
                    else if meanings.is_empty() { OutputToken::generated(surface, work.options.single_wrap == SingleWrap::All) }
                    else { OutputToken::first_meaning(work.options.single_wrap == SingleWrap::All) };
                assembler.push(SegmentInput {
                    source_range, surface,
                    readings: OutputToken { text: &readings.text, generated: readings.known, ..OutputToken::literal("") },
                    phrases: phrase,
                    single_meaning: first,
                    lemma: None, reading, part_of_speech: entry.part_of_speech.clone(),
                    meanings, provenance, unknown: false,
                })?;
            }
            Selection::Rule { rule, capture } => {
                let captured = alternatives(&capture.meanings);
                let templates = alternatives(&rule.meanings);
                let mut meanings = Vec::with_capacity(captured.len() * templates.len());
                for meaning in captured {
                    for template in &templates { meanings.push(template.replace("{0}", &meaning)); }
                }
                let mut provenance = rule.provenance.clone();
                merge_provenance(&mut provenance, &capture.provenance);
                let readings = readings_for(surface, rule, &work.dictionaries, &mut provenance);
                let reading = readings.known.then(|| readings.text.clone());
                assembler.push(SegmentInput {
                    source_range, surface,
                    readings: OutputToken { text: &readings.text, generated: readings.known, ..OutputToken::literal("") },
                    phrases: OutputToken::all_meanings(wrap_full(work.options.full_wrap, meanings.len())),
                    single_meaning: OutputToken::first_meaning(work.options.single_wrap == SingleWrap::All),
                    lemma: None, reading, part_of_speech: None,
                    meanings, provenance, unknown: false,
                })?;
            }
        }
        start += end;
    }
    check_cancelled(work)?;
    Ok(assembler.finish())
}

enum Selection<'a> {
    Exact(&'a DictionaryEntry),
    Han(&'a DictionaryEntry),
    Rule { rule: &'a DictionaryEntry, capture: &'a DictionaryEntry },
    Unchanged { unknown: bool },
}

fn alternatives(values: &[String]) -> Vec<String> {
    values.iter().flat_map(|value| value.split(['/', '|']))
        .map(str::trim).filter(|meaning| !meaning.is_empty()).map(str::to_owned).collect()
}

fn wrap_full(wrap: FullWrap, count: usize) -> bool {
    wrap == FullWrap::All || (wrap == FullWrap::Ambiguous && count > 1)
}

struct Readings {
    text: String,
    known: bool,
}

fn readings_for(surface: &str, entry: &DictionaryEntry, dictionaries: &DictionarySnapshot, provenance: &mut Vec<DictionaryProvenance>) -> Readings {
    if let Some(reading) = entry.reading.as_deref()
        .map(|reading| reading.split(['/', '|']).next().unwrap_or(reading).trim())
        .filter(|reading| !reading.is_empty()) {
        return Readings { text: reading.to_owned(), known: true };
    }
    let mut text = String::new();
    let mut known = false;
    let mut last_generated = false;
    for (byte, scalar) in surface.char_indices() {
        let slice = &surface[byte..byte + scalar.len_utf8()];
        let reading = if is_han(scalar) { dictionaries.lookup(SourceLanguage::Zh, DictionaryKind::HanViet, slice) } else { None };
        let chosen = reading.and_then(|entry| {
            entry.reading.as_deref().filter(|reading| !reading.is_empty())
                .or_else(|| entry.meanings.first().map(String::as_str))
                .map(|reading| reading.split(['/', '|']).next().unwrap_or(reading).trim())
                .filter(|reading| !reading.is_empty())
        });
        let value = chosen.unwrap_or(slice);
        let generated = chosen.is_some();
        if !text.is_empty() && (generated || last_generated)
            && !text.ends_with(char::is_whitespace) && !value.starts_with(char::is_whitespace)
            && !is_closing_punctuation(normalize_punctuation(scalar)) {
            text.push(' ');
        }
        text.push_str(value);
        known |= generated;
        last_generated = generated;
        if let Some(entry) = reading { merge_provenance(provenance, &entry.provenance); }
    }
    Readings { text, known }
}

fn merge_provenance(target: &mut Vec<DictionaryProvenance>, additional: &[DictionaryProvenance]) {
    for item in additional {
        if !target.contains(item) { target.push(item.clone()); }
    }
}

fn check_cancelled(work: &TranslationWork) -> AppResult<()> {
    if work.cancellation.is_cancelled() {
        Err(AppError::new("translationCancelled", "Offline translation was cancelled."))
    } else {
        Ok(())
    }
}
