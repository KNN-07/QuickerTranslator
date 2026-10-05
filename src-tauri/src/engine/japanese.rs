use std::{borrow::Cow, collections::HashMap, sync::Arc};

use lindera::token::Token;

use crate::{
    models::{AppError, AppResult, DictionaryKind, DictionaryLayer, FullWrap, SingleWrap, SourceLanguage, TranslationResult},
    state::{DictionaryEntry, DictionaryIndex, DictionarySnapshot, TranslationWork},
};
use super::{OutputAssembler, OutputStyle, OutputToken, SegmentInput};

#[derive(Debug, Default)]
struct PhraseNode {
    children: HashMap<char, usize>,
    entry: Option<Arc<DictionaryEntry>>,
}

/// Only user Japanese phrases/names enter this index. No Chinese length limit,
/// matching algorithm, ignored phrases, readings or rules are shared.
#[derive(Debug)]
pub struct JapaneseIndex {
    nodes: Vec<PhraseNode>,
}

impl JapaneseIndex {
    pub fn new(entries: &DictionaryIndex) -> Self {
        let mut index = Self { nodes: vec![PhraseNode::default()] };
        if let Some(dictionaries) = entries.get(&SourceLanguage::Ja) {
            for kind in [DictionaryKind::Japanese, DictionaryKind::SecondaryNames, DictionaryKind::PrimaryNames] {
                if let Some(dictionary) = dictionaries.get(&kind) {
                    for (key, entry) in dictionary {
                        if key.is_empty() || !entry.provenance.iter().any(|source| source.layer != DictionaryLayer::Bundled) { continue; }
                        let mut node = 0;
                        for scalar in key.chars() {
                            node = match index.nodes[node].children.get(&scalar) {
                                Some(&child) => child,
                                None => {
                                    let child = index.nodes.len();
                                    index.nodes.push(PhraseNode::default());
                                    index.nodes[node].children.insert(scalar, child);
                                    child
                                }
                            };
                        }
                        index.nodes[node].entry = Some(Arc::clone(entry));
                    }
                }
            }
        }
        index
    }

    fn longest<'a>(&'a self, work: &TranslationWork, tokens: &[Token<'_>], first: usize) -> AppResult<Option<(usize, &'a DictionaryEntry)>> {
        let start = tokens[first].byte_start;
        let mut node = 0;
        let mut boundary = first;
        let mut found = None;
        for (relative, scalar) in work.source.text[start..].char_indices() {
            check_cancelled(work)?;
            let Some(&child) = self.nodes[node].children.get(&scalar) else { break; };
            node = child;
            let end = start + relative + scalar.len_utf8();
            while boundary < tokens.len() && tokens[boundary].byte_end < end { boundary += 1; }
            if boundary == tokens.len() { break; }
            if tokens[boundary].byte_end == end {
                if let Some(entry) = self.nodes[node].entry.as_deref() { found = Some((boundary, entry)); }
            }
        }
        Ok(found)
    }
}

struct TokenFields<'a> {
    lemma: Option<&'a str>,
    reading: Option<&'a str>,
    pos: [Option<&'a str>; 4],
}

fn token_fields<'a>(token: &'a mut Token<'_>) -> TokenFields<'a> {
    let mut fields = TokenFields { lemma: None, reading: None, pos: [None; 4] };
    for (index, value) in token.details_iter().enumerate() {
        let present = (!value.is_empty() && value != "*" && value != "UNK").then_some(value);
        match index {
            0..=3 => fields.pos[index] = present,
            6 => fields.lemma = present,
            7 => fields.reading = present,
            _ => {}
        }
    }
    fields
}

fn append_hiragana(output: &mut String, reading: &str) {
    for scalar in reading.chars() {
        match scalar {
            '\u{30a1}'..='\u{30f6}' => output.push(char::from_u32(scalar as u32 - 0x60).unwrap()),
            'ヷ' => output.push_str("わ゙"),
            'ヸ' => output.push_str("ゐ゙"),
            'ヹ' => output.push_str("ゑ゙"),
            'ヺ' => output.push_str("を゙"),
            _ => output.push(scalar),
        }
    }
}

fn hiragana(reading: &str) -> String {
    let mut output = String::with_capacity(reading.len());
    append_hiragana(&mut output, reading);
    output
}

fn lookup<'a>(snapshot: &'a DictionarySnapshot, key: &str) -> Option<&'a DictionaryEntry> {
    [DictionaryKind::PrimaryNames, DictionaryKind::SecondaryNames, DictionaryKind::Japanese]
        .into_iter().find_map(|kind| snapshot.lookup(SourceLanguage::Ja, kind, key).map(Arc::as_ref))
}

fn push_literal(output: &mut OutputAssembler, work: &TranslationWork, start: usize, end: usize) -> AppResult<()> {
    let surface = &work.source.text[start..end];
    output.push(SegmentInput {
        source_range: work.source.offsets.byte_range_to_utf16(start..end)?, surface,
        readings: OutputToken::literal(surface), phrases: OutputToken::literal(surface), single_meaning: OutputToken::literal(surface),
        lemma: None, reading: None, part_of_speech: None, meanings: Vec::new(), provenance: Vec::new(), unknown: false,
    })
}

pub fn translate(work: &TranslationWork) -> AppResult<TranslationResult> {
    if work.source.language != SourceLanguage::Ja {
        return Err(AppError::new("invalidSourceLanguage", "The Japanese engine requires Japanese source language."));
    }
    check_cancelled(work)?;
    let tokenizer = work.tokenizer.as_ref().ok_or_else(|| AppError::new("tokenizerUnavailable", "The embedded Japanese IPADIC tokenizer is unavailable."))?;
    let source = &work.source.text;
    let mut tokens = tokenizer.segment(Cow::Borrowed(source)).map_err(|_| AppError::new("tokenizationFailed", "Japanese segmentation failed; the source and Vietnamese editor were preserved."))?;
    check_cancelled(work)?;
    let mut previous_end = 0;
    for token in &tokens {
        if token.byte_start < previous_end || token.byte_end <= token.byte_start || token.byte_end > source.len()
            || !source.is_char_boundary(token.byte_start) || !source.is_char_boundary(token.byte_end)
            || token.surface.as_ref() != &source[token.byte_start..token.byte_end] {
            return Err(AppError::new("invalidTokenizerSpan", "Japanese segmentation returned inconsistent source boundaries."));
        }
        previous_end = token.byte_end;
    }
    let phrase_index = work.dictionaries.japanese_index();
    let mut output = OutputAssembler::new(work, OutputStyle::Literal);
    let mut index = 0;
    let mut cursor = 0;
    while index < tokens.len() {
        check_cancelled(work)?;
        let start = tokens[index].byte_start;
        if start > cursor { push_literal(&mut output, work, cursor, start)?; }
        let custom = phrase_index.longest(work, &tokens, index)?;
        let last = custom.map_or(index, |(last, _)| last);
        let end = tokens[last].byte_end;
        let surface = &source[start..end];
        let lexical = surface.chars().any(char::is_alphanumeric);
        let mut lemma = None;
        let mut part_of_speech = None;
        let mut partial_reading = None;
        let mut reading;
        let entry;
        if last > index {
            entry = custom.map(|(_, entry)| entry);
            if let Some(value) = entry.and_then(|entry| entry.reading.as_deref()) {
                reading = Some(hiragana(value));
            } else {
                let mut display = String::with_capacity(surface.len());
                let mut complete = true;
                let mut prior = start;
                for token in &mut tokens[index..=last] {
                    if token.byte_start > prior { display.push_str(&source[prior..token.byte_start]); }
                    let token_start = token.byte_start;
                    let token_end = token.byte_end;
                    let fields = token_fields(token);
                    if let Some(value) = fields.reading { append_hiragana(&mut display, value); }
                    else { display.push_str(&source[token_start..token_end]); complete = false; }
                    prior = token_end;
                }
                if complete { reading = Some(display); }
                else { partial_reading = Some(display); reading = None; }
            }
        } else {
            let fields = token_fields(&mut tokens[index]);
            lemma = fields.lemma.map(str::to_owned);
            entry = if lexical {
                let exact = custom.map(|(_, entry)| entry).or_else(|| lookup(&work.dictionaries, surface))
                    .or_else(|| fields.lemma.and_then(|lemma| lookup(&work.dictionaries, lemma)));
                if exact.is_some() {
                    reading = exact.and_then(|entry| entry.reading.as_deref()).or(fields.reading).map(hiragana);
                    exact
                } else {
                    reading = fields.reading.map(hiragana);
                    let kana_entry = reading.as_deref().and_then(|reading| lookup(&work.dictionaries, reading))
                        .or_else(|| fields.reading.and_then(|reading| lookup(&work.dictionaries, reading)));
                    if let Some(value) = kana_entry.and_then(|entry| entry.reading.as_deref()) {
                        if reading.as_deref() != Some(value) && fields.reading != Some(value) { reading = Some(hiragana(value)); }
                    }
                    kana_entry
                }
            } else {
                reading = fields.reading.map(hiragana);
                None
            };
            if entry.and_then(|entry| entry.part_of_speech.as_ref()).is_none() {
                let mut pos = String::new();
                for value in fields.pos.into_iter().flatten() {
                    if !pos.is_empty() { pos.push('/'); }
                    pos.push_str(value);
                }
                if !pos.is_empty() { part_of_speech = Some(pos); }
            }
        }
        if let Some(value) = entry.and_then(|entry| entry.part_of_speech.as_ref()) { part_of_speech = Some(value.clone()); }
        let meanings = entry.map(|entry| entry.meanings.clone()).unwrap_or_default();
        let known = !meanings.is_empty();
        let readings = if reading.is_some() { OutputToken::reading() }
            else if let Some(partial) = partial_reading.as_deref() { OutputToken::generated(partial, false) }
            else { OutputToken::literal(surface) };
        let full_wrap = work.options.full_wrap == FullWrap::All || (work.options.full_wrap == FullWrap::Ambiguous && meanings.len() > 1);
        output.push(SegmentInput {
            source_range: work.source.offsets.byte_range_to_utf16(start..end)?, surface, readings,
            phrases: if known { OutputToken::all_meanings(full_wrap) } else { OutputToken::literal(surface) },
            single_meaning: if known { OutputToken::first_meaning(work.options.single_wrap == SingleWrap::All) } else { OutputToken::literal(surface) },
            lemma, reading, part_of_speech, meanings,
            provenance: entry.map(|entry| entry.provenance.clone()).unwrap_or_default(), unknown: lexical && !known,
        })?;
        cursor = end;
        index = last + 1;
    }
    if cursor < source.len() { push_literal(&mut output, work, cursor, source.len())?; }
    check_cancelled(work)?;
    Ok(output.finish())
}

fn check_cancelled(work: &TranslationWork) -> AppResult<()> {
    if work.cancellation.is_cancelled() { Err(AppError::new("translationCancelled", "Offline translation was cancelled.")) }
    else { Ok(()) }
}
