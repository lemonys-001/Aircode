//! A character grid plus an ANSI diff emitter.
//!
//! Redrawing a whole terminal frame every tick would flicker and waste bytes.
//! Instead each frame is drawn into a fresh `Screen`, then diffed against the
//! previous one so only changed cells are emitted. The result is flicker-free
//! and stays cheap even while the agent streams.

use super::term::char_width;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: 0xE2E8F0,
            bg: 0x0B0F14,
            bold: false,
        }
    }
}

pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    grid: Vec<Cell>,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            grid: vec![Cell::default(); cols * rows],
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if self.cols == cols && self.rows == rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.grid = vec![Cell::default(); cols * rows];
    }

    pub fn cell(&self, x: usize, y: usize) -> Cell {
        if x >= self.cols || y >= self.rows {
            return Cell::default();
        }
        self.grid[y * self.cols + x]
    }

    /// Paints a run of `n` cells starting at (x, y). Clipped at the right edge;
    /// a cell is only written when it actually changes, which keeps the diff
    /// small.
    pub fn fill(&mut self, x: usize, y: usize, n: usize, fg: u32, bg: u32, bold: bool) {
        for i in 0..n {
            let cx = x + i;
            if cx >= self.cols || y >= self.rows {
                break;
            }
            let c = &mut self.grid[y * self.cols + cx];
            if c.ch != ' ' || c.fg != fg || c.bg != bg || c.bold != bold {
                *c = Cell {
                    ch: ' ',
                    fg,
                    bg,
                    bold,
                };
            }
        }
    }

    /// Places a single cell, clipped. Used by the line painter so it does not
    /// need to know about the grid's layout.
    pub fn put(&mut self, x: usize, y: usize, ch: char, fg: u32, bg: u32, bold: bool) {
        if x >= self.cols || y >= self.rows {
            return;
        }
        self.grid[y * self.cols + x] = Cell { ch, fg, bg, bold };
    }

    /// Draws text starting at (x, y), wrapping nothing and clipping at the edge.
    /// Returns the number of columns consumed.
    pub fn text(&mut self, x: usize, y: usize, s: &str, fg: u32, bg: u32, bold: bool) -> usize {
        let mut cx = x;
        for ch in s.chars() {
            if ch == '\n' || ch == '\r' || ch == '\t' {
                // Tabs and newlines are layout decisions for the caller.
                continue;
            }
            let w = char_width(ch);
            if w == 0 {
                continue;
            }
            if cx + w > self.cols || y >= self.rows {
                break;
            }
            let idx = y * self.cols + cx;
            self.grid[idx] = Cell { ch, fg, bg, bold };
            if w == 2 {
                // The trailing half of a wide glyph is left blank so the diff
                // stays stable when a narrow glyph is replaced by a wide one.
                if cx + 1 < self.cols {
                    self.grid[y * self.cols + cx + 1] = Cell {
                        ch: ' ',
                        fg,
                        bg,
                        bold,
                    };
                }
            }
            cx += w;
        }
        cx - x
    }

    /// Draws text and pads the remainder of the line with `bg`.
    pub fn text_line(&mut self, x: usize, y: usize, s: &str, fg: u32, bg: u32, bold: bool) {
        let used = self.text(x, y, s, fg, bg, bold);
        if self.cols > x + used {
            self.fill(x + used, y, self.cols - (x + used), fg, bg, bold);
        }
    }
}

/// Emits the ANSI sequence that turns `prev` into `next`.
///
/// Only cells that differ are written, grouped into horizontal runs so a row
/// costs one cursor move plus one SGR (when the colour changes) per run.
pub fn render_diff(out: &mut String, prev: &Screen, next: &Screen) {
    if prev.cols != next.cols || prev.rows != next.rows {
        out.push_str("\x1b[2J");
    }
    let mut cur_fg: Option<u32> = None;
    let mut cur_bg: Option<u32> = None;
    let mut cur_bold: Option<bool> = None;

    for y in 0..next.rows {
        let mut x = 0;
        while x < next.cols {
            let a = prev.cell(x, y);
            let b = next.cell(x, y);
            if a == b {
                x += 1;
                continue;
            }
            // Extend the run over consecutive differing cells.
            let start = x;
            let mut end = x;
            while end < next.cols {
                let pa = prev.cell(end, y);
                let pb = next.cell(end, y);
                if pa == pb {
                    break;
                }
                end += 1;
            }

            // Cursor moves are 1-based.
            out.push_str("\x1b[");
            out.push_str(&(y + 1).to_string());
            out.push(';');
            out.push_str(&(start + 1).to_string());
            out.push('H');

            if cur_fg != Some(b.fg) || cur_bg != Some(b.bg) || cur_bold != Some(b.bold) {
                out.push_str("\x1b[0");
                if b.bold {
                    out.push_str(";1");
                }
                out.push_str(";38;2;");
                out.push_str(&((b.fg >> 16) & 0xff).to_string());
                out.push(';');
                out.push_str(&((b.fg >> 8) & 0xff).to_string());
                out.push(';');
                out.push_str(&(b.fg & 0xff).to_string());
                out.push_str(";48;2;");
                out.push_str(&((b.bg >> 16) & 0xff).to_string());
                out.push(';');
                out.push_str(&((b.bg >> 8) & 0xff).to_string());
                out.push(';');
                out.push_str(&(b.bg & 0xff).to_string());
                out.push('m');
                cur_fg = Some(b.fg);
                cur_bg = Some(b.bg);
                cur_bold = Some(b.bold);
            }

            for cx in start..end {
                let c = next.cell(cx, y);
                out.push(c.ch);
            }
            x = end;
        }
    }
    // Nothing else is appended: a frame that changed nothing must be a
    // zero-length string, otherwise the main loop would write to the terminal
    // on every tick even while idle.
}
