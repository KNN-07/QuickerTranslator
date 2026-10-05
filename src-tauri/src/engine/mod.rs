//! Offline output assembly shares UTF-16 alignment, not language normalization.
//! Chinese scanning uses immutable per-revision prefix/rule indexes and never
//! changes source text. `longest` rejects strictly longer interior matches;
//! `longestConditional` uses a minimum competing length of four; `leftToRight`
//! accepts the longest current match. Name preference is a separate exact-phrase
//! constraint. Japanese callers must opt into the literal punctuation/case path.
//!
//! Generated ranges exclude inserted token separators. Ignored and discarded
//! whitespace spans retain empty ranges. IDs depend only on language/source span;
//! consumers must additionally guard the result's source/dictionary revisions.

pub mod alignment;
pub mod chinese;
pub mod japanese;

use crate::{
    models::{AppError, AppResult, DictionaryProvenance, TextRange, TranslationResult, TranslationSegment},
    state::TranslationWork,
};

/// Chinese display conventions are deliberately opt-in. Japanese output must use Literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStyle {
    Chinese,
    Literal,
}

#[derive(Clone, Copy)]
pub struct OutputToken<'a> {
    pub text: &'a str,
    pub generated: bool,
    pub wrap: bool,
    projection: OutputProjection,
}

#[derive(Clone, Copy)]
enum OutputProjection {
    Text,
    All,
    First,
    Reading,
}

impl<'a> OutputToken<'a> {
    pub fn literal(text: &'a str) -> Self {
        Self { text, generated: false, wrap: false, projection: OutputProjection::Text }
    }

    pub fn generated(text: &'a str, wrap: bool) -> Self {
        Self { text, generated: true, wrap, projection: OutputProjection::Text }
    }

    /// Read the segment's owned meanings directly, without joining a temporary string.
    pub fn all_meanings(wrap: bool) -> Self {
        Self { text: "", generated: true, wrap, projection: OutputProjection::All }
    }

    pub fn first_meaning(wrap: bool) -> Self {
        Self { text: "", generated: true, wrap, projection: OutputProjection::First }
    }

    pub fn reading() -> Self {
        Self { text: "", generated: true, wrap: false, projection: OutputProjection::Reading }
    }

    fn characters<'b>(self, meanings: &'b [String], reading: Option<&'b str>) -> impl Iterator<Item = char> + Clone + 'b
    where 'a: 'b {
        let text = if matches!(self.projection, OutputProjection::Reading) { reading.unwrap_or("") } else { self.text };
        let literal = matches!(self.projection, OutputProjection::Text | OutputProjection::Reading);
        let count = match self.projection { OutputProjection::All => usize::MAX, OutputProjection::First => 1, _ => 0 };
        std::iter::once(text).filter(move |_| literal)
            .chain(meanings.iter().take(count).map(String::as_str))
            .enumerate().flat_map(|(index, text)| {
                std::iter::once('/').filter(move |_| index > 0).chain(text.chars())
            })
    }
}

pub struct SegmentInput<'a> {
    pub source_range: TextRange,
    pub surface: &'a str,
    pub readings: OutputToken<'a>,
    pub phrases: OutputToken<'a>,
    pub single_meaning: OutputToken<'a>,
    pub lemma: Option<String>,
    pub reading: Option<String>,
    pub part_of_speech: Option<String>,
    pub meanings: Vec<String>,
    pub provenance: Vec<DictionaryProvenance>,
    pub unknown: bool,
}

/// Appends display edits and UTF-16 range updates together. No final text-only
/// normalization pass can leave source mappings pointing into changed output.
pub struct OutputAssembler {
    document_id: String,
    source_revision: u64,
    dictionary_revision: u64,
    source_language: &'static str,
    readings: TextAssembler,
    phrases: TextAssembler,
    single_meaning: TextAssembler,
    segments: Vec<TranslationSegment>,
}

impl OutputAssembler {
    pub fn new(work: &TranslationWork, style: OutputStyle) -> Self {
        Self {
            document_id: work.document_id.clone(),
            source_revision: work.source.revision,
            dictionary_revision: work.dictionaries.revision(),
            source_language: match work.source.language {
                crate::models::SourceLanguage::Zh => "zh",
                crate::models::SourceLanguage::Ja => "ja",
            },
            readings: TextAssembler::new(style),
            phrases: TextAssembler::new(style),
            single_meaning: TextAssembler::new(style),
            segments: Vec::new(),
        }
    }

    pub fn push(&mut self, input: SegmentInput<'_>) -> AppResult<()> {
        if let Some(end) = self.readings.trim_before(input.readings, &input.meanings, input.reading.as_deref()) {
            clamp_ranges(&mut self.segments, end, |segment| &mut segment.readings_range);
        }
        if let Some(end) = self.phrases.trim_before(input.phrases, &input.meanings, input.reading.as_deref()) {
            clamp_ranges(&mut self.segments, end, |segment| &mut segment.phrases_range);
        }
        if let Some(end) = self.single_meaning.trim_before(input.single_meaning, &input.meanings, input.reading.as_deref()) {
            clamp_ranges(&mut self.segments, end, |segment| &mut segment.single_meaning_range);
        }
        let readings_range = self.readings.append(input.readings, &input.meanings, input.reading.as_deref())?;
        let phrases_range = self.phrases.append(input.phrases, &input.meanings, input.reading.as_deref())?;
        let single_meaning_range = self.single_meaning.append(input.single_meaning, &input.meanings, input.reading.as_deref())?;
        self.segments.push(TranslationSegment {
            id: format!("{}:{}-{}", self.source_language, input.source_range.start, input.source_range.end),
            source_range: input.source_range,
            readings_range,
            phrases_range,
            single_meaning_range,
            surface: input.surface.to_owned(),
            lemma: input.lemma,
            reading: input.reading,
            part_of_speech: input.part_of_speech,
            meanings: input.meanings,
            provenance: input.provenance,
            unknown: input.unknown,
        });
        Ok(())
    }

    pub fn finish(self) -> TranslationResult {
        TranslationResult {
            document_id: self.document_id,
            source_revision: self.source_revision,
            dictionary_revision: self.dictionary_revision,
            readings: self.readings.text,
            phrases: self.phrases.text,
            single_meaning: self.single_meaning.text,
            segments: self.segments,
        }
    }
}

fn clamp_ranges(segments: &mut [TranslationSegment], end: u32, select: fn(&mut TranslationSegment) -> &mut TextRange) {
    for segment in segments.iter_mut().rev() {
        let range = select(segment);
        if range.end <= end { break; }
        range.start = range.start.min(end);
        range.end = end;
    }
}

struct TextAssembler {
    style: OutputStyle,
    text: String,
    utf16: u32,
    last: Option<char>,
    last_generated: bool,
    sentence_start: bool,
}

impl TextAssembler {
    fn new(style: OutputStyle) -> Self {
        Self { style, text: String::new(), utf16: 0, last: None, last_generated: false, sentence_start: true }
    }

    fn trim_before(&mut self, token: OutputToken<'_>, meanings: &[String], reading: Option<&str>) -> Option<u32> {
        if self.style != OutputStyle::Chinese || token.wrap
            || !token.characters(meanings, reading).next().map(normalize_punctuation).is_some_and(is_closing_punctuation)
            || !self.last.is_some_and(is_horizontal_space) {
            return None;
        }
        while self.text.chars().next_back().is_some_and(is_horizontal_space) {
            let scalar = self.text.pop().unwrap();
            self.utf16 -= scalar.len_utf16() as u32;
        }
        self.last = self.text.chars().next_back();
        Some(self.utf16)
    }

    fn append(&mut self, token: OutputToken<'_>, meanings: &[String], reading: Option<&str>) -> AppResult<TextRange> {
        let mut scalars = token.characters(meanings, reading).peekable();
        let Some(&initial) = scalars.peek() else {
            return Ok(TextRange { start: self.utf16, end: self.utf16 });
        };
        if self.style == OutputStyle::Literal {
            // Keep Japanese source punctuation, case and spacing untouched. Only
            // generated gloss/reading tokens get separators and requested wraps.
            let first = if token.wrap { '[' } else { initial };
            if self.needs_separator(first, token.generated) {
                self.push_char(' ')?;
            }
            let start = self.utf16;
            if token.wrap { self.push_char('[')?; }
            for scalar in scalars { self.push_char(scalar)?; }
            if token.wrap { self.push_char(']')?; }
            self.last_generated = token.generated;
            return Ok(TextRange { start, end: self.utf16 });
        }

        let first = if token.wrap { '[' } else { normalize_punctuation(initial) };
        if self.needs_separator(first, token.generated) { self.push_char(' ')?; }
        let start = self.utf16;
        if token.wrap { self.push_char('[')?; }
        while let Some(original) = scalars.next() {
            let scalar = normalize_punctuation(original);
            // Values are trimmed at lookup. This additionally normalizes spaces
            // before punctuation inside a dictionary meaning, without editing it.
            if scalar.is_whitespace() && !matches!(scalar, '\n' | '\r') {
                let next = scalars.clone().find(|next| !is_horizontal_space(*next)).map(normalize_punctuation);
                let suppress = next.is_some_and(is_closing_punctuation);
                if !suppress { self.push_char(scalar)?; }
                while scalars.peek().is_some_and(|next| is_horizontal_space(*next)) {
                    let next = normalize_punctuation(scalars.next().unwrap());
                    if !suppress { self.push_char(next)?; }
                }
                continue;
            }
            if token.generated && self.sentence_start && scalar.is_alphabetic() {
                for uppercase in scalar.to_uppercase() { self.push_char(uppercase)?; }
                self.sentence_start = false;
            } else {
                self.push_char(scalar)?;
                if scalar.is_alphanumeric() { self.sentence_start = false; }
            }
            if matches!(scalar, '.' | '!' | '?' | '\n' | '\r') { self.sentence_start = true; }
        }
        if token.wrap { self.push_char(']')?; }
        self.last_generated = token.generated;
        Ok(TextRange { start, end: self.utf16 })
    }

    fn needs_separator(&self, first: char, generated: bool) -> bool {
        self.last.is_some_and(|last| {
            !last.is_whitespace() && !first.is_whitespace()
                && !is_opening_punctuation(last) && !is_closing_punctuation(first)
                && (generated || self.last_generated)
        })
    }

    fn push_char(&mut self, scalar: char) -> AppResult<()> {
        self.utf16 = self.utf16.checked_add(scalar.len_utf16() as u32).ok_or_else(|| {
            AppError::new("textTooLarge", "Generated text exceeds the supported UTF-16 offset range.")
        })?;
        self.text.push(scalar);
        self.last = Some(scalar);
        Ok(())
    }
}

pub(crate) fn is_horizontal_space(scalar: char) -> bool {
    scalar.is_whitespace() && !matches!(scalar, '\n' | '\r')
}

pub(crate) fn normalize_punctuation(scalar: char) -> char {
    match scalar {
        '，' | '、' => ',',
        '。' | '．' => '.',
        '！' => '!',
        '？' => '?',
        '：' => ':',
        '；' => ';',
        '（' => '(',
        '）' => ')',
        '［' => '[',
        '］' => ']',
        '｛' => '{',
        '｝' => '}',
        '「' | '『' | '《' => '“',
        '」' | '』' | '》' => '”',
        '　' => ' ',
        _ => scalar,
    }
}

pub(crate) fn is_closing_punctuation(scalar: char) -> bool {
    matches!(scalar, ',' | '.' | '!' | '?' | ':' | ';' | ')' | ']' | '}' | '”' | '’' | '…'
        | '，' | '、' | '。' | '！' | '？' | '：' | '；' | '）' | '］' | '｝' | '」' | '』' | '》')
}

fn is_opening_punctuation(scalar: char) -> bool {
    matches!(scalar, '(' | '[' | '{' | '“' | '‘' | '（' | '［' | '｛' | '「' | '『' | '《')
}
