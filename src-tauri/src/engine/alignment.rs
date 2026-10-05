use std::ops::Range;

use crate::models::{AppError, AppResult, TextRange};

#[derive(Debug, Clone, Copy)]
struct Boundary {
    byte: usize,
    utf16: u32,
}

/// One immutable scalar-boundary map per source revision.
///
/// Both directions reject offsets inside a UTF-8 scalar or a UTF-16 surrogate
/// pair rather than rounding a selection into unrelated text. Combining marks
/// are distinct scalars; editor selections may legitimately include them alone.
#[derive(Debug)]
pub struct OffsetMap {
    boundaries: Vec<Boundary>,
}

impl OffsetMap {
    pub fn new(text: &str) -> AppResult<Self> {
        let mut boundaries = Vec::new();
        boundaries.push(Boundary { byte: 0, utf16: 0 });
        let mut utf16 = 0_u32;
        for (byte, scalar) in text.char_indices() {
            utf16 = utf16.checked_add(scalar.len_utf16() as u32).ok_or_else(|| {
                AppError::new("textTooLarge", "Text exceeds the supported UTF-16 offset range.")
            })?;
            boundaries.push(Boundary {
                byte: byte + scalar.len_utf8(),
                utf16,
            });
        }
        Ok(Self { boundaries })
    }

    pub fn byte_len(&self) -> usize {
        self.boundaries.last().expect("map contains its origin").byte
    }

    pub fn utf16_len(&self) -> u32 {
        self.boundaries.last().expect("map contains its origin").utf16
    }

    pub fn scalar_len(&self) -> usize {
        self.boundaries.len() - 1
    }

    pub fn byte_to_utf16(&self, byte: usize) -> AppResult<u32> {
        self.boundaries
            .binary_search_by_key(&byte, |boundary| boundary.byte)
            .map(|index| self.boundaries[index].utf16)
            .map_err(|_| invalid_range())
    }

    pub fn utf16_to_byte(&self, utf16: u32) -> AppResult<usize> {
        self.boundaries
            .binary_search_by_key(&utf16, |boundary| boundary.utf16)
            .map(|index| self.boundaries[index].byte)
            .map_err(|_| invalid_range())
    }

    pub fn byte_range_to_utf16(&self, range: Range<usize>) -> AppResult<TextRange> {
        if range.start > range.end {
            return Err(invalid_range());
        }
        Ok(TextRange {
            start: self.byte_to_utf16(range.start)?,
            end: self.byte_to_utf16(range.end)?,
        })
    }

    pub fn utf16_range_to_byte(&self, range: TextRange) -> AppResult<Range<usize>> {
        if range.start > range.end {
            return Err(invalid_range());
        }
        Ok(self.utf16_to_byte(range.start)?..self.utf16_to_byte(range.end)?)
    }

    pub fn validate_range(&self, range: TextRange) -> AppResult<()> {
        self.utf16_range_to_byte(range).map(|_| ())
    }
}

fn invalid_range() -> AppError {
    AppError::new(
        "invalidTextRange",
        "Text range is out of bounds, reversed, or splits a Unicode scalar.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_fixture_emoji_occupies_two_utf16_units() {
        let text = "张三，你好。🙂";
        let map = OffsetMap::new(text).unwrap();
        assert_eq!(map.scalar_len(), 7);
        assert_eq!(map.utf16_len(), 8);
        assert_eq!(map.byte_range_to_utf16(18..22).unwrap(), TextRange { start: 6, end: 8 });
        assert_eq!(map.utf16_range_to_byte(TextRange { start: 6, end: 8 }).unwrap(), 18..22);
        assert!(map.utf16_to_byte(7).is_err());
    }

    #[test]
    fn every_valid_boundary_and_half_open_span_round_trips() {
        for text in ["", "ASCII\r\n", "私は学校で", "Tiếng Việt", "a\u{301}\u{1f642}\u{200d}\u{1f4bb}", "\0\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}\u{10000}\u{10ffff}"] {
            let map = OffsetMap::new(text).unwrap();
            let bytes: Vec<_> = text.char_indices().map(|(offset, _)| offset).chain([text.len()]).collect();
            for (index, &start) in bytes.iter().enumerate() {
                for &end in &bytes[index..] {
                    let utf16 = map.byte_range_to_utf16(start..end).unwrap();
                    assert_eq!(map.utf16_range_to_byte(utf16).unwrap(), start..end);
                    assert_eq!(utf16.len().unwrap() as usize, text[start..end].encode_utf16().count());
                }
            }
            for byte in 0..=text.len() {
                assert_eq!(map.byte_to_utf16(byte).is_ok(), text.is_char_boundary(byte));
            }
            assert!(map.byte_to_utf16(text.len() + 1).is_err());
            assert!(map.utf16_to_byte(map.utf16_len() + 1).is_err());
        }
    }

    #[test]
    fn neither_encoding_accepts_reversed_or_split_ranges() {
        let map = OffsetMap::new("你🙂好").unwrap();
        assert!(map.byte_range_to_utf16(3..2).is_err());
        assert!(map.byte_range_to_utf16(1..3).is_err());
        assert!(map.byte_range_to_utf16(3..6).is_err());
        assert!(map.utf16_range_to_byte(TextRange { start: 3, end: 1 }).is_err());
        assert!(map.utf16_range_to_byte(TextRange { start: 1, end: 2 }).is_err());
        assert!(map.utf16_range_to_byte(TextRange { start: 2, end: 3 }).is_err());
        assert_eq!(map.utf16_range_to_byte(TextRange { start: 4, end: 4 }).unwrap(), 10..10);
    }
}
