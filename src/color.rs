/// A terminal colour. Indexed values 0..=15 are the standard palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Foreground, background, and basic SGR attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub dim: bool,
    pub underline: bool,
    pub reverse: bool,
    pub italic: bool,
    pub blink: bool,
    pub strikeout: bool,
    pub double_underline: bool,
    pub overline: bool,
    pub invisible: bool,
}

impl Style {
    pub const fn new(foreground: Color, background: Color) -> Self {
        Self {
            foreground,
            background,
            bold: false,
            dim: false,
            underline: false,
            reverse: false,
            italic: false,
            blink: false,
            strikeout: false,
            double_underline: false,
            overline: false,
            invisible: false,
        }
    }
}
