use std::{collections::HashMap, sync::Arc};

use quickertranslator_lib::{
    engine::{alignment::OffsetMap, japanese::translate},
    models::{DictionaryKind as Kind, DictionaryLayer, DictionaryProvenance, FullWrap, SingleWrap, SourceLanguage as Language, TextRange, TranslationOptions, TranslationRequest, TranslationResult},
    state::{AppState, DictionaryEntry, DictionaryIndex, DictionarySnapshot, TranslationWork},
};
use serde::Deserialize;

fn add(index: &mut DictionaryIndex, language: Language, kind: Kind, key: &str, meanings: &[&str], reading: Option<&str>) {
    index.entry(language).or_default().entry(kind).or_default().insert(key.to_owned(), Arc::new(DictionaryEntry {
        headword: key.to_owned(), meanings: meanings.iter().map(|value| (*value).to_owned()).collect(), reading: reading.map(str::to_owned), part_of_speech: None,
        provenance: vec![DictionaryProvenance { dictionary_id: format!("fixture-{kind:?}"), dictionary_name: "Authored fixture".to_owned(), kind, layer: DictionaryLayer::Imported, source_urls: Vec::new() }],
    }));
}

#[derive(Deserialize)]
struct Fixture { headword: String, meanings: Vec<String>, reading: Option<String> }
fn fixture() -> DictionaryIndex {
    let mut index = HashMap::new();
    let records: Vec<Fixture> = serde_json::from_str(include_str!("fixtures/japanese-engine.json")).unwrap();
    for record in records {
        let values: Vec<_> = record.meanings.iter().map(String::as_str).collect();
        add(&mut index, Language::Ja, Kind::Japanese, &record.headword, &values, record.reading.as_deref());
    }
    index
}

fn work(index: DictionaryIndex, source: &str, options: TranslationOptions) -> TranslationWork {
    let state = AppState::default();
    state.replace_dictionaries(DictionarySnapshot::new(2, index, 3)).unwrap();
    let identity = state.register_window("japanese-fixture").unwrap();
    state.prepare_translation("japanese-fixture", TranslationRequest { document_id: identity.document_id, source_revision: 1, source_language: Language::Ja, source_text: source.to_owned(), options }).unwrap()
}

fn span<'a>(text: &'a str, range: TextRange) -> &'a str {
    let offsets = OffsetMap::new(text).unwrap();
    &text[offsets.utf16_range_to_byte(range).unwrap()]
}

fn correspondence(result: &TranslationResult, source: &str) {
    let offsets = OffsetMap::new(source).unwrap();
    let mut end = 0;
    for segment in &result.segments {
        assert_eq!(segment.source_range.start, end);
        assert_eq!(span(source, segment.source_range), segment.surface);
        for (text, range) in [(&result.readings, segment.readings_range), (&result.phrases, segment.phrases_range), (&result.single_meaning, segment.single_meaning_range)] {
            OffsetMap::new(text).unwrap().validate_range(range).unwrap();
        }
        end = segment.source_range.end;
    }
    assert_eq!(end, offsets.utf16_len());
    assert_eq!(result.segments.iter().map(|segment| segment.surface.as_str()).collect::<String>(), source);
}

#[test]
fn sample_exposes_hiragana_glosses_inflected_lemma_and_emoji_offsets() {
    let source = "私は学校で日本語を勉強しています。🙂";
    let result = translate(&work(fixture(), source, TranslationOptions::default())).unwrap();
    let school = result.segments.iter().find(|segment| segment.surface == "学校").unwrap();
    assert_eq!(school.reading.as_deref(), Some("がっこう"));
    assert_eq!(school.meanings, ["trường học", "nhà trường"]);
    assert_eq!(span(&result.phrases, school.phrases_range), "trường học/nhà trường");
    assert_eq!(span(&result.single_meaning, school.single_meaning_range), "trường học");
    let japanese = result.segments.iter().find(|segment| segment.surface == "日本語").unwrap();
    assert_eq!(japanese.meanings, ["tiếng Nhật"]);
    let inflected = result.segments.iter().find(|segment| segment.surface == "し").unwrap();
    assert_eq!(inflected.lemma.as_deref(), Some("する"));
    assert_eq!(inflected.meanings, ["làm"]);
    let emoji = result.segments.iter().find(|segment| segment.surface == "🙂").unwrap();
    assert_eq!(emoji.source_range.end - emoji.source_range.start, 2);
    assert_eq!(span(&result.phrases, emoji.phrases_range), "🙂");
    assert!(!emoji.unknown);
    assert!(result.segments.iter().any(|segment| segment.surface == "私" && segment.unknown));
    correspondence(&result, source);
}

#[test]
fn longest_custom_phrase_and_primary_name_use_whole_token_boundaries() {
    let mut index = fixture();
    add(&mut index, Language::Ja, Kind::Japanese, "日本語を", &["ngắn"], None);
    add(&mut index, Language::Ja, Kind::Japanese, "日本語を勉強", &["học tiếng Nhật"], None);
    add(&mut index, Language::Ja, Kind::SecondaryNames, "日本語を勉強", &["tên phụ"], None);
    add(&mut index, Language::Ja, Kind::PrimaryNames, "日本語を勉強", &["tên chính"], Some("ニホンゴヲベンキョウ"));
    let source = "日本語を勉強しています。";
    let result = translate(&work(index, source, TranslationOptions::default())).unwrap();
    assert_eq!(result.segments[0].surface, "日本語を勉強");
    assert_eq!(result.segments[0].meanings, ["tên chính"]);
    assert_eq!(result.segments[0].reading.as_deref(), Some("にほんごをべんきょう"));
    assert_eq!(result.segments[0].provenance[0].kind, Kind::PrimaryNames);
    let mut index = HashMap::new();
    add(&mut index, Language::Ja, Kind::Japanese, "学", &["not a whole token"], None);
    let result = translate(&work(index, "学校。", TranslationOptions::default())).unwrap();
    assert_eq!(result.segments[0].surface, "学校");
    assert!(result.segments[0].unknown);
    assert_eq!(result.segments[0].meanings, Vec::<String>::new());
}

#[test]
fn chinese_readings_rules_ignored_phrases_and_punctuation_conversion_do_not_leak() {
    let mut index = HashMap::new();
    add(&mut index, Language::Zh, Kind::HanViet, "你", &["nhĩ"], Some("nhĩ"));
    add(&mut index, Language::Zh, Kind::Ignored, "你", &[], None);
    add(&mut index, Language::Zh, Kind::Pronouns, "你", &["bạn"], None);
    add(&mut index, Language::Zh, Kind::Rules, "{0}", &["sai {0}"], None);
    let source = "你。🙂";
    let result = translate(&work(index, source, TranslationOptions::default())).unwrap();
    assert_eq!(result.phrases, source);
    assert_eq!(result.single_meaning, source);
    let unknown = result.segments.iter().find(|segment| segment.surface == "你").unwrap();
    assert!(unknown.unknown);
    assert_eq!(unknown.reading, None);
    assert_eq!(unknown.lemma, None);
    correspondence(&result, source);
}

#[test]
fn gaps_literal_glosses_and_wraps_keep_paragraphs_and_scalar_boundaries() {
    let mut index = fixture();
    add(&mut index, Language::Ja, Kind::Japanese, "学校", &["trường / lớp", "nhà trường"], None);
    let options = TranslationOptions { full_wrap: FullWrap::Ambiguous, single_wrap: SingleWrap::All, ..TranslationOptions::default() };
    let source = "\n学校。\n\n日本語\t🙂\n";
    let result = translate(&work(index, source, options)).unwrap();
    let school = result.segments.iter().find(|segment| segment.surface == "学校").unwrap();
    assert_eq!(span(&result.phrases, school.phrases_range), "[trường / lớp/nhà trường]");
    assert_eq!(span(&result.single_meaning, school.single_meaning_range), "[trường / lớp]");
    assert!(result.phrases.starts_with('\n'));
    assert!(result.phrases.contains("。\n\n"));
    assert!(result.phrases.ends_with("\t🙂\n"));
    correspondence(&result, source);
}

#[test]
fn reading_lookup_and_inflected_base_lookup_remain_distinct_from_surface() {
    let mut index = HashMap::new();
    add(&mut index, Language::Ja, Kind::Japanese, "がっこう", &["tra theo cách đọc"], None);
    add(&mut index, Language::Ja, Kind::Japanese, "食べる", &["ăn"], None);
    let source = "学校で食べました。";
    let result = translate(&work(index, source, TranslationOptions::default())).unwrap();
    let school = result.segments.iter().find(|segment| segment.surface == "学校").unwrap();
    assert_eq!(school.meanings, ["tra theo cách đọc"]);
    let verb = result.segments.iter().find(|segment| segment.lemma.as_deref() == Some("食べる")).unwrap();
    assert_eq!(verb.surface, "食べ");
    assert_eq!(verb.meanings, ["ăn"]);
    correspondence(&result, source);
}

#[test]
fn cancellation_prevents_japanese_output_publication() {
    let work = work(fixture(), "学校。", TranslationOptions::default());
    work.cancellation.cancel();
    assert_eq!(translate(&work).unwrap_err().code, "translationCancelled");
}
