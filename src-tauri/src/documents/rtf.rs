//! A non-executing RTF reader. Only text and editor-supported formatting are
//! imported; the caller retains the original RTF for lossless legacy recovery.

use std::collections::{BTreeMap, BTreeSet};

use encoding_rs::Encoding;
use serde_json::{Map, Value, json};

use crate::models::{AppError, AppResult};

#[derive(Debug)]
pub struct ParsedRtf {
    pub target_document: Value,
    pub warnings: Vec<String>,
}

/// Parse one balanced RTF 1 root, decoding byte escapes without replacements.
/// Raw Unicode is accepted because legacy QuickTranslator files also contain
/// UTF-8 literal text. RTF byte escapes still use the declared RTF code page.
pub fn parse_rtf(input: &str) -> AppResult<ParsedRtf> {
    Parser::new(input).parse()
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("invalidRtf", message)
}

#[derive(Debug)]
enum Token<'a> {
    Open,
    Close,
    Text(&'a str),
    Word(&'a str, Option<i32>),
    Symbol(char),
    Hex(u8),
}

struct Lexer<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> Lexer<'a> {
    fn next(&mut self) -> AppResult<Option<Token<'a>>> {
        let bytes = self.input.as_bytes();
        // Unescaped source line endings are not RTF paragraph breaks.
        while matches!(bytes.get(self.position), Some(b'\r' | b'\n')) {
            self.position += 1;
        }
        let Some(&byte) = bytes.get(self.position) else {
            return Ok(None);
        };
        self.position += 1;
        match byte {
            b'{' => Ok(Some(Token::Open)),
            b'}' => Ok(Some(Token::Close)),
            b'\\' => {
                let Some(&next) = bytes.get(self.position) else {
                    return Err(invalid("RTF ends inside an escape."));
                };
                self.position += 1;
                if next == b'\'' {
                    let end = self.position.checked_add(2).ok_or_else(|| invalid("Invalid RTF hex escape."))?;
                    let pair = bytes.get(self.position..end).ok_or_else(|| invalid("Incomplete RTF hex escape."))?;
                    let high = hex_digit(pair[0]).ok_or_else(|| invalid("Invalid RTF hex escape."))?;
                    let low = hex_digit(pair[1]).ok_or_else(|| invalid("Invalid RTF hex escape."))?;
                    self.position = end;
                    return Ok(Some(Token::Hex(high * 16 + low)));
                }
                if next.is_ascii_alphabetic() {
                    let start = self.position - 1;
                    while bytes.get(self.position).is_some_and(u8::is_ascii_alphabetic) {
                        self.position += 1;
                    }
                    let name = &self.input[start..self.position];
                    let number_start = self.position;
                    if bytes.get(self.position) == Some(&b'-') {
                        self.position += 1;
                        if !bytes.get(self.position).is_some_and(u8::is_ascii_digit) {
                            return Err(invalid(format!("RTF control \\{name} has an invalid number.")));
                        }
                    }
                    while bytes.get(self.position).is_some_and(u8::is_ascii_digit) {
                        self.position += 1;
                    }
                    let parameter = if self.position != number_start {
                        Some(self.input[number_start..self.position].parse::<i32>()
                            .map_err(|_| invalid(format!("RTF control \\{name} has an out-of-range number.")))?)
                    } else {
                        None
                    };
                    // Exactly one ASCII space delimits a control word. Any
                    // further spaces are actual document text.
                    if bytes.get(self.position) == Some(&b' ') {
                        self.position += 1;
                    }
                    Ok(Some(Token::Word(name, parameter)))
                } else if next.is_ascii() {
                    if next == b'\r' && bytes.get(self.position) == Some(&b'\n') {
                        self.position += 1;
                    }
                    Ok(Some(Token::Symbol(char::from(next))))
                } else {
                    Err(invalid("An RTF control symbol must be ASCII."))
                }
            }
            _ => {
                let start = self.position - 1;
                while let Some(&next) = bytes.get(self.position) {
                    if matches!(next, b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                        break;
                    }
                    self.position += 1;
                }
                let text = self.input.get(start..self.position)
                    .ok_or_else(|| invalid("RTF binary data ends inside a Unicode character."))?;
                Ok(Some(Token::Text(text)))
            }
        }
    }

    fn binary(&mut self, count: usize) -> AppResult<()> {
        let end = self.position.checked_add(count).ok_or_else(|| invalid("Invalid RTF binary length."))?;
        if end > self.input.len() || !self.input.is_char_boundary(end) {
            return Err(invalid("RTF binary data is truncated or has an invalid byte boundary."));
        }
        self.position = end;
        Ok(())
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
    Body,
    FontTable,
    ColorTable,
    Field,
    UnicodeAlternative,
    Ignored,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl Alignment {
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "justify",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CharacterFormat {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    font: Option<i32>,
    half_points: u32,
    color: Option<usize>,
    background: Option<usize>,
}

impl Default for CharacterFormat {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            font: None,
            half_points: 24,
            color: None,
            background: None,
        }
    }
}

#[derive(Clone, Copy)]
struct State {
    destination: Destination,
    format: CharacterFormat,
    alignment: Alignment,
    encoding: &'static Encoding,
    unicode_fallback: u32,
    font_entry: Option<i32>,
    at_start: bool,
    optional_destination: bool,
    in_field: bool,
    alternative_children: u8,
    unicode_branch: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            destination: Destination::Body,
            format: CharacterFormat::default(),
            alignment: Alignment::Left,
            encoding: encoding_rs::WINDOWS_1252,
            unicode_fallback: 1,
            font_entry: None,
            at_start: true,
            optional_destination: false,
            in_field: false,
            alternative_children: 0,
            unicode_branch: false,
        }
    }
}

#[derive(Default)]
struct Font {
    name: String,
    charset: Option<u16>,
    code_page: Option<u16>,
    terminated: bool,
}

#[derive(Default)]
struct Color {
    red: Option<u8>,
    green: Option<u8>,
    blue: Option<u8>,
}

impl Color {
    fn finish(&mut self) -> Option<String> {
        let color = if self.red.is_none() && self.green.is_none() && self.blue.is_none() {
            None
        } else {
            Some(format!("#{:02x}{:02x}{:02x}", self.red.unwrap_or(0), self.green.unwrap_or(0), self.blue.unwrap_or(0)))
        };
        *self = Self::default();
        color
    }

    fn is_empty(&self) -> bool {
        self.red.is_none() && self.green.is_none() && self.blue.is_none()
    }
}

struct TextRun {
    text: String,
    format: CharacterFormat,
}

enum Inline {
    Text(TextRun),
    HardBreak,
}

struct Paragraph {
    alignment: Alignment,
    content: Vec<Inline>,
}

#[derive(Default)]
struct DocumentBuilder {
    paragraphs: Vec<Paragraph>,
    content: Vec<Inline>,
    alignment: Alignment,
}

impl DocumentBuilder {
    fn text(&mut self, text: &str, format: CharacterFormat, alignment: Alignment) {
        if text.is_empty() {
            return;
        }
        if self.content.is_empty() {
            self.alignment = alignment;
        }
        if let Some(Inline::Text(previous)) = self.content.last_mut() {
            if previous.format == format {
                previous.text.push_str(text);
                return;
            }
        }
        self.content.push(Inline::Text(TextRun { text: text.to_owned(), format }));
    }

    fn line(&mut self, alignment: Alignment) {
        if self.content.is_empty() {
            self.alignment = alignment;
        }
        self.content.push(Inline::HardBreak);
    }

    fn paragraph(&mut self, alignment: Alignment) {
        if self.content.is_empty() {
            self.alignment = alignment;
        }
        self.paragraphs.push(Paragraph {
            alignment: self.alignment,
            content: std::mem::take(&mut self.content),
        });
        self.alignment = alignment;
    }

    fn finish(&mut self, alignment: Alignment) {
        if !self.content.is_empty() || self.paragraphs.is_empty() {
            self.paragraph(alignment);
        }
    }
}

struct PendingSurrogate {
    high: u16,
    format: CharacterFormat,
    destination: Destination,
    font_entry: Option<i32>,
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    state: State,
    groups: Vec<State>,
    bytes: Vec<u8>,
    fallback_remaining: u32,
    surrogate: Option<PendingSurrogate>,
    default_font: Option<i32>,
    fonts: BTreeMap<i32, Font>,
    colors: Vec<Option<String>>,
    color: Color,
    builder: DocumentBuilder,
    warnings: BTreeSet<String>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            lexer: Lexer { input, position: 0 },
            state: State::default(),
            groups: Vec::new(),
            bytes: Vec::new(),
            fallback_remaining: 0,
            surrogate: None,
            default_font: None,
            fonts: BTreeMap::new(),
            colors: Vec::new(),
            color: Color::default(),
            builder: DocumentBuilder::default(),
            warnings: BTreeSet::new(),
        }
    }

    fn parse(mut self) -> AppResult<ParsedRtf> {
        // Outside the root only ASCII whitespace is accepted.
        while self.lexer.input.as_bytes().get(self.lexer.position).is_some_and(u8::is_ascii_whitespace) {
            self.lexer.position += 1;
        }
        if !matches!(self.lexer.next()?, Some(Token::Open))
            || !matches!(self.lexer.next()?, Some(Token::Word("rtf", Some(1))))
        {
            return Err(invalid("The document must begin with a single {\\rtf1 header."));
        }
        // The root's header has already been consumed, so it cannot declare a
        // destination or another RTF root afterward.
        self.state.at_start = false;
        let mut closed = false;
        while let Some(token) = self.lexer.next()? {
            match token {
                Token::Open => {
                    self.require_destination_word()?;
                    self.flush_bytes()?;
                    self.fallback_remaining = 0;
                    let mut child = self.state;
                    if self.state.destination == Destination::UnicodeAlternative {
                        self.state.alternative_children += 1;
                        child.destination = match self.state.alternative_children {
                            1 => Destination::Ignored,
                            2 => Destination::Body,
                            _ => return Err(invalid("RTF \\upr must contain exactly two alternative groups.")),
                        };
                        child.unicode_branch = self.state.alternative_children == 2;
                    } else {
                        child.unicode_branch = false;
                    }
                    self.groups.push(self.state);
                    child.at_start = true;
                    child.optional_destination = false;
                    child.alternative_children = 0;
                    self.state = child;
                }
                Token::Close => {
                    self.require_destination_word()?;
                    self.flush_bytes()?;
                    self.fallback_remaining = 0;
                    if self.state.destination == Destination::UnicodeAlternative && self.state.alternative_children != 2 {
                        return Err(invalid("RTF \\upr must contain exactly two alternative groups."));
                    }
                    if self.state.unicode_branch {
                        return Err(invalid("The Unicode alternative of RTF \\upr must start with \\*\\ud."));
                    }
                    if let Some(parent) = self.groups.pop() {
                        self.state = parent;
                    } else {
                        closed = true;
                        break;
                    }
                }
                Token::Text(text) => {
                    self.require_destination_word()?;
                    if self.state.unicode_branch {
                        return Err(invalid("The Unicode alternative of RTF \\upr must start with \\*\\ud."));
                    }
                    self.state.at_start = false;
                    for character in text.chars() {
                        if character.is_control() && character != '\t' {
                            return Err(invalid("RTF text contains an unescaped control character."));
                        }
                        if self.skip_fallback() {
                            continue;
                        }
                        if character.is_ascii() {
                            self.bytes.push(character as u8);
                        } else {
                            self.flush_bytes()?;
                            self.require_complete_unicode()?;
                            let mut buffer = [0; 4];
                            self.emit_text(character.encode_utf8(&mut buffer));
                        }
                    }
                }
                Token::Hex(byte) => {
                    self.require_destination_word()?;
                    if self.state.unicode_branch {
                        return Err(invalid("The Unicode alternative of RTF \\upr must start with \\*\\ud."));
                    }
                    self.state.at_start = false;
                    if !self.skip_fallback() {
                        self.bytes.push(byte);
                    }
                }
                Token::Word(name, parameter) => {
                    // Binary payload is opaque even in an ignored destination:
                    // braces and backslashes in it must not alter nesting.
                    if name == "bin" {
                        self.require_destination_word()?;
                        self.flush_bytes()?;
                        let count = nonnegative(name, parameter)? as usize;
                        self.lexer.binary(count)?;
                        if !self.skip_fallback() {
                            self.require_complete_unicode()?;
                            self.warn("RTF binary payloads were omitted; embedded content is not imported.");
                        }
                        self.state.at_start = false;
                    } else if !self.skip_fallback() {
                        self.flush_bytes()?;
                        self.word(name, parameter)?;
                    }
                }
                Token::Symbol(symbol) => {
                    if !self.skip_fallback() {
                        self.flush_bytes()?;
                        self.symbol(symbol)?;
                    }
                }
            }
        }
        if !closed {
            return Err(invalid("RTF groups are not balanced: the root is not closed."));
        }
        if !self.lexer.input[self.lexer.position..].bytes().all(|byte| byte.is_ascii_whitespace()) {
            return Err(invalid("RTF contains data outside its single root group."));
        }
        self.require_complete_unicode()?;
        if self.fonts.values().any(|font| !font.terminated) {
            return Err(invalid("An RTF font definition is missing its terminating semicolon."));
        }
        if !self.color.is_empty() {
            return Err(invalid("An RTF color definition is missing its terminating semicolon."));
        }
        self.builder.finish(self.state.alignment);
        let paragraphs = std::mem::take(&mut self.builder.paragraphs);
        let mut nodes = Vec::with_capacity(paragraphs.len());
        for paragraph in paragraphs {
            let mut content = Vec::with_capacity(paragraph.content.len());
            for inline in paragraph.content {
                match inline {
                    Inline::HardBreak => content.push(json!({ "type": "hardBreak" })),
                    Inline::Text(run) => {
                        let marks = self.marks(run.format);
                        let mut node = json!({ "type": "text", "text": run.text });
                        if !marks.is_empty() {
                            node["marks"] = Value::Array(marks);
                        }
                        content.push(node);
                    }
                }
            }
            nodes.push(json!({
                "type": "paragraph",
                "attrs": { "textAlign": paragraph.alignment.name() },
                "content": content,
            }));
        }
        Ok(ParsedRtf {
            target_document: json!({ "type": "doc", "content": nodes }),
            warnings: self.warnings.into_iter().collect(),
        })
    }

    fn warn(&mut self, warning: &str) {
        if !self.warnings.contains(warning) {
            self.warnings.insert(warning.to_owned());
        }
    }

    fn skip_fallback(&mut self) -> bool {
        if self.fallback_remaining == 0 {
            false
        } else {
            self.fallback_remaining -= 1;
            true
        }
    }

    fn require_destination_word(&self) -> AppResult<()> {
        if self.state.optional_destination {
            Err(invalid("RTF \\* must be followed by a destination control word."))
        } else {
            Ok(())
        }
    }

    fn require_complete_unicode(&self) -> AppResult<()> {
        if self.surrogate.is_some() {
            Err(invalid("RTF contains an unpaired UTF-16 high surrogate."))
        } else {
            Ok(())
        }
    }

    fn current_encoding(&self) -> AppResult<&'static Encoding> {
        let font_id = if self.state.destination == Destination::FontTable {
            self.state.font_entry
        } else {
            self.state.format.font.or(self.default_font)
        };
        let Some(font) = font_id.and_then(|id| self.fonts.get(&id)) else {
            return Ok(self.state.encoding);
        };
        if let Some(code_page) = font.code_page {
            return code_page_encoding(u32::from(code_page));
        }
        match font.charset {
            None | Some(0 | 1) => Ok(self.state.encoding),
            Some(charset) => match charset_encoding(charset) {
                // ASCII font-family metadata does not use the font's glyph
                // encoding. An unused Symbol/OEM font must not prevent an
                // otherwise strictly decodable document from being imported.
                Err(_) if self.state.destination == Destination::FontTable && self.bytes.is_ascii() => Ok(self.state.encoding),
                result => result,
            },
        }
    }

    fn flush_bytes(&mut self) -> AppResult<()> {
        if self.bytes.is_empty() {
            return Ok(());
        }
        self.require_complete_unicode()?;
        let encoding = self.current_encoding()?;
        let bytes = std::mem::take(&mut self.bytes);
        let decoded = encoding.decode_without_bom_handling_and_without_replacement(&bytes)
            .ok_or_else(|| AppError::new("invalidRtfEncoding", format!("RTF byte escapes are invalid in {}. No replacement characters were inserted.", encoding.name())))?;
        // Control bytes are not valid text even when hidden in a hex escape.
        if decoded.chars().any(|character| character.is_control() && character != '\t') {
            return Err(invalid("RTF byte escapes decode to an unescaped control character."));
        }
        self.emit_text(&decoded);
        self.bytes = bytes;
        self.bytes.clear();
        Ok(())
    }

    fn emit_text(&mut self, text: &str) {
        match self.state.destination {
            Destination::Body => self.builder.text(text, self.state.format, self.state.alignment),
            Destination::FontTable => {
                for character in text.chars() {
                    if character == ';' {
                        if let Some(id) = self.state.font_entry.take() {
                            if let Some(font) = self.fonts.get_mut(&id) {
                                font.name = font.name.trim().to_owned();
                                font.terminated = true;
                            }
                        }
                    } else if let Some(font) = self.state.font_entry.and_then(|id| self.fonts.get_mut(&id)) {
                        if !font.terminated {
                            font.name.push(character);
                        }
                    }
                }
            }
            Destination::ColorTable => {
                // Color table text is validated in flush_bytes/word rather
                // than accepted as body text. Semicolons finish entries.
                for character in text.chars() {
                    if character == ';' {
                        self.colors.push(self.color.finish());
                    } else if !character.is_whitespace() {
                        self.warn("Nonstandard text in the RTF color table was omitted.");
                    }
                }
            }
            Destination::Field | Destination::UnicodeAlternative | Destination::Ignored => {}
        }
    }

    fn symbol(&mut self, symbol: char) -> AppResult<()> {
        if symbol == '*' {
            if !self.state.at_start || self.state.optional_destination {
                return Err(invalid("RTF \\* is only valid before a group destination."));
            }
            self.state.optional_destination = true;
            return Ok(());
        }
        self.require_destination_word()?;
        if self.state.unicode_branch {
            return Err(invalid("The Unicode alternative of RTF \\upr must start with \\*\\ud."));
        }
        self.state.at_start = false;
        self.require_complete_unicode()?;
        match symbol {
            '\\' => self.emit_text("\\"),
            '{' => self.emit_text("{"),
            '}' => self.emit_text("}"),
            '~' => self.emit_text("\u{00a0}"),
            '_' => self.emit_text("\u{2011}"),
            '-' => self.emit_text("\u{00ad}"),
            ' ' => self.emit_text(" "),
            '\r' | '\n' => self.paragraph(),
            _ => self.warn(&format!("Unsupported RTF control symbol \\{symbol} was omitted.")),
        }
        Ok(())
    }

    fn destination(&mut self, name: &str) -> AppResult<bool> {
        let destination = match name {
            "fonttbl" => Some(Destination::FontTable),
            "colortbl" => Some(Destination::ColorTable),
            "field" => Some(Destination::Field),
            "fldinst" => Some(Destination::Ignored),
            "fldrslt" => Some(Destination::Body),
            "upr" => Some(Destination::UnicodeAlternative),
            "ud" => Some(Destination::Body),
            "listtext" | "pntext" => Some(Destination::Body),
            name if is_nontext_destination(name) => Some(Destination::Ignored),
            _ => None,
        };
        let Some(destination) = destination else {
            if self.state.optional_destination {
                self.state.destination = Destination::Ignored;
                self.warn(&format!("Unsupported RTF destination \\{name} was omitted."));
                return Ok(true);
            }
            return Ok(false);
        };
        if !self.state.at_start {
            return Err(invalid(format!("RTF destination \\{name} must start a group.")));
        }
        if self.state.unicode_branch && (name != "ud" || !self.state.optional_destination) {
            return Err(invalid("The Unicode alternative of RTF \\upr must start with \\*\\ud."));
        }
        // No destination inside an omitted object, picture, instruction, or
        // unknown optional destination can reactivate its contents.
        if self.state.destination == Destination::Ignored {
            return Ok(true);
        }
        match name {
            "fldrslt" if !self.state.in_field => {
                return Err(invalid("RTF field results must occur inside a field group."));
            }
            "field" => {
                self.state.in_field = true;
                self.warn("RTF field instructions were omitted; only displayed field results were imported.");
            }
            "fldinst" => self.warn("RTF field instructions were omitted; only displayed field results were imported."),
            "ud" => {
                if !self.state.unicode_branch {
                    return Err(invalid("RTF \\ud must occur in the Unicode alternative of \\upr."));
                }
                self.state.unicode_branch = false;
            }
            "listtext" | "pntext" => self.warn("RTF list labels were imported as plain text; list structure was not imported."),
            _ if destination == Destination::Ignored => {
                self.warn(&format!("RTF destination \\{name} was omitted; non-text content is not imported."));
            }
            _ => {}
        }
        self.state.destination = destination;
        Ok(true)
    }

    fn word(&mut self, name: &str, parameter: Option<i32>) -> AppResult<()> {
        if name == "rtf" {
            return Err(invalid("An RTF header is only valid at the start of the root."));
        }
        if self.destination(name)? {
            self.state.optional_destination = false;
            self.state.at_start = false;
            return Ok(());
        }
        if self.state.unicode_branch {
            return Err(invalid("The Unicode alternative of RTF \\upr must start with \\*\\ud."));
        }
        self.state.optional_destination = false;
        self.state.at_start = false;
        match name {
            "ansi" => self.state.encoding = encoding_rs::WINDOWS_1252,
            "mac" => self.state.encoding = encoding_rs::MACINTOSH,
            "pc" | "pca" => return Err(AppError::new("invalidRtfEncoding", "OEM RTF code pages are not supported; save this document using ANSI or Unicode RTF.")),
            "ansicpg" => self.state.encoding = code_page_encoding(nonnegative(name, parameter)?)?,
            "deff" => {
                let id = nonnegative(name, parameter)? as i32;
                self.default_font = Some(id);
                self.state.format.font = Some(id);
            }
            "uc" => self.state.unicode_fallback = nonnegative(name, parameter)?,
            "u" => {
                let number = required(name, parameter)?;
                let unit = i16::try_from(number).map_err(|_| invalid("RTF \\u requires a signed 16-bit value."))? as u16;
                self.unicode(unit)?;
                self.fallback_remaining = self.state.unicode_fallback;
            }
            "f" if self.state.destination == Destination::FontTable => {
                let id = nonnegative(name, parameter)? as i32;
                if self.state.font_entry.is_some_and(|previous| self.fonts.get(&previous).is_some_and(|font| !font.terminated)) {
                    return Err(invalid("An RTF font definition is missing its terminating semicolon."));
                }
                if self.fonts.insert(id, Font::default()).is_some() {
                    return Err(invalid("RTF contains a duplicate font definition."));
                }
                self.state.font_entry = Some(id);
            }
            "fcharset" if self.state.destination == Destination::FontTable => {
                let charset = u16::try_from(nonnegative(name, parameter)?).map_err(|_| invalid("RTF font charset is out of range."))?;
                let id = self.state.font_entry.ok_or_else(|| invalid("RTF font charset has no font definition."))?;
                self.fonts.get_mut(&id).ok_or_else(|| invalid("RTF font definition is missing."))?.charset = Some(charset);
                if charset != 0 && charset != 1 && charset_encoding(charset).is_err() {
                    self.warn(&format!("RTF font charset {charset} has no supported byte decoder; Unicode text is retained, but its glyph mapping may differ."));
                }
            }
            "cpg" if self.state.destination == Destination::FontTable => {
                let code_page = u16::try_from(nonnegative(name, parameter)?).map_err(|_| invalid("RTF font code page is out of range."))?;
                code_page_encoding(u32::from(code_page))?;
                let id = self.state.font_entry.ok_or_else(|| invalid("RTF font code page has no font definition."))?;
                self.fonts.get_mut(&id).ok_or_else(|| invalid("RTF font definition is missing."))?.code_page = Some(code_page);
            }
            "fnil" | "froman" | "fswiss" | "fmodern" | "fscript" | "fdecor" | "ftech" | "fbidi"
                if self.state.destination == Destination::FontTable => {}
            "fprq" if self.state.destination == Destination::FontTable => { nonnegative(name, parameter)?; }
            "red" | "green" | "blue" if self.state.destination == Destination::ColorTable => {
                let component = u8::try_from(nonnegative(name, parameter)?).map_err(|_| invalid("RTF color components must be between 0 and 255."))?;
                match name {
                    "red" => self.color.red = Some(component),
                    "green" => self.color.green = Some(component),
                    _ => self.color.blue = Some(component),
                }
            }
            "f" => self.state.format.font = Some(nonnegative(name, parameter)? as i32),
            "fs" => {
                let size = nonnegative(name, parameter)?;
                if size == 0 || size > 20_000 {
                    return Err(invalid("RTF font size must be positive and no greater than the editor's 10000pt limit."));
                }
                self.state.format.half_points = size;
            }
            "cf" => self.state.format.color = color_index(name, parameter)?,
            "highlight" | "cb" => self.state.format.background = color_index(name, parameter)?,
            "b" => self.state.format.bold = toggle(name, parameter)?,
            "i" => self.state.format.italic = toggle(name, parameter)?,
            "ul" => self.state.format.underline = toggle(name, parameter)?,
            "ulnone" => self.state.format.underline = false,
            "strike" => self.state.format.strike = toggle(name, parameter)?,
            "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulhwave" | "ulldash" | "ulth" | "ulthd" | "ulthdash" | "ulthdashd" | "ulthdashdd" | "ulthldash" | "ululdbwave" | "ulw" | "ulwave" => {
                self.state.format.underline = toggle(name, parameter)?;
                self.warn("RTF patterned or double underlines were converted to a single underline.");
            }
            "striked" => {
                self.state.format.strike = toggle(name, parameter)?;
                self.warn("RTF double strikethrough was converted to a single strikethrough.");
            }
            "plain" => self.state.format = CharacterFormat { font: self.default_font, ..CharacterFormat::default() },
            "pard" => self.align(Alignment::Left),
            "ql" => self.align(Alignment::Left),
            "qc" => self.align(Alignment::Center),
            "qr" => self.align(Alignment::Right),
            "qj" => self.align(Alignment::Justify),
            "par" => { self.require_complete_unicode()?; self.paragraph(); }
            "line" => {
                self.require_complete_unicode()?;
                if self.state.destination == Destination::Body {
                    self.builder.line(self.state.alignment);
                }
            }
            "tab" => self.character("\t")?,
            "emdash" => self.character("\u{2014}")?,
            "endash" => self.character("\u{2013}")?,
            "emspace" => self.character("\u{2003}")?,
            "enspace" => self.character("\u{2002}")?,
            "qmspace" => self.character("\u{2005}")?,
            "bullet" => self.character("\u{2022}")?,
            "lquote" => self.character("\u{2018}")?,
            "rquote" => self.character("\u{2019}")?,
            "ldblquote" => self.character("\u{201c}")?,
            "rdblquote" => self.character("\u{201d}")?,
            "zwj" => self.character("\u{200d}")?,
            "zwnj" => self.character("\u{200c}")?,
            "page" | "column" | "sect" => {
                self.require_complete_unicode()?;
                self.warn("RTF page, column, or section breaks were converted to paragraph breaks; page layout was not imported.");
                self.paragraph();
            }
            "cell" | "nestcell" => {
                self.warn("RTF tables were converted to paragraphs and tabs; table layout was not imported.");
                self.character("\t")?;
            }
            "row" | "nestrow" => {
                self.require_complete_unicode()?;
                self.warn("RTF tables were converted to paragraphs and tabs; table layout was not imported.");
                self.paragraph();
            }
            "v" => {
                toggle(name, parameter)?;
                self.warn("RTF hidden-text formatting is unsupported; hidden text was imported visibly.");
            }
            // These describe a reader's UI, not document text or formatting.
            "viewkind" | "viewscale" | "viewzk" | "fet" => { nonnegative(name, parameter)?; }
            _ => {
                if self.state.destination != Destination::Ignored {
                    self.warn(&format!("Unsupported RTF control \\{name} was omitted; its formatting was not imported."));
                }
            }
        }
        Ok(())
    }

    fn character(&mut self, text: &str) -> AppResult<()> {
        self.require_complete_unicode()?;
        self.emit_text(text);
        Ok(())
    }

    fn paragraph(&mut self) {
        if self.state.destination == Destination::Body {
            self.builder.paragraph(self.state.alignment);
        }
    }

    fn align(&mut self, alignment: Alignment) {
        self.state.alignment = alignment;
        if self.state.destination == Destination::Body {
            self.builder.alignment = alignment;
        }
    }

    fn unicode(&mut self, unit: u16) -> AppResult<()> {
        if (0xd800..=0xdbff).contains(&unit) {
            self.require_complete_unicode()?;
            self.surrogate = Some(PendingSurrogate {
                high: unit,
                format: self.state.format,
                destination: self.state.destination,
                font_entry: self.state.font_entry,
            });
            return Ok(());
        }
        let scalar = if (0xdc00..=0xdfff).contains(&unit) {
            let high = self.surrogate.take().ok_or_else(|| invalid("RTF contains an unpaired UTF-16 low surrogate."))?;
            if high.format != self.state.format || high.destination != self.state.destination || high.font_entry != self.state.font_entry {
                return Err(invalid("RTF changes text formatting or destination inside a UTF-16 surrogate pair."));
            }
            0x10000 + ((u32::from(high.high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00)
        } else {
            self.require_complete_unicode()?;
            u32::from(unit)
        };
        let character = char::from_u32(scalar).ok_or_else(|| invalid("Invalid RTF Unicode scalar."))?;
        if character.is_control() && character != '\t' {
            return Err(invalid("RTF Unicode escape encodes an unsupported control character."));
        }
        let mut buffer = [0; 4];
        self.emit_text(character.encode_utf8(&mut buffer));
        Ok(())
    }

    fn marks(&mut self, format: CharacterFormat) -> Vec<Value> {
        let mut marks = Vec::new();
        for (enabled, name) in [(format.bold, "bold"), (format.italic, "italic"), (format.underline, "underline"), (format.strike, "strike")] {
            if enabled {
                marks.push(json!({ "type": name }));
            }
        }
        let mut attrs = Map::new();
        if let Some(id) = format.font.or(self.default_font) {
            if let Some(font) = self.fonts.get(&id).filter(|font| !font.name.is_empty()) {
                attrs.insert("fontFamily".to_owned(), Value::String(font.name.clone()));
            } else {
                self.warn(&format!("RTF font {id} is undefined or unnamed; its family was not imported."));
            }
        }
        let size = if format.half_points % 2 == 0 {
            format!("{}pt", format.half_points / 2)
        } else {
            format!("{}.5pt", format.half_points / 2)
        };
        attrs.insert("fontSize".to_owned(), Value::String(size));
        for (index, name) in [(format.color, "color"), (format.background, "backgroundColor")] {
            if let Some(index) = index {
                if let Some(color) = self.colors.get(index) {
                    if let Some(color) = color {
                        attrs.insert(name.to_owned(), Value::String(color.clone()));
                    }
                } else {
                    self.warn(&format!("RTF color {index} is undefined; that color was not imported."));
                }
            }
        }
        marks.push(json!({ "type": "textStyle", "attrs": attrs }));
        marks
    }
}

fn required(name: &str, parameter: Option<i32>) -> AppResult<i32> {
    parameter.ok_or_else(|| invalid(format!("RTF control \\{name} requires a numeric argument.")))
}

fn nonnegative(name: &str, parameter: Option<i32>) -> AppResult<u32> {
    u32::try_from(required(name, parameter)?)
        .map_err(|_| invalid(format!("RTF control \\{name} requires a nonnegative number.")))
}

fn toggle(name: &str, parameter: Option<i32>) -> AppResult<bool> {
    match parameter {
        None | Some(1) => Ok(true),
        Some(0) => Ok(false),
        _ => Err(invalid(format!("RTF toggle \\{name} requires 0 or 1."))),
    }
}

fn color_index(name: &str, parameter: Option<i32>) -> AppResult<Option<usize>> {
    let index = nonnegative(name, parameter)? as usize;
    Ok((index != 0).then_some(index))
}

fn code_page_encoding(code_page: u32) -> AppResult<&'static Encoding> {
    let label = match code_page {
        65001 => "utf-8",
        874 => "windows-874",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        10000 => "macintosh",
        10007 => "x-mac-cyrillic",
        1250 => "windows-1250",
        1251 => "windows-1251",
        1252 => "windows-1252",
        1253 => "windows-1253",
        1254 => "windows-1254",
        1255 => "windows-1255",
        1256 => "windows-1256",
        1257 => "windows-1257",
        1258 => "windows-1258",
        28592 => "iso-8859-2",
        28593 => "iso-8859-3",
        28594 => "iso-8859-4",
        28595 => "iso-8859-5",
        28596 => "iso-8859-6",
        28597 => "iso-8859-7",
        28598 => "iso-8859-8",
        28603 => "iso-8859-13",
        28605 => "iso-8859-15",
        _ => return Err(AppError::new("invalidRtfEncoding", format!("RTF code page {code_page} is unsupported; no replacement decoding was attempted."))),
    };
    Encoding::for_label(label.as_bytes())
        .ok_or_else(|| AppError::new("invalidRtfEncoding", format!("RTF code page {code_page} has no strict decoder.")))
}

fn charset_encoding(charset: u16) -> AppResult<&'static Encoding> {
    let code_page = match charset {
        77 => 10000,
        128 => 932,
        129 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        _ => return Err(AppError::new("invalidRtfEncoding", format!("RTF font charset {charset} has no supported strict byte decoder."))),
    };
    code_page_encoding(code_page)
}

fn is_nontext_destination(name: &str) -> bool {
    matches!(name,
        "pict" | "object" | "objdata" | "objclass" | "objname" | "objalias" | "result"
        | "nonshppict" | "shppict" | "shp" | "shpinst" | "shprslt" | "shptxt"
        | "info" | "title" | "subject" | "author" | "manager" | "company" | "operator"
        | "category" | "keywords" | "comment" | "doccomm" | "hlinkbase" | "generator"
        | "creatim" | "revtim" | "printim" | "buptim" | "stylesheet" | "filetbl"
        | "listtable" | "listoverridetable" | "list" | "listlevel" | "leveltext" | "levelnumbers"
        | "pn" | "revtbl" | "rsidtbl" | "fontemb" | "fontfile" | "falt" | "panose"
        | "header" | "headerl" | "headerr" | "headerf" | "footer" | "footerl" | "footerr" | "footerf"
        | "footnote" | "ftnsep" | "ftnsepc" | "aftnsep" | "aftnsepc" | "annotation"
        | "atnid" | "atnauthor" | "atntime" | "atrfstart" | "atrfend"
        | "bkmkstart" | "bkmkend" | "docvar" | "userprops" | "propname" | "staticval"
        | "background" | "datastore" | "datafield" | "private" | "xmlopen" | "xmlclose"
        | "xmlattrname" | "xmlattrvalue" | "xmlnstbl" | "themedata" | "colorschememapping"
        | "mmathPr" | "mhtmltag" | "htmltag"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph_text(paragraph: &Value) -> String {
        paragraph["content"].as_array().unwrap().iter().map(|node| {
            if node["type"] == "hardBreak" { "\n" } else { node["text"].as_str().unwrap() }
        }).collect()
    }

    fn text(document: &Value) -> String {
        document["content"].as_array().unwrap().iter().map(paragraph_text).collect::<Vec<_>>().join("\n")
    }

    fn has_mark(node: &Value, name: &str) -> bool {
        node["marks"].as_array().unwrap().iter().any(|mark| mark["type"] == name)
    }

    fn style(node: &Value) -> &Value {
        &node["marks"].as_array().unwrap().iter().find(|mark| mark["type"] == "textStyle").unwrap()["attrs"]
    }

    #[test]
    fn unicode_vietnamese_bold_braces_paragraphs_and_emoji() {
        let parsed = parse_rtf(r"{\rtf1\ansi\uc1 Xin {\b ch\u224?o} \{b\}\par Ti\u7871?ng Vi\u7879?t \u-10179?\u-8704?}").unwrap();
        assert_eq!(text(&parsed.target_document), "Xin chào {b}\nTiếng Việt 😀");
        let paragraphs = parsed.target_document["content"].as_array().unwrap();
        assert!(has_mark(&paragraphs[0]["content"][1], "bold"));
        assert!(!has_mark(&paragraphs[0]["content"][2], "bold"));
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn fallback_counts_and_formatting_restore_at_group_boundaries() {
        let parsed = parse_rtf(r"{\rtf1\uc1 \u224?{\uc2\i\u7879\'65\'3f}\u233?\tab x\line y}").unwrap();
        assert_eq!(text(&parsed.target_document), "àệé\tx\ny");
        let nodes = parsed.target_document["content"][0]["content"].as_array().unwrap();
        assert!(has_mark(&nodes[1], "italic"));
        assert!(!has_mark(&nodes[2], "italic"));
    }

    #[test]
    fn fallback_control_tokens_count_once_and_braces_end_the_skip() {
        let parsed = parse_rtf(r"{\rtf1\uc2\u233\tab\'3f{\uc0\u224}\u7879?{x}}").unwrap();
        assert_eq!(text(&parsed.target_document), "éàệx");
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn ansi_and_font_charsets_decode_hex_runs_strictly() {
        let parsed = parse_rtf(r"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0\fnil\fcharset0 Arial;}{\f1\fnil\fcharset204 Cyrillic;}{\f2\fnil\fcharset128 Japanese;}}\f0 caf\'e9 \f1\'cf\'f0\'e8\'e2\'e5\'f2 \f2\'82\'a0}").unwrap();
        assert_eq!(text(&parsed.target_document), "café Привет あ");
        let nodes = parsed.target_document["content"][0]["content"].as_array().unwrap();
        assert_eq!(style(&nodes[0])["fontFamily"], "Arial");
        assert_eq!(style(&nodes[1])["fontFamily"], "Cyrillic");
        assert_eq!(style(&nodes[2])["fontFamily"], "Japanese");
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn vietnamese_code_page_and_font_code_page_override() {
        let parsed = parse_rtf(r"{\rtf1\ansicpg1258 a\'ec{\fonttbl{\f0\fcharset0\cpg1251 Russian;}}\f0\'df}").unwrap();
        assert_eq!(text(&parsed.target_document), "a\u{0301}Я");
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn unused_unsupported_font_charset_does_not_decode_body_as_that_font() {
        let parsed = parse_rtf(r"{\rtf1{\fonttbl{\f0\fcharset255 OEM;}{\f1\fcharset0 Arial;}}\f1 Text}").unwrap();
        assert_eq!(text(&parsed.target_document), "Text");
        assert!(parsed.warnings.iter().any(|warning| warning.contains("charset 255")));
    }

    #[test]
    fn font_sizes_colors_alignment_and_scoped_marks() {
        let parsed = parse_rtf(r"{\rtf1\deff0{\fonttbl{\f0\fswiss Arial;}}{\colortbl;\red255\green0\blue0;\red0\green128\blue255;}\qc\fs25\cf1\highlight2\ul\i A{\b\strike B}\ulnone\i0\cf0\highlight0 C\par\pard\plain D}").unwrap();
        let paragraphs = parsed.target_document["content"].as_array().unwrap();
        assert_eq!(paragraphs[0]["attrs"]["textAlign"], "center");
        assert_eq!(paragraphs[1]["attrs"]["textAlign"], "left");
        let first = &paragraphs[0]["content"][0];
        assert_eq!(style(first)["fontSize"], "12.5pt");
        assert_eq!(style(first)["color"], "#ff0000");
        assert_eq!(style(first)["backgroundColor"], "#0080ff");
        assert!(has_mark(first, "underline"));
        assert!(has_mark(first, "italic"));
        let second = &paragraphs[0]["content"][1];
        assert!(has_mark(second, "bold"));
        assert!(has_mark(second, "strike"));
        assert!(!has_mark(&paragraphs[0]["content"][2], "bold"));
        assert_eq!(style(&paragraphs[1]["content"][0])["fontSize"], "12pt");
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn pictures_objects_unknown_destinations_and_binary_are_not_text() {
        let parsed = parse_rtf(r"{\rtf1 before{\pict\pngblip 012345}{\object{\objdata 123}{\result secret}}{\*\unknown secret}{\pict\bin4 {}\\}after}").unwrap();
        assert_eq!(text(&parsed.target_document), "beforeafter");
        assert!(parsed.warnings.iter().any(|warning| warning.contains("\\pict")));
        assert!(parsed.warnings.iter().any(|warning| warning.contains("\\object")));
        assert!(parsed.warnings.iter().any(|warning| warning.contains("\\unknown")));
        assert!(parsed.warnings.iter().any(|warning| warning.contains("binary")));
    }

    #[test]
    fn field_display_only_has_no_link_or_executable_instruction() {
        let parsed = parse_rtf(r#"{\rtf1 A {\field{\*\fldinst HYPERLINK "https://example.invalid"}{\fldrslt{\b displayed}}} Z}"#).unwrap();
        assert_eq!(text(&parsed.target_document), "A displayed Z");
        assert!(has_mark(&parsed.target_document["content"][0]["content"][1], "bold"));
        assert!(!parsed.target_document.to_string().contains("https://"));
        assert!(!parsed.target_document.to_string().contains("\"type\":\"link\""));
        assert_eq!(parsed.warnings.len(), 1);
    }

    #[test]
    fn unicode_alternative_does_not_duplicate_ansi_fallback() {
        let parsed = parse_rtf(r"{\rtf1 {\upr{fallback}{\*\ud\uc0 Ti\u7871 ng}}}").unwrap();
        assert_eq!(text(&parsed.target_document), "Tiếng");
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn escaped_slashes_symbols_literal_utf8_and_source_newlines() {
        let parsed = parse_rtf("{\\rtf1 Tiếng\r\n Việt \\\\ \\~\\_\\-}").unwrap();
        assert_eq!(text(&parsed.target_document), "Tiếng Việt \\ \u{00a0}\u{2011}\u{00ad}");
    }

    #[test]
    fn unsupported_formatting_warns_once_without_removing_text() {
        let parsed = parse_rtf(r"{\rtf1\super A\super B\uld C}").unwrap();
        assert_eq!(text(&parsed.target_document), "ABC");
        assert_eq!(parsed.warnings.iter().filter(|warning| warning.contains("\\super")).count(), 1);
        assert!(parsed.warnings.iter().any(|warning| warning.contains("single underline")));
    }

    #[test]
    fn malformed_headers_groups_controls_and_unicode_abort() {
        for input in [
            "", "plain", r"{\rtf2 x}", r"{\rtf1 x", r"{\rtf1 x}}", r"{\rtf1 x}{\rtf1 y}",
            r"{\rtf1 {x}", r"{\rtf1\b- x}", r"{\rtf1\fs2147483648 x}", r"{\rtf1\fs-1 x}",
            r"{\rtf1\fs x}", r"{\rtf1\uc-1 x}", r"{\rtf1\u32768?}", r"{\rtf1\u-32769?}",
            r"{\rtf1\fs20001 x}", r"{\rtf1\b2 x}",
            r"{\rtf1\u x}", r"{\rtf1\u-10179?}", r"{\rtf1\u-8704?}", r"{\rtf1\u-10179?x}",
            r"{\rtf1\u-10179?\b\u-8704?}", r"{\rtf1\'zz}", r"{\rtf1\'a}",
            r"{\rtf1\* x}", r"{\rtf1{\*}}", r"{\rtf1\bin20 x}",
            r"{\rtf1{\fonttbl{\f0 Arial}}x}", r"{\rtf1{\colortbl;\red256;}x}",
            r"{\rtf1{\upr{one}}}", r"{\rtf1{\upr{one}{two}}}",
        ] {
            assert!(parse_rtf(input).is_err(), "unexpectedly accepted {input:?}");
        }
    }

    #[test]
    fn malformed_or_unavailable_byte_encodings_never_replace_text() {
        for input in [
            r"{\rtf1\ansicpg932\'82}", r"{\rtf1\ansicpg65001\'ff}", r"{\rtf1\ansicpg99999 x}",
            r"{\rtf1\ansicpg28591\'80}",
            r"{\rtf1{\fonttbl{\f0\fcharset2 Symbol;}}\f0\'e0}",
        ] {
            let error = parse_rtf(input).unwrap_err();
            assert_eq!(error.code, "invalidRtfEncoding", "{input}");
        }
    }

    #[test]
    fn empty_and_blank_paragraphs_are_valid_editor_nodes() {
        let empty = parse_rtf(r"{\rtf1}").unwrap();
        assert_eq!(empty.target_document, json!({ "type": "doc", "content": [{ "type": "paragraph", "attrs": { "textAlign": "left" }, "content": [] }] }));
        let parsed = parse_rtf(r"{\rtf1 a\par\par b\par}").unwrap();
        assert_eq!(text(&parsed.target_document), "a\n\nb");
    }
}
