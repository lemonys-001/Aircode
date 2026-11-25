//! Pure-Rust ANSI / VT100 terminal screen buffer & escape sequence parser for ConPTY / PTY tabs.

const DEFAULT_FG: u32 = 0xE2E8F0;
const DEFAULT_BG: u32 = 0x070B12;

#[derive(Clone, Copy, Debug)]
pub struct VtCell {
    pub ch: char,
    pub fg: u32,
    pub bg: u32,
}

impl Default for VtCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParseState {
    Ground,
    Escape,
    Csi,
    Osc,
    OscEsc,
}

pub struct VtScreen {
    pub cols: usize,
    pub rows: usize,
    pub grid: Vec<Vec<VtCell>>,
    pub scrollback: Vec<Vec<VtCell>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scroll_offset: usize,
    cur_fg: u32,
    cur_bg: u32,
    bold: bool,
    state: ParseState,
    csi_buf: String,
    osc_buf: String,
    pub window_title: String,
}

impl VtScreen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let c = cols.max(20);
        let r = rows.max(5);
        Self {
            cols: c,
            rows: r,
            grid: vec![vec![VtCell::default(); c]; r],
            scrollback: Vec::new(),
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            cur_fg: DEFAULT_FG,
            cur_bg: DEFAULT_BG,
            bold: false,
            state: ParseState::Ground,
            csi_buf: String::new(),
            osc_buf: String::new(),
            window_title: String::new(),
        }
    }

    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        let c = new_cols.max(20);
        let r = new_rows.max(5);
        if c == self.cols && r == self.rows {
            return;
        }
        for row in &mut self.grid {
            row.resize(c, VtCell::default());
        }
        self.grid.resize(r, vec![VtCell::default(); c]);
        self.cols = c;
        self.rows = r;
        self.cursor_row = self.cursor_row.min(r - 1);
        self.cursor_col = self.cursor_col.min(c - 1);
    }

    pub fn feed_bytes(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        self.feed_str(&text);
    }

    pub fn feed_str(&mut self, text: &str) {
        self.scroll_offset = 0;
        for ch in text.chars() {
            match self.state {
                ParseState::Ground => match ch {
                    '\x1b' => {
                        self.state = ParseState::Escape;
                    }
                    '\r' => {
                        self.cursor_col = 0;
                    }
                    '\n' => {
                        self.line_feed();
                    }
                    '\x08' => {
                        self.cursor_col = self.cursor_col.saturating_sub(1);
                    }
                    '\t' => {
                        let next_stop = ((self.cursor_col / 8) + 1) * 8;
                        self.cursor_col = next_stop.min(self.cols - 1);
                    }
                    '\x07' => {}
                    c if c >= ' ' => {
                        self.put_char(c);
                    }
                    _ => {}
                },
                ParseState::Escape => match ch {
                    '[' => {
                        self.csi_buf.clear();
                        self.state = ParseState::Csi;
                    }
                    ']' => {
                        self.osc_buf.clear();
                        self.state = ParseState::Osc;
                    }
                    'c' => {
                        // Full reset
                        self.grid = vec![vec![VtCell::default(); self.cols]; self.rows];
                        self.cursor_row = 0;
                        self.cursor_col = 0;
                        self.cur_fg = DEFAULT_FG;
                        self.cur_bg = DEFAULT_BG;
                        self.state = ParseState::Ground;
                    }
                    _ => {
                        self.state = ParseState::Ground;
                    }
                },
                ParseState::Osc => match ch {
                    '\x07' => {
                        self.finish_osc();
                        self.state = ParseState::Ground;
                    }
                    '\x1b' => {
                        self.state = ParseState::OscEsc;
                    }
                    c => {
                        if self.osc_buf.len() < 256 {
                            self.osc_buf.push(c);
                        }
                    }
                },
                ParseState::OscEsc => {
                    if ch == '\\' {
                        self.finish_osc();
                    }
                    self.state = ParseState::Ground;
                }
                ParseState::Csi => {
                    if ('\x40'..='\x7e').contains(&ch) {
                        self.dispatch_csi(ch);
                        self.state = ParseState::Ground;
                    } else if self.csi_buf.len() < 128 {
                        self.csi_buf.push(ch);
                    } else {
                        self.state = ParseState::Ground;
                    }
                }
            }
        }
    }

    fn finish_osc(&mut self) {
        if let Some(rest) = self
            .osc_buf
            .strip_prefix("0;")
            .or_else(|| self.osc_buf.strip_prefix("2;"))
        {
            self.window_title = rest.to_string();
        }
    }

    fn put_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.line_feed();
        }
        self.grid[self.cursor_row][self.cursor_col] = VtCell {
            ch,
            fg: self.cur_fg,
            bg: self.cur_bg,
        };
        self.cursor_col += 1;
    }

    fn line_feed(&mut self) {
        if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        } else {
            let evicted = self.grid.remove(0);
            if self.scrollback.len() >= 1500 {
                self.scrollback.remove(0);
            }
            self.scrollback.push(evicted);
            self.grid.push(vec![VtCell::default(); self.cols]);
        }
    }

    fn dispatch_csi(&mut self, final_byte: char) {
        let params_raw = self.csi_buf.trim_start_matches('?').to_string();
        let nums: Vec<usize> = if params_raw.is_empty() {
            Vec::new()
        } else {
            params_raw
                .split(';')
                .map(|p| p.parse::<usize>().unwrap_or(0))
                .collect()
        };

        match final_byte {
            'A' => {
                let n = nums.first().copied().unwrap_or(1).max(1);
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            'B' => {
                let n = nums.first().copied().unwrap_or(1).max(1);
                self.cursor_row = (self.cursor_row + n).min(self.rows - 1);
            }
            'C' => {
                let n = nums.first().copied().unwrap_or(1).max(1);
                self.cursor_col = (self.cursor_col + n).min(self.cols - 1);
            }
            'D' => {
                let n = nums.first().copied().unwrap_or(1).max(1);
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            'G' => {
                let col = nums.first().copied().unwrap_or(1).max(1) - 1;
                self.cursor_col = col.min(self.cols - 1);
            }
            'H' | 'f' => {
                let r = nums.first().copied().unwrap_or(1).max(1) - 1;
                let c = nums.get(1).copied().unwrap_or(1).max(1) - 1;
                self.cursor_row = r.min(self.rows - 1);
                self.cursor_col = c.min(self.cols - 1);
            }
            'J' => {
                let mode = nums.first().copied().unwrap_or(0);
                match mode {
                    2 | 3 => {
                        self.grid = vec![vec![VtCell::default(); self.cols]; self.rows];
                        self.cursor_row = 0;
                        self.cursor_col = 0;
                    }
                    0 => {
                        for c in self.cursor_col..self.cols {
                            self.grid[self.cursor_row][c] = VtCell::default();
                        }
                        for r in (self.cursor_row + 1)..self.rows {
                            self.grid[r] = vec![VtCell::default(); self.cols];
                        }
                    }
                    _ => {}
                }
            }
            'K' => {
                let mode = nums.first().copied().unwrap_or(0);
                match mode {
                    0 => {
                        for c in self.cursor_col..self.cols {
                            self.grid[self.cursor_row][c] = VtCell::default();
                        }
                    }
                    1 => {
                        for c in 0..=self.cursor_col.min(self.cols - 1) {
                            self.grid[self.cursor_row][c] = VtCell::default();
                        }
                    }
                    2 => {
                        self.grid[self.cursor_row] = vec![VtCell::default(); self.cols];
                    }
                    _ => {}
                }
            }
            'X' => {
                let count = nums.first().copied().unwrap_or(1).max(1);
                for c in self.cursor_col..(self.cursor_col + count).min(self.cols) {
                    self.grid[self.cursor_row][c] = VtCell::default();
                }
            }
            'm' => {
                if nums.is_empty() {
                    self.cur_fg = DEFAULT_FG;
                    self.cur_bg = DEFAULT_BG;
                    self.bold = false;
                    return;
                }
                let mut i = 0usize;
                while i < nums.len() {
                    match nums[i] {
                        0 => {
                            self.cur_fg = DEFAULT_FG;
                            self.cur_bg = DEFAULT_BG;
                            self.bold = false;
                        }
                        1 => self.bold = true,
                        22 => self.bold = false,
                        39 => self.cur_fg = DEFAULT_FG,
                        49 => self.cur_bg = DEFAULT_BG,
                        c @ 30..=37 => {
                            self.cur_fg = ansi_16_rgb(c - 30 + if self.bold { 8 } else { 0 })
                        }
                        c @ 90..=97 => self.cur_fg = ansi_16_rgb(c - 90 + 8),
                        c @ 40..=47 => self.cur_bg = ansi_16_rgb(c - 40),
                        c @ 100..=107 => self.cur_bg = ansi_16_rgb(c - 100 + 8),
                        38 | 48 => {
                            let is_fg = nums[i] == 38;
                            if nums.get(i + 1) == Some(&5) && i + 2 < nums.len() {
                                let rgb = ansi_256_rgb(nums[i + 2] as u8);
                                if is_fg {
                                    self.cur_fg = rgb;
                                } else {
                                    self.cur_bg = rgb;
                                }
                                i += 2;
                            } else if nums.get(i + 1) == Some(&2) && i + 4 < nums.len() {
                                let r = (nums[i + 2] & 0xff) as u32;
                                let g = (nums[i + 3] & 0xff) as u32;
                                let b = (nums[i + 4] & 0xff) as u32;
                                let rgb = (r << 16) | (g << 8) | b;
                                if is_fg {
                                    self.cur_fg = rgb;
                                } else {
                                    self.cur_bg = rgb;
                                }
                                i += 4;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {}
        }
    }

    pub fn visible_row(&self, view_row: usize) -> Option<&[VtCell]> {
        let total_hist = self.scrollback.len();
        let effective_off = self.scroll_offset.min(total_hist);
        if effective_off == 0 {
            self.grid.get(view_row).map(|v| v.as_slice())
        } else {
            let start = total_hist - effective_off;
            let idx = start + view_row;
            if idx < total_hist {
                Some(self.scrollback[idx].as_slice())
            } else {
                self.grid.get(idx - total_hist).map(|v| v.as_slice())
            }
        }
    }
}

fn ansi_16_rgb(idx: usize) -> u32 {
    match idx & 15 {
        0 => 0x0F172A,
        1 => 0xEF4444,
        2 => 0x22C55E,
        3 => 0xEAB308,
        4 => 0x3B82F6,
        5 => 0xA855F7,
        6 => 0x06B6D4,
        7 => 0xCBD5E1,
        8 => 0x475569,
        9 => 0xF87171,
        10 => 0x34D399,
        11 => 0xFBBF24,
        12 => 0x60A5FA,
        13 => 0xC084FC,
        14 => 0x22D3EE,
        _ => 0xF8FAFC,
    }
}

fn ansi_256_rgb(code: u8) -> u32 {
    if code < 16 {
        return ansi_16_rgb(code as usize);
    }
    if code < 232 {
        let idx = code - 16;
        let r = (idx / 36) % 6;
        let g = (idx / 6) % 6;
        let b = idx % 6;
        let map = |v: u8| if v == 0 { 0u32 } else { 55 + v as u32 * 40 };
        return (map(r) << 16) | (map(g) << 8) | map(b);
    }
    let gray = 8 + (code as u32 - 232) * 10;
    (gray << 16) | (gray << 8) | gray
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ansi_colors_and_cursor() {
        let mut s = VtScreen::new(80, 24);
        s.feed_str("\x1b[32mPS C:\\Work>\x1b[0m cargo build\r\nDone.");
        assert_eq!(s.grid[0][0].ch, 'P');
        assert_eq!(s.grid[0][0].fg, 0x22C55E);
        assert_eq!(s.grid[1][0].ch, 'D');
    }
}
