use std::{collections::HashMap, sync::Arc};

use quickertranslator_lib::{
    engine::{alignment::OffsetMap, chinese::translate, OutputAssembler, OutputStyle, OutputToken, SegmentInput},
    models::{DictionaryKind as Kind, DictionaryLayer, DictionaryProvenance, FullWrap, SingleWrap, SourceLanguage, TextRange, TranslationAlgorithm as Algorithm, TranslationOptions, TranslationRequest, TranslationResult},
    state::{AppState, DictionaryEntry, DictionaryIndex, DictionarySnapshot, SourceSnapshot, TranslationWork},
};
use serde::Deserialize;

#[derive(Deserialize)]
struct FixtureEntry {
    kind: Kind,
    headword: String,
    meanings: Vec<String>,
    reading: Option<String>,
}

fn add(index: &mut DictionaryIndex, kind: Kind, key: &str, meanings: &[&str], reading: Option<&str>) {
    index.entry(SourceLanguage::Zh).or_default().entry(kind).or_default().insert(key.to_owned(), Arc::new(DictionaryEntry {
        headword: key.to_owned(),
        meanings: meanings.iter().map(|meaning| (*meaning).to_owned()).collect(),
        reading: reading.map(str::to_owned),
        part_of_speech: None,
        provenance: vec![DictionaryProvenance {
            dictionary_id: format!("fixture-{kind:?}"), dictionary_name: format!("Independent {kind:?} fixture"),
            kind, layer: DictionaryLayer::Imported, source_urls: Vec::new(),
        }],
    }));
}

fn fixture() -> DictionaryIndex {
    let records: Vec<FixtureEntry> = serde_json::from_str(include_str!("fixtures/chinese-engine.json")).unwrap();
    let mut index = HashMap::new();
    for record in records {
        let meanings: Vec<_> = record.meanings.iter().map(String::as_str).collect();
        add(&mut index, record.kind, &record.headword, &meanings, record.reading.as_deref());
    }
    index
}

fn work(index: DictionaryIndex, text: &str, options: TranslationOptions, rule_algorithm: u8) -> TranslationWork {
    let state = AppState::default();
    state.replace_dictionaries(DictionarySnapshot::new(3, index, rule_algorithm)).unwrap();
    let identity = state.register_window("main").unwrap();
    state.prepare_translation("main", TranslationRequest {
        document_id: identity.document_id, source_revision: 7, source_language: SourceLanguage::Zh,
        source_text: text.to_owned(), options,
    }).unwrap()
}

fn run(index: DictionaryIndex, text: &str, options: TranslationOptions, rule_algorithm: u8) -> TranslationResult {
    translate(&work(index, text, options, rule_algorithm)).unwrap()
}

fn output_span<'a>(output: &'a str, range: TextRange) -> &'a str {
    let map = OffsetMap::new(output).unwrap();
    &output[map.utf16_range_to_byte(range).unwrap()]
}

fn assert_mapping(result: &TranslationResult, source: &str) {
    let map = OffsetMap::new(source).unwrap();
    let mut cursor = 0;
    for segment in &result.segments {
        assert_eq!(segment.source_range.start, cursor);
        assert!(!segment.source_range.is_empty());
        let bytes = map.utf16_range_to_byte(segment.source_range).unwrap();
        assert_eq!(segment.surface, source[bytes]);
        cursor = segment.source_range.end;
        for (text, range) in [(&result.readings, segment.readings_range), (&result.phrases, segment.phrases_range), (&result.single_meaning, segment.single_meaning_range)] {
            OffsetMap::new(text).unwrap().validate_range(range).unwrap();
        }
    }
    assert_eq!(cursor, map.utf16_len());
    assert_eq!(result.segments.iter().map(|segment| segment.surface.as_str()).collect::<String>(), source);
}

#[test]
fn primary_name_alternatives_readings_and_emoji_have_exact_source_correspondence() {
    let source = "张三，你好。🙂";
    let result = run(fixture(), source, TranslationOptions::default(), 1);
    let name = &result.segments[0];
    assert_eq!(name.surface, "张三");
    assert_eq!(name.meanings, ["Trương Tam"]);
    assert_eq!(name.provenance[0].kind, Kind::PrimaryNames);
    assert_eq!(output_span(&result.phrases, name.phrases_range), "Trương Tam");
    assert_eq!(name.reading.as_deref(), Some("trương tam"));
    let greeting = result.segments.iter().find(|segment| segment.surface == "你好").unwrap();
    assert_eq!(greeting.meanings, ["xin chào", "chào bạn"]);
    assert_eq!(output_span(&result.phrases, greeting.phrases_range), "xin chào/chào bạn");
    assert_eq!(output_span(&result.single_meaning, greeting.single_meaning_range), "xin chào");
    assert_eq!(greeting.reading.as_deref(), Some("nhĩ hảo"));
    let emoji = result.segments.last().unwrap();
    assert_eq!(emoji.source_range, TextRange { start: 6, end: 8 });
    assert_eq!(output_span(&result.readings, emoji.readings_range), "🙂");
    assert_eq!(output_span(&result.phrases, emoji.phrases_range), "🙂");
    assert_eq!(output_span(&result.single_meaning, emoji.single_meaning_range), "🙂");
    assert!(result.segments.iter().all(|segment| segment.lemma.is_none() && segment.part_of_speech.is_none()));
    assert_mapping(&result, source);
}

#[test]
fn name_removal_exposes_secondary_then_phrase_without_touching_manual_target_revision() {
    let mut index = fixture();
    let state = AppState::default();
    state.replace_dictionaries(DictionarySnapshot::new(1, index.clone(), 1)).unwrap();
    let identity = state.register_window("main").unwrap();
    state.observe_target("main", &identity.document_id, 11).unwrap();
    index.get_mut(&SourceLanguage::Zh).unwrap().get_mut(&Kind::PrimaryNames).unwrap().remove("张三");
    state.replace_dictionaries(DictionarySnapshot::new(2, index.clone(), 1)).unwrap();
    let secondary = run(index.clone(), "张三", TranslationOptions::default(), 1);
    assert_eq!(secondary.segments[0].meanings, ["tên phụ"]);
    assert_eq!(secondary.segments[0].provenance[0].kind, Kind::SecondaryNames);
    index.get_mut(&SourceLanguage::Zh).unwrap().get_mut(&Kind::SecondaryNames).unwrap().remove("张三");
    state.replace_dictionaries(DictionarySnapshot::new(3, index.clone(), 1)).unwrap();
    let phrase = run(index, "张三", TranslationOptions::default(), 1);
    assert_eq!(phrase.segments[0].meanings, ["sai"]);
    assert_eq!(phrase.segments[0].provenance[0].kind, Kind::VietPhrase);
    assert_eq!(state.health("main").unwrap().target_revision, 11);
}

#[test]
fn han_fallback_uses_first_reading_and_unknown_han_remains_visible() {
    let mut index = fixture();
    index.get_mut(&SourceLanguage::Zh).unwrap().get_mut(&Kind::VietPhrase).unwrap().remove("你好");
    let source = "你好𠀀龘";
    let result = run(index, source, TranslationOptions::default(), 1);
    assert_eq!(result.phrases, "Nhĩ hảo ha 龘");
    assert_eq!(result.single_meaning, result.phrases);
    assert_eq!(result.segments[0].meanings, ["nhĩ", "nhị"]);
    assert!(!result.segments[0].unknown);
    assert_eq!(result.segments[2].source_range, TextRange { start: 2, end: 4 });
    assert!(result.segments[3].unknown);
    assert_eq!(result.segments[3].surface, "龘");
    assert_mapping(&result, source);
}

#[test]
fn unmatched_latin_whitespace_and_symbols_are_not_rewritten() {
    let source = "abc XYZ\t  123\n🙂✦";
    let result = run(HashMap::new(), source, TranslationOptions::default(), 1);
    assert_eq!(result.readings, source);
    assert_eq!(result.phrases, source);
    assert_eq!(result.single_meaning, source);
    assert_mapping(&result, source);
}

#[test]
fn overlapping_fixture_distinguishes_all_three_algorithms() {
    let mut index = HashMap::new();
    for (key, meaning) in [("甲乙", "pair one"), ("乙丙丁", "triple"), ("戊己", "pair two"), ("己庚辛壬", "four")] {
        add(&mut index, Kind::VietPhrase, key, &[meaning], None);
    }
    let source = "甲乙丙丁戊己庚辛壬癸";
    let surfaces = |algorithm| {
        let result = run(index.clone(), source, TranslationOptions { algorithm, ..Default::default() }, 1);
        assert_mapping(&result, source);
        result.segments.into_iter().map(|segment| segment.surface).collect::<Vec<_>>()
    };
    assert_eq!(surfaces(Algorithm::Longest), ["甲", "乙丙丁", "戊", "己庚辛壬", "癸"]);
    assert_eq!(surfaces(Algorithm::LongestConditional), ["甲乙", "丙", "丁", "戊", "己庚辛壬", "癸"]);
    assert_eq!(surfaces(Algorithm::LeftToRight), ["甲乙", "丙", "丁", "戊己", "庚", "辛", "壬", "癸"]);
}

#[test]
fn one_scalar_candidate_always_survives_longer_interior_lookahead() {
    let mut index = HashMap::new();
    add(&mut index, Kind::VietPhrase, "甲", &["one"], None);
    add(&mut index, Kind::PrimaryNames, "乙丙丁戊", &["long name"], None);
    for algorithm in [Algorithm::Longest, Algorithm::LongestConditional, Algorithm::LeftToRight] {
        let result = run(index.clone(), "甲乙丙丁戊", TranslationOptions { algorithm, ..Default::default() }, 1);
        assert_eq!(result.segments[0].meanings, ["one"]);
    }
}

#[test]
fn prioritize_names_checks_interior_starts_even_when_name_extends_past_candidate() {
    let mut index = HashMap::new();
    add(&mut index, Kind::VietPhrase, "甲乙丙", &["phrase"], None);
    add(&mut index, Kind::PrimaryNames, "乙丙丁", &["name"], None);
    let prioritized = run(index.clone(), "甲乙丙丁", TranslationOptions::default(), 1);
    assert_eq!(prioritized.segments[0].surface, "甲");
    assert_eq!(prioritized.segments[1].meanings, ["name"]);
    let unprioritized = run(index, "甲乙丙丁", TranslationOptions { prioritize_names: false, ..Default::default() }, 1);
    assert_eq!(unprioritized.segments[0].meanings, ["phrase"]);
}

#[test]
fn exact_name_exempts_embedded_name_rejection_but_not_algorithm_lookahead() {
    let mut index = HashMap::new();
    add(&mut index, Kind::PrimaryNames, "甲乙丙", &["outer name"], None);
    add(&mut index, Kind::SecondaryNames, "乙丙", &["inner name"], None);
    assert_eq!(run(index, "甲乙丙", TranslationOptions::default(), 1).segments[0].meanings, ["outer name"]);
    let mut index = HashMap::new();
    add(&mut index, Kind::PrimaryNames, "甲乙", &["short name"], None);
    add(&mut index, Kind::VietPhrase, "乙丙丁", &["long phrase"], None);
    let longest = run(index.clone(), "甲乙丙丁", TranslationOptions::default(), 1);
    assert_eq!(longest.segments[0].surface, "甲");
    let conditional = run(index, "甲乙丙丁", TranslationOptions { algorithm: Algorithm::LongestConditional, ..Default::default() }, 1);
    assert_eq!(conditional.segments[0].meanings, ["short name"]);
}

#[test]
fn one_character_embedded_name_does_not_preempt_a_phrase() {
    let mut index = HashMap::new();
    add(&mut index, Kind::VietPhrase, "甲乙", &["phrase"], None);
    add(&mut index, Kind::PrimaryNames, "乙", &["one-char name"], None);
    assert_eq!(run(index, "甲乙", TranslationOptions::default(), 1).segments[0].meanings, ["phrase"]);
}

#[test]
fn lookup_limit_counts_scalars_not_utf16_or_utf8_units() {
    let mut index = HashMap::new();
    let twenty = "𠀀".repeat(20);
    let twenty_one = "𠀀".repeat(21);
    add(&mut index, Kind::VietPhrase, &twenty, &["twenty"], None);
    add(&mut index, Kind::PrimaryNames, &twenty_one, &["too long"], None);
    let result = run(index, &twenty_one, TranslationOptions::default(), 1);
    assert_eq!(result.segments[0].meanings, ["twenty"]);
    assert_eq!(result.segments[0].source_range, TextRange { start: 0, end: 40 });
    assert_eq!(result.segments[1].source_range, TextRange { start: 40, end: 42 });
    assert_mapping(&result, &twenty_one);
}

#[test]
fn slash_and_pipe_alternatives_preserve_order_and_first_projection() {
    let mut index = HashMap::new();
    add(&mut index, Kind::VietPhrase, "你好", &["xin chào/chào bạn|kính chào", "lời chào"], None);
    let result = run(index, "你好", TranslationOptions::default(), 1);
    assert_eq!(result.segments[0].meanings, ["xin chào", "chào bạn", "kính chào", "lời chào"]);
    assert_eq!(result.single_meaning, "Xin chào");
    assert_eq!(result.phrases, "Xin chào/chào bạn/kính chào/lời chào");
}

#[test]
fn full_and_single_wrap_modes_do_not_wrap_unknown_source() {
    for (full_wrap, single_wrap) in [(FullWrap::None, SingleWrap::None), (FullWrap::All, SingleWrap::All), (FullWrap::Ambiguous, SingleWrap::None)] {
        let result = run(fixture(), "张三你好龘", TranslationOptions { full_wrap, single_wrap, ..Default::default() }, 1);
        let name = &result.segments[0];
        let phrase = &result.segments[1];
        assert_eq!(output_span(&result.phrases, name.phrases_range).starts_with('['), full_wrap == FullWrap::All);
        assert_eq!(output_span(&result.phrases, phrase.phrases_range).starts_with('['), full_wrap != FullWrap::None);
        assert_eq!(output_span(&result.single_meaning, phrase.single_meaning_range).starts_with('['), single_wrap == SingleWrap::All);
        assert_eq!(output_span(&result.phrases, result.segments[2].phrases_range), "龘");
        assert_mapping(&result, "张三你好龘");
    }
}

#[test]
fn ignored_longest_span_emits_nothing_but_keeps_source_mapping_and_provenance() {
    let mut index = fixture();
    add(&mut index, Kind::Ignored, "你好", &[], None);
    add(&mut index, Kind::Ignored, "你好🙂", &[], None);
    let source = "张三你好🙂你好";
    let result = run(index, source, TranslationOptions::default(), 1);
    assert_eq!(result.phrases, "Trương Tam");
    assert_eq!(result.segments[1].surface, "你好🙂");
    for segment in &result.segments[1..] {
        assert!(segment.readings_range.is_empty());
        assert!(segment.phrases_range.is_empty());
        assert!(segment.single_meaning_range.is_empty());
        assert_eq!(segment.provenance[0].kind, Kind::Ignored);
    }
    assert_mapping(&result, source);
}

#[test]
fn ignored_spans_are_not_artificially_limited_to_twenty_scalars() {
    let mut index = HashMap::new();
    let ignored = "甲".repeat(25);
    add(&mut index, Kind::Ignored, &ignored, &[], None);
    let result = run(index, &ignored, TranslationOptions::default(), 1);
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments[0].source_range, TextRange { start: 0, end: 25 });
    assert!(result.readings.is_empty() && result.phrases.is_empty() && result.single_meaning.is_empty());
}

#[test]
fn generated_chinese_punctuation_spacing_and_capitalization_leave_source_untouched() {
    let source = "你好  ，你好！\n你好（你好）。abc XYZ";
    let result = run(fixture(), source, TranslationOptions::default(), 1);
    assert!(result.phrases.starts_with("Xin chào/chào bạn, xin chào/chào bạn!\nXin chào"));
    assert!(!result.phrases.contains('，') && !result.phrases.contains('！') && !result.phrases.contains('（'));
    assert!(!result.phrases.contains(" ,"));
    assert!(result.phrases.ends_with(".abc XYZ"));
    for segment in result.segments.iter().filter(|segment| segment.surface == "  ") { assert!(segment.phrases_range.is_empty()); }
    assert_mapping(&result, source);
}

#[test]
fn generated_dictionary_value_spaces_before_closing_punctuation_are_removed_with_valid_ranges() {
    let mut index = HashMap::new();
    add(&mut index, Kind::VietPhrase, "你好", &["xin chào   ， bạn  ！"], None);
    let result = run(index, "你好", TranslationOptions::default(), 1);
    assert_eq!(result.phrases, "Xin chào, bạn!");
    assert_eq!(output_span(&result.phrases, result.segments[0].phrases_range), result.phrases);
}

#[test]
fn rules_use_named_capture_and_preserve_capture_then_template_alternative_order() {
    let mut index = fixture();
    add(&mut index, Kind::Pronouns, "你", &["anh/chị"], None);
    add(&mut index, Kind::Rules, "(我的){0}", &["của {0}|thuộc {0}"], None);
    let result = run(index, "我的你", TranslationOptions::default(), 1);
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments[0].meanings, ["của anh", "thuộc anh", "của chị", "thuộc chị"]);
    assert_eq!(result.single_meaning, "Của anh");
    assert_eq!(result.segments[0].provenance[0].kind, Kind::Rules);
    assert!(result.segments[0].provenance.iter().any(|source| source.kind == Kind::Pronouns));
    assert_mapping(&result, "我的你");
}

#[test]
fn rule_algorithm_capture_sources_and_duplicate_priority_are_explicit() {
    let mut index = HashMap::new();
    add(&mut index, Kind::Rules, "我的{0}", &["của {0}"], None);
    add(&mut index, Kind::Pronouns, "小明", &["pronoun"], None);
    add(&mut index, Kind::PrimaryNames, "小明", &["primary"], None);
    add(&mut index, Kind::SecondaryNames, "小明", &["secondary"], None);
    add(&mut index, Kind::VietPhrase, "小明", &["phrase"], None);
    add(&mut index, Kind::PrimaryNames, "小李", &["primary name"], None);
    add(&mut index, Kind::SecondaryNames, "小李", &["secondary name"], None);
    add(&mut index, Kind::VietPhrase, "小李", &["phrase name"], None);
    add(&mut index, Kind::VietPhrase, "电影", &["film"], None);
    let options = TranslationOptions::default();
    for algorithm in 1..=3 {
        let pronoun = run(index.clone(), "我的小明", options.clone(), algorithm);
        assert_eq!(pronoun.segments[0].meanings, ["của pronoun"]);
        let name = run(index.clone(), "我的小李", options.clone(), algorithm);
        let film = run(index.clone(), "我的电影", options.clone(), algorithm);
        if algorithm == 1 {
            assert!(!name.segments.iter().any(|segment| segment.provenance.iter().any(|source| source.kind == Kind::Rules)));
        } else {
            assert_eq!(name.segments[0].meanings, ["của primary name"]);
        }
        if algorithm < 3 {
            assert!(!film.segments.iter().any(|segment| segment.provenance.iter().any(|source| source.kind == Kind::Rules)));
        } else {
            assert_eq!(film.segments[0].meanings, ["của film"]);
        }
    }
}

#[test]
fn rules_order_by_scalar_pattern_length_then_lexical_and_exact_phrase_wins_same_span() {
    let mut index = HashMap::new();
    add(&mut index, Kind::Pronouns, "你", &["anh"], None);
    add(&mut index, Kind::Rules, "我{0}", &["short {0}"], None);
    add(&mut index, Kind::Rules, "我(?:){0}", &["lexically later {0}"], None);
    add(&mut index, Kind::Rules, "(?:我){0}", &["lexically first {0}"], None);
    let result = run(index.clone(), "我你", TranslationOptions::default(), 1);
    assert_eq!(result.segments[0].meanings, ["lexically first anh"]);
    add(&mut index, Kind::VietPhrase, "我你", &["exact"], None);
    assert_eq!(run(index, "我你", TranslationOptions::default(), 1).segments[0].meanings, ["exact"]);
}

#[test]
fn empty_unresolved_or_unanchored_rule_capture_is_not_applied() {
    let mut index = HashMap::new();
    add(&mut index, Kind::Pronouns, "你", &["anh"], None);
    add(&mut index, Kind::Rules, "我的{0}", &["của {0}"], None);
    for source in ["我的", "我的未知", "甲我的你乙"] {
        let result = run(index.clone(), source, TranslationOptions::default(), 1);
        assert_mapping(&result, source);
        if source != "甲我的你乙" {
            assert!(!result.segments.iter().any(|segment| segment.provenance.iter().any(|source| source.kind == Kind::Rules)));
        } else {
            let matched = result.segments.iter().find(|segment| segment.provenance.iter().any(|source| source.kind == Kind::Rules)).unwrap();
            assert_eq!(matched.surface, "我的你");
        }
    }
}

#[test]
fn invalid_regex_and_invalid_capture_mode_cannot_publish_a_new_dictionary_revision() {
    let state = AppState::default();
    state.replace_dictionaries(DictionarySnapshot::new(1, HashMap::new(), 1)).unwrap();
    let mut invalid = HashMap::new();
    add(&mut invalid, Kind::Rules, r"(?=我){0}", &["unsupported {0}"], None);
    assert_eq!(state.replace_dictionaries(DictionarySnapshot::new(2, invalid, 1)).unwrap_err().code, "invalidRule");
    assert_eq!(state.dictionaries().unwrap().revision(), 1);
    for algorithm in [0, 4] {
        assert_eq!(state.replace_dictionaries(DictionarySnapshot::new(2, HashMap::new(), algorithm)).unwrap_err().code, "invalidRuleAlgorithm");
        assert_eq!(state.dictionaries().unwrap().revision(), 1);
    }
}

#[test]
fn cancellation_returns_no_partial_success_and_empty_input_is_valid() {
    let work = work(fixture(), "你好", TranslationOptions::default(), 1);
    work.cancellation.cancel();
    assert_eq!(translate(&work).unwrap_err().code, "translationCancelled");
    let empty = run(fixture(), "", TranslationOptions::default(), 1);
    assert!(empty.readings.is_empty() && empty.phrases.is_empty() && empty.single_meaning.is_empty() && empty.segments.is_empty());
}

#[test]
fn segment_ids_survive_document_and_revision_identity_changes_for_same_source() {
    let first = run(fixture(), "张三你好🙂", TranslationOptions::default(), 1);
    let mut second_work = work(fixture(), "张三你好🙂", TranslationOptions::default(), 1);
    second_work.source = Arc::new(SourceSnapshot {
        revision: 200, language: SourceLanguage::Zh, text: second_work.source.text.clone(),
        offsets: OffsetMap::new(&second_work.source.text).unwrap(),
    });
    let second = translate(&second_work).unwrap();
    assert_ne!(first.document_id, second.document_id);
    assert_ne!(first.source_revision, second.source_revision);
    assert_eq!(first.segments.iter().map(|segment| &segment.id).collect::<Vec<_>>(), second.segments.iter().map(|segment| &segment.id).collect::<Vec<_>>());
}

#[test]
fn literal_assembler_path_never_applies_chinese_punctuation_or_capitalization() {
    let mut work = work(HashMap::new(), "学校。🙂", TranslationOptions::default(), 1);
    work.source = Arc::new(SourceSnapshot { revision: 7, language: SourceLanguage::Ja, text: "学校。🙂".to_owned(), offsets: OffsetMap::new("学校。🙂").unwrap() });
    assert_eq!(translate(&work).unwrap_err().code, "invalidSourceLanguage");
    let mut assembler = OutputAssembler::new(&work, OutputStyle::Literal);
    for (surface, source_range, text, generated) in [
        ("学校", TextRange { start: 0, end: 2 }, "trường học", true),
        ("。", TextRange { start: 2, end: 3 }, "。", false),
        ("🙂", TextRange { start: 3, end: 5 }, "🙂", false),
    ] {
        let output = if generated { OutputToken::generated(text, false) } else { OutputToken::literal(text) };
        assembler.push(SegmentInput {
            source_range, surface, readings: output,
            phrases: output, single_meaning: output,
            lemma: None, reading: None, part_of_speech: None, meanings: Vec::new(), provenance: Vec::new(), unknown: false,
        }).unwrap();
    }
    let result = assembler.finish();
    assert_eq!(result.phrases, "trường học。🙂");
    assert_mapping(&result, "学校。🙂");
}

#[test]
fn rules_can_replace_a_same_length_exact_phrase_rejected_for_an_embedded_name() {
    let mut index = HashMap::new();
    add(&mut index, Kind::Rules, "我的{0}", &["của {0}"], None);
    add(&mut index, Kind::PrimaryNames, "小明", &["Tiểu Minh"], None);
    add(&mut index, Kind::VietPhrase, "我的小明", &["rejected exact phrase"], None);
    let result = run(index, "我的小明", TranslationOptions::default(), 2);
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments[0].meanings, ["của Tiểu Minh"]);
    assert_eq!(result.segments[0].provenance[0].kind, Kind::Rules);
}

#[test]
fn rule_candidates_still_obey_longer_interior_phrase_lookahead() {
    let mut index = HashMap::new();
    add(&mut index, Kind::Pronouns, "你", &["anh"], None);
    add(&mut index, Kind::Rules, "我{0}", &["của {0}"], None);
    add(&mut index, Kind::VietPhrase, "你好吗", &["longer phrase"], None);
    for algorithm in [Algorithm::Longest, Algorithm::LongestConditional, Algorithm::LeftToRight] {
        let result = run(index.clone(), "我你好吗", TranslationOptions { algorithm, ..Default::default() }, 1);
        let applied = result.segments.iter().any(|segment| segment.provenance.iter().any(|source| source.kind == Kind::Rules));
        assert_eq!(applied, algorithm != Algorithm::Longest);
        assert_mapping(&result, "我你好吗");
    }
}

#[test]
fn whitespace_before_punctuation_across_ignored_spans_has_empty_valid_mappings() {
    let mut index = fixture();
    add(&mut index, Kind::Ignored, "忽略", &[], None);
    let source = "你好 \t忽略，你好";
    let result = run(index, source, TranslationOptions::default(), 1);
    assert!(!result.phrases.contains(" ,") && !result.phrases.contains("\t,"));
    let whitespace = result.segments.iter().find(|segment| segment.surface == " \t").unwrap();
    assert!(whitespace.readings_range.is_empty() && whitespace.phrases_range.is_empty() && whitespace.single_meaning_range.is_empty());
    assert_mapping(&result, source);
}

#[test]
fn embedded_name_can_begin_at_the_last_scalar_of_an_exact_candidate() {
    let mut index = HashMap::new();
    add(&mut index, Kind::VietPhrase, "甲乙丙", &["phrase"], None);
    add(&mut index, Kind::PrimaryNames, "丙丁", &["last-start name"], None);
    let result = run(index, "甲乙丙丁", TranslationOptions::default(), 1);
    assert_eq!(result.segments[0].surface, "甲");
    assert_eq!(result.segments.last().unwrap().meanings, ["last-start name"]);
}
