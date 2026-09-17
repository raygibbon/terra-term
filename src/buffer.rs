use crate::{Cell, Size, Style};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Slot {
    pub cell: Cell,
    pub continuation: bool,
}

pub(crate) struct Buffer {
    pub size: Size,
    pub cells: Vec<Slot>,
}
impl Buffer {
    pub fn new(size: Size) -> Self {
        Self {
            size,
            cells: vec![Slot::default(); usize::from(size.width) * usize::from(size.height)],
        }
    }
    pub fn resize(&mut self, size: Size) {
        *self = Self::new(size);
    }
    pub fn clear_with_style(&mut self, style: Style) {
        self.cells.fill(Slot {
            cell: Cell::new(' ', style),
            continuation: false,
        });
    }
    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.size.width && y < self.size.height)
            .then(|| usize::from(y) * usize::from(self.size.width) + usize::from(x))
    }
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        let slot = self.cells.get(self.index(x, y)?)?;
        (!slot.continuation).then_some(&slot.cell)
    }
    pub fn set_cell_style(&mut self, x: u16, y: u16, style: Style) -> bool {
        let Some(index) = self.index(x, y) else {
            return false;
        };
        if self.cells[index].continuation {
            return false;
        }
        self.cells[index].cell.style = style;
        if x + 1 < self.size.width && self.cells[index + 1].continuation {
            self.cells[index + 1].cell.style = style;
        }
        true
    }
    pub fn extend_cell(&mut self, x: u16, y: u16, suffix: &str) -> bool {
        let Some(existing) = self.cell(x, y) else {
            return false;
        };
        let mut text = existing.text();
        text.push_str(suffix);
        let Some(cell) = Cell::grapheme(&text, existing.style) else {
            return false;
        };
        if cell.width() == 2 && usize::from(x) + 1 >= usize::from(self.size.width) {
            return false;
        }
        self.put_cell(x, y, cell);
        true
    }
    fn erase(&mut self, index: usize) {
        if self.cells[index].continuation && index > 0 {
            self.cells[index - 1] = Slot::default();
        }
        if !self.cells[index].continuation
            && index + 1 < self.cells.len()
            && self.cells[index + 1].continuation
        {
            self.cells[index + 1] = Slot::default();
        }
        self.cells[index] = Slot::default();
    }
    pub fn put_char(&mut self, x: u16, y: u16, ch: char, style: Style) {
        self.put_cell(x, y, Cell::new(ch, style));
    }
    pub fn put_cell(&mut self, x: u16, y: u16, cell: Cell) {
        let Some(i) = self.index(x, y) else {
            return;
        };
        let width = cell.width();
        if width == 0 || width > 2 {
            return;
        }
        if width == 2 && x >= self.size.width - 1 {
            return;
        }
        self.erase(i);
        if width == 2 {
            self.erase(i + 1);
        }
        let style = cell.style;
        self.cells[i] = Slot {
            cell,
            continuation: false,
        };
        if width == 2 {
            self.cells[i + 1] = Slot {
                cell: Cell::new(' ', style),
                continuation: true,
            };
        }
    }
    pub fn put_str(&mut self, x: u16, y: u16, text: &str, style: Style) {
        let (mut x, mut y) = (usize::from(x), usize::from(y));
        let start_x = x;
        for grapheme in UnicodeSegmentation::graphemes(text, true) {
            if grapheme == "\n" {
                y += 1;
                x = start_x;
                continue;
            }
            let Some(cell) = Cell::grapheme(grapheme, style) else {
                continue;
            };
            let width = cell.width();
            if width == 0 {
                continue;
            }
            if x < usize::from(self.size.width) && y < usize::from(self.size.height) {
                self.put_cell(x as u16, y as u16, cell);
            }
            x = x.saturating_add(width);
        }
    }
    pub fn dirty_rows(&self, front: &[Slot], invalid: bool) -> Vec<u16> {
        let w = usize::from(self.size.width);
        if w == 0 {
            return vec![];
        }
        (0..self.size.height)
            .filter(|&y| {
                let start = usize::from(y) * w;
                invalid || self.cells[start..start + w] != front[start..start + w]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_and_string_drawing() {
        let mut b = Buffer::new(Size {
            width: 4,
            height: 2,
        });
        b.put_str(2, 0, "abcd", Style::default());
        b.put_char(100, 100, 'x', Style::default());
        assert_eq!(b.cells[2].cell.scalar(), 'a');
        assert_eq!(b.cells[3].cell.scalar(), 'b');
        assert_eq!(b.cells[4].cell.scalar(), ' ');
    }

    #[test]
    fn wide_glyphs_reserve_and_clear_both_cells() {
        let mut b = Buffer::new(Size {
            width: 4,
            height: 1,
        });
        b.put_str(0, 0, "界x", Style::default());
        assert_eq!(b.cells[0].cell.scalar(), '界');
        assert!(b.cells[1].continuation);
        assert_eq!(b.cells[2].cell.scalar(), 'x');
        b.put_char(1, 0, 'z', Style::default());
        assert_eq!(b.cells[0].cell.scalar(), ' ');
        assert_eq!(b.cells[1].cell.scalar(), 'z');
    }

    #[test]
    fn resize_and_dirty_rows() {
        let mut b = Buffer::new(Size {
            width: 2,
            height: 2,
        });
        let front = b.cells.clone();
        assert!(b.dirty_rows(&front, false).is_empty());
        b.put_char(0, 1, 'a', Style::default());
        assert_eq!(b.dirty_rows(&front, false), vec![1]);
        b.resize(Size {
            width: 3,
            height: 1,
        });
        assert_eq!(b.cells.len(), 3);
        assert!(b.cells.iter().all(|s| *s == Slot::default()));
    }
    #[test]
    fn composed_graphemes_and_newlines() {
        let mut b = Buffer::new(Size {
            width: 8,
            height: 2,
        });
        b.put_str(1, 0, "e\u{301}界\n👩‍💻", Style::default());
        assert_eq!(b.cells[1].cell.text(), "e\u{301}");
        assert_eq!(b.cells[2].cell.text(), "界");
        assert!(b.cells[3].continuation);
        assert_eq!(b.cells[9].cell.text(), "👩‍💻");
        assert!(b.cells[10].continuation);
    }
    #[test]
    fn control_and_zero_width_cells_are_visible_replacements() {
        let mut b = Buffer::new(Size {
            width: 4,
            height: 1,
        });
        b.put_char(0, 0, '\0', Style::default());
        b.put_char(1, 0, '\u{200d}', Style::default());
        b.put_str(2, 0, "\u{301}", Style::default());
        assert_eq!(b.cell(0, 0).unwrap().scalar(), '\u{fffd}');
        assert_eq!(b.cell(1, 0).unwrap().scalar(), '\u{fffd}');
        assert_eq!(b.cell(2, 0).unwrap().scalar(), '\u{fffd}');
    }
    #[test]
    fn extends_a_cluster_without_exposing_continuation() {
        let mut b = Buffer::new(Size {
            width: 3,
            height: 1,
        });
        b.put_char(0, 0, 'e', Style::default());
        assert!(b.extend_cell(0, 0, "\u{301}"));
        assert_eq!(b.cell(0, 0).unwrap().text(), "e\u{301}");
        assert!(!b.extend_cell(0, 0, "x"));
        b.put_char(1, 0, '👩', Style::default());
        assert!(b.extend_cell(1, 0, "\u{200d}💻"));
        assert_eq!(b.cell(1, 0).unwrap().text(), "👩‍💻");
        assert!(b.cell(2, 0).is_none());
    }
    #[test]
    fn changing_wide_cell_style_updates_continuation() {
        let mut b = Buffer::new(Size {
            width: 2,
            height: 1,
        });
        b.put_char(0, 0, '界', Style::default());
        let style = Style {
            bold: true,
            ..Style::default()
        };
        assert!(b.set_cell_style(0, 0, style));
        assert_eq!(b.cells[0].cell.style, style);
        assert_eq!(b.cells[1].cell.style, style);
        assert!(!b.set_cell_style(1, 0, style));
    }
}
