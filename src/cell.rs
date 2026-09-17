use crate::Style;
use unicode_width::UnicodeWidthStr;

/// One displayed grapheme cluster and its style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    ch: char,
    pub style: Style,
    extra: Option<Box<str>>,
}

impl Cell {
    pub fn scalar(&self) -> char {
        self.ch
    }
    pub fn new(ch: char, style: Style) -> Self {
        let ch = if ch.is_control() || unicode_width::UnicodeWidthChar::width(ch) == Some(0) {
            '\u{fffd}'
        } else {
            ch
        };
        Self {
            ch,
            style,
            extra: None,
        }
    }
    /// Returns `None` for an empty string or more than one grapheme cluster.
    pub fn grapheme(text: &str, style: Style) -> Option<Self> {
        use unicode_segmentation::UnicodeSegmentation;
        if UnicodeSegmentation::graphemes(text, true).count() != 1 {
            return None;
        }
        let mut chars = text.chars();
        let ch = chars.next()?;
        let rest = chars.as_str();
        if ch.is_control() || UnicodeWidthStr::width(text) == 0 {
            return Some(Self::new('\u{fffd}', style));
        }
        if rest.chars().any(char::is_control) {
            return Some(Self::new('\u{fffd}', style));
        }
        Some(Self {
            ch,
            style,
            extra: (!rest.is_empty()).then(|| rest.into()),
        })
    }
    pub fn text(&self) -> String {
        let mut text = self.ch.to_string();
        if let Some(extra) = &self.extra {
            text.push_str(extra);
        }
        text
    }
    pub(crate) fn append_to(&self, out: &mut Vec<u8>) {
        let mut buf = [0; 4];
        out.extend_from_slice(self.ch.encode_utf8(&mut buf).as_bytes());
        if let Some(extra) = &self.extra {
            out.extend_from_slice(extra.as_bytes());
        }
    }
    pub(crate) fn width(&self) -> usize {
        UnicodeWidthStr::width(self.text().as_str())
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::new(' ', Style::default())
    }
}
