//! Terminal plumbing for the TUI front-end: raw mode, size and input decoding.
//!
//! Everything here is deliberately dependency-free. On Unix the console is put
//! into raw mode with a `termios` FFI whose layout is identical under glibc and
//! musl (both use `NCCS == 32` on Linux), so the same code cross-compiles for
//! the static musl targets. On Windows the equivalent is
//! `GetConsoleMode`/`SetConsoleMode`.
//!
//! If raw mode cannot be established — no controlling TTY, or a console that
//! refuses the request — the TUI falls back to cooked line mode rather than
//! refusing to start. Keys still work; they just arrive one line at a time.

use std::io::{Read, Write};
use std::time::Duration;

use crate::platform::{KeyCode, Modifiers};

/// A decoded key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// A named key.
    Named(KeyCode),
    /// A control combination, e.g. ctrl+c.
    Ctrl(char),
}

impl Key {
    pub fn modifiers(&self) -> Modifiers {
        match self {
            Key::Ctrl(_) => Modifiers {
                ctrl: true,
                ..Default::default()
            },
            _ => Modifiers::default(),
        }
    }

    /// The key code to hand to `StudioApp::handle_key_down`, if this key maps to
    /// one. Printable characters go through `handle_char_input` instead.
    pub fn as_key_code(&self) -> Option<KeyCode> {
        match self {
            Key::Named(k) => Some(*k),
            Key::Ctrl('c') => Some(KeyCode::Escape),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Unix: termios raw mode + TIOCGWINSZ
// ---------------------------------------------------------------------------
#[cfg(unix)]
mod sys {
    use std::os::raw::{c_int, c_ulong};

    // Layout matches glibc and musl on Linux: four tcflag_t, c_line, NCCS
    // control chars, then the two speed fields. 60 bytes on x86-64/aarch64.
    const NCCS: usize = 32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Termios {
        pub c_iflag: u32,
        pub c_oflag: u32,
        pub c_cflag: u32,
        pub c_lflag: u32,
        pub c_line: u8,
        pub c_cc: [u8; NCCS],
        pub c_ispeed: u32,
        pub c_ospeed: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Winsize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    // Linux termios flag bits (asm-generic/termbits.h).
    pub const IGNBRK: u32 = 0o000001;
    pub const BRKINT: u32 = 0o000002;
    pub const PARMRK: u32 = 0o000010;
    pub const ISTRIP: u32 = 0o000020;
    pub const INLCR: u32 = 0o000100;
    pub const IGNCR: u32 = 0o000200;
    pub const ICRNL: u32 = 0o000400;
    pub const IXON: u32 = 0o002000;
    pub const OPOST: u32 = 0o000001;
    pub const ECHO: u32 = 0o000010;
    pub const ECHONL: u32 = 0o000100;
    pub const ICANON: u32 = 0o000002;
    pub const ISIG: u32 = 0o000001;
    pub const IEXTEN: u32 = 0o100000;
    pub const VMIN: usize = 6;
    pub const VTIME: usize = 5;

    const TIOCGWINSZ: c_ulong = 0x5413;

    extern "C" {
        fn tcgetattr(fd: c_int, termios_p: *mut Termios) -> c_int;
        fn tcsetattr(fd: c_int, optional_actions: c_int, termios_p: *const Termios) -> c_int;
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
        fn isatty(fd: c_int) -> c_int;
    }

    const TCSANOW: c_int = 0;
    const STDIN_FD: c_int = 0;

    /// Reads the window size in (columns, rows). Falls back to 80x24.
    pub fn window_size() -> (usize, usize) {
        unsafe {
            let mut ws = Winsize::default();
            if ioctl(STDIN_FD, TIOCGWINSZ, &mut ws as *mut Winsize) == 0
                && ws.ws_col > 0
                && ws.ws_row > 0
            {
                (ws.ws_col as usize, ws.ws_row as usize)
            } else {
                (80, 24)
            }
        }
    }

    /// True when stdin is a terminal we can talk to.
    pub fn is_interactive() -> bool {
        unsafe { isatty(STDIN_FD) == 1 }
    }

    /// Puts the console into raw mode, returning the previous state to restore.
    pub fn enter_raw() -> Option<Termios> {
        if !is_interactive() {
            return None;
        }
        unsafe {
            let mut original: Termios = std::mem::zeroed();
            if tcgetattr(STDIN_FD, &mut original) != 0 {
                return None;
            }
            let mut raw = original;
            // Input: no break handling, no CR/NL translation, no flow control,
            // no stripping of the 8th bit.
            raw.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
            // Output: we emit our own newlines, so disable post-processing.
            raw.c_oflag &= !OPOST;
            // Local: no echo, no canonical mode, no signals, no extended
            // processing. This is what makes every keystroke arrive instantly.
            raw.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
            raw.c_cc[VMIN] = 1;
            raw.c_cc[VTIME] = 0;
            if tcsetattr(STDIN_FD, TCSANOW, &raw) != 0 {
                return None;
            }
            Some(original)
        }
    }

    pub fn leave_raw(saved: &Termios) {
        unsafe {
            let _ = tcsetattr(STDIN_FD, TCSANOW, saved);
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: console mode
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod sys {
    use std::os::raw::c_void;

    type BOOL = i32;
    type DWORD = u32;
    type HANDLE = *mut c_void;

    const STD_INPUT_HANDLE: DWORD = 0xFFFF_FFF6u32;
    const STD_OUTPUT_HANDLE: DWORD = 0xFFFF_FFF5u32;

    // ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT
    const CONSOLE_DEFAULT: DWORD = 0x0001 | 0x0002 | 0x0004;
    // ENABLE_VIRTUAL_TERMINAL_INPUT
    const ENABLE_VTI: DWORD = 0x0200;
    // ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN
    const OUT_VT: DWORD = 0x0004 | 0x0008;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct ConsoleScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }

    extern "system" {
        fn GetStdHandle(which: DWORD) -> HANDLE;
        fn GetConsoleMode(handle: HANDLE, mode: *mut DWORD) -> BOOL;
        fn SetConsoleMode(handle: HANDLE, mode: DWORD) -> BOOL;
        fn GetConsoleScreenBufferInfo(handle: HANDLE, info: *mut ConsoleScreenBufferInfo) -> BOOL;
    }

    pub fn window_size() -> (usize, usize) {
        unsafe {
            let h = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut info = ConsoleScreenBufferInfo::default();
            if !h.is_null()
                && GetConsoleScreenBufferInfo(h, &mut info) != 0
                && info.window.right > info.window.left
                && info.window.bottom > info.window.top
            {
                let cols = (info.window.right - info.window.left + 1) as usize;
                let rows = (info.window.bottom - info.window.top + 1) as usize;
                (cols.max(20), rows.max(6))
            } else {
                (80, 24)
            }
        }
    }

    pub fn is_interactive() -> bool {
        unsafe {
            let h = GetStdHandle(STD_INPUT_HANDLE);
            if h.is_null() {
                return false;
            }
            let mut mode: DWORD = 0;
            GetConsoleMode(h, &mut mode) != 0
        }
    }

    /// Enables virtual-terminal input so escape sequences arrive as bytes.
    /// Returns the previous input mode to restore, or None on failure.
    pub fn enter_raw() -> Option<DWORD> {
        let hin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        if hin.is_null() {
            return None;
        }
        unsafe {
            let mut saved: DWORD = 0;
            if GetConsoleMode(hin, &mut saved) == 0 {
                return None;
            }
            // Clear the default processing and turn on VT input.
            let mode = (saved & !CONSOLE_DEFAULT) | ENABLE_VTI;
            if SetConsoleMode(hin, mode) == 0 {
                return None;
            }
            let hout = GetStdHandle(STD_OUTPUT_HANDLE);
            if !hout.is_null() {
                let mut omode: DWORD = 0;
                if GetConsoleMode(hout, &mut omode) != 0 {
                    let _ = SetConsoleMode(hout, omode | OUT_VT);
                }
            }
            Some(saved)
        }
    }

    pub fn leave_raw(saved: &DWORD) {
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            if !hin.is_null() {
                let _ = SetConsoleMode(hin, *saved);
            }
        }
    }
}

/// Owns the console: puts it into raw mode, restores it on drop, and knows the
/// current window size.
pub struct Terminal {
    #[cfg(unix)]
    saved: Option<sys::Termios>,
    #[cfg(windows)]
    saved: Option<u32>,
    cols: usize,
    rows: usize,
    /// True when raw mode was actually established.
    raw: bool,
    out: std::io::Stdout,
}

impl Terminal {
    pub fn new() -> Self {
        let (cols, rows) = sys::window_size();
        let (saved, raw) = match sys::enter_raw() {
            Some(s) => (Some(s), true),
            None => (None, false),
        };
        let mut t = Self {
            #[cfg(unix)]
            saved,
            #[cfg(windows)]
            saved,
            cols,
            rows,
            raw,
            out: std::io::stdout(),
        };
        if raw {
            // Alternate screen + hide the cursor + clear. We put them back on
            // drop so a crash never leaves the user's terminal unusable.
            let _ = t.write_raw("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
            let _ = t.flush();
        }
        t
    }

    pub fn raw(&self) -> bool {
        self.raw
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// Re-reads the window size; call on every frame so resizes are picked up.
    pub fn refresh_size(&mut self) {
        let (c, r) = sys::window_size();
        self.cols = c;
        self.rows = r;
    }

    fn write_raw(&mut self, s: &str) -> std::io::Result<()> {
        self.out.write_all(s.as_bytes())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }

    /// Emits a frame built by `render_diff`.
    pub fn present(&mut self, frame: &str) -> std::io::Result<()> {
        self.write_raw(frame)?;
        self.flush()
    }

    /// Shows the cursor and leaves the alternate screen.
    pub fn teardown(&mut self) {
        let _ = self.write_raw("\x1b[?25h\x1b[?1049l");
        let _ = self.flush();
        #[cfg(unix)]
        {
            if let Some(s) = self.saved.take() {
                sys::leave_raw(&s);
            }
        }
        #[cfg(windows)]
        {
            if let Some(s) = self.saved.take() {
                sys::leave_raw(&s);
            }
        }
        self.raw = false;
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Idempotent: teardown() takes the saved state, so a second call is a
        // no-op. Guarantees the terminal is restored even on a panic unwind.
        self.teardown();
    }
}

/// Spawns the reader thread. Bytes arrive on the channel; the main loop decodes
/// them so a slow render never blocks input.
pub fn spawn_input_reader() -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

// ---------------------------------------------------------------------------
// Byte stream -> Key events
// ---------------------------------------------------------------------------

/// Decodes a chunk of terminal input into key events.
///
/// A chunk may split an escape sequence across reads, so an incomplete trailing
/// sequence is returned as `None` and re-fed next time by the caller.
pub struct KeyDecoder {
    pending: Vec<u8>,
}

impl KeyDecoder {
    pub fn new() -> Self {
        Self { pending: Vec::new() }
    }

    /// Feeds bytes and returns whatever complete keys were recognised. Any
    /// bytes that start an incomplete escape sequence are retained.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            let b = self.pending[i];
            match b {
                b'\r' | b'\n' => {
                    out.push(Key::Named(KeyCode::Enter));
                    i += 1;
                }
                0x7f | 0x08 => {
                    out.push(Key::Named(KeyCode::Backspace));
                    i += 1;
                }
                b'\t' => {
                    out.push(Key::Named(KeyCode::Tab));
                    i += 1;
                }
                0x1b => {
                    // Escape, or the start of a CSI / SS3 sequence.
                    match self.decode_escape(i) {
                        Some((key, used)) => {
                            out.push(key);
                            i += used;
                        }
                        None => break, // incomplete — wait for more bytes
                    }
                }
                0x01..=0x1a => {
                    // ctrl+a .. ctrl+z
                    out.push(Key::Ctrl((b'a' + (b - 1)) as char));
                    i += 1;
                }
                _ => {
                    // UTF-8 text run.
                    let (ch, used) = decode_utf8(&self.pending[i..]);
                    match ch {
                        Some(c) => {
                            out.push(Key::Char(c));
                            i += used;
                        }
                        None => break, // partial multi-byte char
                    }
                }
            }
        }
        self.pending.drain(..i);
        out
    }

    /// Releases a lone pending `ESC` as an Escape key press.
    ///
    /// A bare ESC at the end of a read is ambiguous — it may be the start of an
    /// arrow sequence whose remaining bytes have not arrived yet. The main loop
    /// calls this once the input channel has gone quiet, so pressing Escape
    /// always registers instead of waiting forever for a sequel.
    pub fn flush_pending_escape(&mut self) -> Option<Key> {
        if self.pending.as_slice() == [0x1b] {
            self.pending.clear();
            Some(Key::Named(KeyCode::Escape))
        } else {
            None
        }
    }

    /// True when bytes are being held back for a longer sequence.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Decodes an escape sequence starting at `i`. Returns the key and how many
    /// bytes it consumed, or None if the sequence is still incomplete.
    fn decode_escape(&self, i: usize) -> Option<(Key, usize)> {
        let b = self.pending.get(i + 1)?;
        match b {
            b'[' => {
                // CSI: parameters then a final byte in 0x40..=0x7e.
                let mut j = i + 2;
                let mut params = String::new();
                while j < self.pending.len() {
                    let c = self.pending[j];
                    if (0x40..=0x7e).contains(&c) {
                        params.push(c as char);
                        let key = csi_to_key(&params)?;
                        return Some((key, j + 1 - i));
                    }
                    params.push(c as char);
                    j += 1;
                }
                None
            }
            b'O' => {
                // SS3 (some terminals send these for arrows / Home / End).
                let c = *self.pending.get(i + 2)?;
                let key = match c {
                    b'A' => Key::Named(KeyCode::Up),
                    b'B' => Key::Named(KeyCode::Down),
                    b'C' => Key::Named(KeyCode::Right),
                    b'D' => Key::Named(KeyCode::Left),
                    b'H' => Key::Named(KeyCode::Home),
                    b'F' => Key::Named(KeyCode::End),
                    b'P' => Key::Named(KeyCode::Other(0x70)),
                    _ => return None,
                };
                Some((key, 3))
            }
            _ => {
                // A bare ESC, or alt+<char> which we treat as ESC for simplicity.
                Some((Key::Named(KeyCode::Escape), 1))
            }
        }
    }
}

fn csi_to_key(params: &str) -> Option<Key> {
    // Strip a leading modifier parameter, e.g. "1;5A" for ctrl+up.
    let (nums, final_byte) = params.split_at(params.len().saturating_sub(1));
    let shift = nums
        .split(';')
        .nth(1)
        .and_then(|m| m.parse::<u32>().ok())
        .map(|m| m >= 2)
        .unwrap_or(false);
    let _ = shift; // shift+arrows are not distinguished by the app's key model
    match final_byte.as_bytes().first().copied()? {
        b'A' => Some(Key::Named(KeyCode::Up)),
        b'B' => Some(Key::Named(KeyCode::Down)),
        b'C' => Some(Key::Named(KeyCode::Right)),
        b'D' => Some(Key::Named(KeyCode::Left)),
        b'H' => Some(Key::Named(KeyCode::Home)),
        b'F' => Some(Key::Named(KeyCode::End)),
        b'Z' => Some(Key::Named(KeyCode::Tab)),
        b'~' => {
            let n: u32 = nums.parse().ok()?;
            Some(Key::Named(match n {
                1 | 7 => KeyCode::Home,
                2 => KeyCode::Other(2),
                3 => KeyCode::Delete,
                4 | 8 => KeyCode::End,
                5 => KeyCode::PageUp,
                6 => KeyCode::PageDown,
                _ => return None,
            }))
        }
        _ => None,
    }
}

/// Decodes one UTF-8 scalar from the front of `bytes`. Returns None if the
/// buffer holds a partial sequence.
fn decode_utf8(bytes: &[u8]) -> (Option<char>, usize) {
    let first = bytes[0];
    let len = if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else if first >> 3 == 0b11110 {
        4
    } else {
        return (None, 1); // invalid lead byte; skip it
    };
    if bytes.len() < len {
        return (None, 0);
    }
    match std::str::from_utf8(&bytes[..len]) {
        Ok(s) => (s.chars().next(), len),
        Err(_) => (None, len),
    }
}

/// A helper for the view layer: how many columns a character occupies.
pub fn char_width(c: char) -> usize {
    let cp = c as u32;
    // Zero-width: combining marks and joiners.
    if (0x0300..=0x036f).contains(&cp)
        || (0x200b..=0x200f).contains(&cp)
        || cp == 0xfe0f
        || cp == 0xfe0e
    {
        return 0;
    }
    // Wide: CJK, Hangul, fullwidth forms and emoji blocks.
    if (0x1100..=0x115f).contains(&cp)
        || (0x2e80..=0x303e).contains(&cp)
        || (0x3041..=0x33ff).contains(&cp)
        || (0x3400..=0x4dbf).contains(&cp)
        || (0x4e00..=0x9fff).contains(&cp)
        || (0xa000..=0xa4cf).contains(&cp)
        || (0xac00..=0xd7a3).contains(&cp)
        || (0xf900..=0xfaff).contains(&cp)
        || (0xfe30..=0xfe6f).contains(&cp)
        || (0xff00..=0xff60).contains(&cp)
        || (0xffe0..=0xffe6).contains(&cp)
        || (0x1f300..=0x1f64f).contains(&cp)
        || (0x1f900..=0x1f9ff).contains(&cp)
        || (0x20000..=0x3fffd).contains(&cp)
    {
        return 2;
    }
    1
}

/// Poll interval while the agent is streaming, so the transcript keeps moving.
pub const BUSY_TICK: Duration = Duration::from_millis(80);
/// Poll interval when idle, to keep CPU use near zero.
pub const IDLE_TICK: Duration = Duration::from_millis(250);
