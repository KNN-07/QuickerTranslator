use std::fs;
use std::path::Path;

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};

use crate::models::{AppError, AppResult};

#[derive(Debug)]
pub struct DecodedText {
    pub text: String,
    pub encoding: String,
    pub detected: bool,
}

pub fn read_text(path: &Path, override_encoding: Option<&str>) -> AppResult<DecodedText> {
    let bytes = fs::read(path).map_err(|error| {
        AppError::io("dictionaryFileUnavailable", "The selected file could not be read; choose or remap that file", &error)
    })?;
    decode_bytes(&bytes, override_encoding)
}

/// BOMs are authoritative. Without one, a user override is authoritative;
/// otherwise valid UTF-8 is preferred over a visible statistical guess.
pub fn decode_bytes(bytes: &[u8], override_encoding: Option<&str>) -> AppResult<DecodedText> {
    if bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00]) || bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff]) {
        return Err(AppError::new("encodingUnavailable", "UTF-32 is not supported. Convert this file to UTF-8 or UTF-16 before importing."));
    }
    let (encoding, skip, detected) = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (UTF_8, 3, false)
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        (UTF_16LE, 2, false)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        (UTF_16BE, 2, false)
    } else if let Some(label) = override_encoding.filter(|label| !label.trim().is_empty()) {
        let encoding = Encoding::for_label(label.trim().as_bytes()).ok_or_else(|| {
            AppError::new("encodingUnavailable", "Choose a supported text encoding, such as UTF-8, UTF-16LE/BE, GB18030, GBK, Big5, Shift-JIS, EUC-JP or Windows-1258.")
        })?;
        (encoding, 0, false)
    } else if std::str::from_utf8(bytes).is_ok() {
        (UTF_8, 0, false)
    } else {
        let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
        detector.feed(bytes, true);
        (detector.guess(None, Utf8Detection::Allow), 0, true)
    };
    let content = &bytes[skip..];
    let text = if encoding == UTF_16LE || encoding == UTF_16BE {
        if content.len() % 2 != 0 { return Err(invalid_encoding()); }
        let units = content.chunks_exact(2).map(|pair| {
            if encoding == UTF_16LE { u16::from_le_bytes([pair[0], pair[1]]) }
            else { u16::from_be_bytes([pair[0], pair[1]]) }
        });
        let mut result = String::with_capacity(content.len());
        for scalar in char::decode_utf16(units) {
            result.push(scalar.map_err(|_| invalid_encoding())?);
        }
        result
    } else {
        encoding.decode_without_bom_handling_and_without_replacement(content)
            .ok_or_else(invalid_encoding)?.into_owned()
    };
    Ok(DecodedText { text, encoding: encoding.name().to_owned(), detected })
}

fn invalid_encoding() -> AppError {
    AppError::new("invalidEncoding", "The selected encoding cannot decode all bytes. No replacement characters were inserted; choose another encoding and preview again.")
}
