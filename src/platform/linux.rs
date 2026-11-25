//! Pure-Rust Linux X11 + POSIX platform backend (enabling cross-platform Linux builds & tests).

use crate::fs_ops::DriveInfo;
use crate::platform::{
    DesktopAppHandler, FontRole, HttpResponse, KeyCode, Modifiers, MouseBtn, PtyEvent,
    SurfaceCanvas, WindowEvent,
};
use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

static CLIPBOARD_MEM: Mutex<String> = Mutex::new(String::new());

pub fn app_data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("ai-coding-agent");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/ai-coding-agent");
    }
    std::env::temp_dir().join("ai-coding-agent")
}

pub fn list_drives() -> Vec<DriveInfo> {
    let mut out = vec![DriveInfo {
        letter: "/".to_string(),
        mount_path: "/".to_string(),
        drive_type: "Root FS".to_string(),
        total_gb: 0.0,
        free_gb: 0.0,
    }];
    if let Ok(home) = std::env::var("HOME") {
        out.push(DriveInfo {
            letter: "~".to_string(),
            mount_path: home,
            drive_type: "Home".to_string(),
            total_gb: 0.0,
            free_gb: 0.0,
        });
    }
    out
}

pub fn pick_folder_dialog(current: &Path) -> Option<PathBuf> {
    if let Ok(out) = Command::new("zenity")
        .args(["--file-selection", "--directory"])
        .current_dir(current)
        .output()
    {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
    }
    None
}

pub fn reveal_in_file_manager(path: &Path) {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let _ = Command::new("xdg-open").arg(dir).spawn();
}

pub fn clipboard_get_text() -> Option<String> {
    if let Ok(out) = Command::new("xclip").args(["-selection", "clipboard", "-o"]).output() {
        if out.status.success() {
            return Some(String::from_utf8_lossy(&out.stdout).into_owned());
        }
    }
    let s = CLIPBOARD_MEM.lock().unwrap().clone();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn clipboard_set_text(text: &str) {
    *CLIPBOARD_MEM.lock().unwrap() = text.to_string();
    if let Ok(mut child) = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &str,
    timeout_ms: u32,
) -> HttpResponse {
    let secs = (timeout_ms / 1000).max(3).to_string();
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "-X",
        method,
        "--max-time",
        &secs,
        "-w",
        "\n__HTTP_STATUS__:%{http_code}",
    ]);
    for (k, v) in headers {
        cmd.arg("-H").arg(format!("{}: {}", k, v));
    }
    if !body.is_empty() {
        cmd.arg("--data-binary").arg(body);
    }
    cmd.arg(url);

    match cmd.output() {
        Ok(out) => {
            let full = String::from_utf8_lossy(&out.stdout).into_owned();
            if let Some((payload, status_str)) = full.rsplit_once("\n__HTTP_STATUS__:") {
                let code = status_str.trim().parse::<u16>().unwrap_or(0);
                if code == 0 && !out.status.success() {
                    HttpResponse {
                        status: 0,
                        body: String::new(),
                        error: Some(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                    }
                } else {
                    HttpResponse {
                        status: code,
                        body: payload.to_string(),
                        error: None,
                    }
                }
            } else {
                HttpResponse {
                    status: 0,
                    body: full,
                    error: Some(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                }
            }
        }
        Err(e) => HttpResponse {
            status: 0,
            body: String::new(),
            error: Some(format!("curl failed: {}", e)),
        },
    }
}

pub struct NativePtySession {
    stdin: Arc<Mutex<Option<std::process::ChildStdin>>>,
    child: Arc<Mutex<Option<std::process::Child>>>,
}

impl NativePtySession {
    pub fn spawn(
        shell: &str,
        cwd: &Path,
        _cols: u16,
        _rows: u16,
        extra_env: &[(String, String)],
    ) -> Result<(Self, Receiver<PtyEvent>), String> {
        let exe = if shell == "cmd" || shell == "powershell" {
            "/bin/bash"
        } else {
            shell
        };
        let mut cmd = Command::new(exe);
        cmd.current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("TERM", "xterm-256color");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| format!("Failed spawning shell: {}", e))?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or("No stdout")?;
        let mut stderr = child.stderr.take().ok_or("No stderr")?;

        let (tx, rx) = mpsc::channel();
        let tx_err = tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let _ = tx_err.send(PtyEvent::Output(buf[..n].to_vec()));
            }
        });

        let child_arc = Arc::new(Mutex::new(Some(child)));
        let child_wait = child_arc.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            let code = child_wait
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|c| c.wait().ok())
                .and_then(|s| s.code())
                .unwrap_or(0);
            let _ = tx.send(PtyEvent::Exited(code));
        });

        Ok((
            Self {
                stdin: Arc::new(Mutex::new(stdin)),
                child: child_arc,
            },
            rx,
        ))
    }

    pub fn write(&self, bytes: &[u8]) {
        if let Some(stdin) = self.stdin.lock().unwrap().as_mut() {
            let _ = stdin.write_all(bytes);
            let _ = stdin.flush();
        }
    }

    pub fn resize(&self, _cols: u16, _rows: u16) {}

    pub fn kill(&mut self) {
        *self.stdin.lock().unwrap() = None;
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
        }
    }
}

impl Drop for NativePtySession {
    fn drop(&mut self) {
        self.kill();
    }
}

pub struct DirectoryWatcher {
    active: Arc<AtomicBool>,
}

impl DirectoryWatcher {
    pub fn start(_root: &Path, _changed_flag: Arc<AtomicBool>) -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn stop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

// ============================================================================
// Pure-Rust Software Surface + X11 Native Window (for Linux Desktop)
// ============================================================================
pub struct SoftwareSurface {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
    clip_stack: Vec<(i32, i32, i32, i32)>,
}

impl SoftwareSurface {
    pub fn new(w: i32, h: i32) -> Self {
        let nw = w.max(64);
        let nh = h.max(64);
        Self {
            width: nw,
            height: nh,
            pixels: vec![0; (nw * nh) as usize],
            clip_stack: Vec::new(),
        }
    }

    pub fn ensure_size(&mut self, w: i32, h: i32) {
        let nw = w.max(64);
        let nh = h.max(64);
        if nw != self.width || nh != self.height {
            self.width = nw;
            self.height = nh;
            self.pixels.resize((nw * nh) as usize, 0);
        }
    }

    fn current_clip(&self) -> (i32, i32, i32, i32) {
        self.clip_stack
            .last()
            .copied()
            .unwrap_or((0, 0, self.width, self.height))
    }
}

impl SurfaceCanvas for SoftwareSurface {
    fn width(&self) -> i32 {
        self.width
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn clear(&mut self, rgb: u32) {
        self.pixels.fill(rgb);
    }
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (cx, cy, cw, ch) = self.current_clip();
        let x0 = x.max(cx).max(0);
        let y0 = y.max(cy).max(0);
        let x1 = (x + w).min(cx + cw).min(self.width);
        let y1 = (y + h).min(cy + ch).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let stride = self.width as usize;
        for py in y0..y1 {
            let off = py as usize * stride;
            self.pixels[(off + x0 as usize)..(off + x1 as usize)].fill(rgb);
        }
    }
    fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, rgb: u32) {
        let r = radius.clamp(0, w.min(h) / 2);
        if r <= 1 {
            self.fill_rect(x, y, w, h, rgb);
            return;
        }
        self.fill_rect(x + r, y, w - 2 * r, h, rgb);
        self.fill_rect(x, y + r, r, h - 2 * r, rgb);
        self.fill_rect(x + w - r, y + r, r, h - 2 * r, rgb);
    }
    fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, rgb);
        self.fill_rect(x, y + h - 1, w, 1, rgb);
        self.fill_rect(x, y, 1, h, rgb);
        self.fill_rect(x + w - 1, y, 1, h, rgb);
    }
    fn stroke_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, _r: i32, rgb: u32) {
        self.stroke_rect(x, y, w, h, rgb);
    }
    fn draw_hline(&mut self, x: i32, y: i32, w: i32, rgb: u32) {
        self.fill_rect(x, y, w, 1, rgb);
    }
    fn draw_vline(&mut self, x: i32, y: i32, h: i32, rgb: u32) {
        self.fill_rect(x, y, 1, h, rgb);
    }
    fn draw_text(&mut self, x: i32, y: i32, text: &str, _font: FontRole, rgb: u32) {
        let mut cx = x;
        for ch in text.chars() {
            draw_glyph_8x14(self, cx, y + 2, ch, rgb);
            cx += 8;
        }
    }
    fn draw_text_clipped(
        &mut self,
        x: i32,
        y: i32,
        max_w: i32,
        text: &str,
        font: FontRole,
        rgb: u32,
    ) {
        let max_chars = (max_w / 8).max(0) as usize;
        let clipped: String = text.chars().take(max_chars).collect();
        self.draw_text(x, y, &clipped, font, rgb);
    }
    fn measure_text_width(&mut self, text: &str, _font: FontRole) -> i32 {
        (text.chars().count() * 8) as i32
    }
    fn mono_cell_size(&mut self, _font: FontRole) -> (i32, i32) {
        (8, 18)
    }
    fn push_clip(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let (px, py, pw, ph) = self.current_clip();
        let nx0 = x.max(px);
        let ny0 = y.max(py);
        let nx1 = (x + w).min(px + pw);
        let ny1 = (y + h).min(py + ph);
        self.clip_stack
            .push((nx0, ny0, (nx1 - nx0).max(0), (ny1 - ny0).max(0)));
    }
    fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }
}

// Standard IBM/VGA 8x8 bitmap glyphs for ASCII 0x20..=0x7E (MSB = leftmost pixel or bit0 = left; here bit0 = left pixel)
const FONT8X8_BASIC: [[u8; 8]; 95] = [
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // ' '
    [0x18,0x3C,0x3C,0x18,0x18,0x00,0x18,0x00], // '!'
    [0x36,0x36,0x00,0x00,0x00,0x00,0x00,0x00], // '"'
    [0x36,0x36,0x7F,0x36,0x7F,0x36,0x36,0x00], // '#'
    [0x0C,0x3E,0x03,0x1E,0x30,0x1F,0x0C,0x00], // '$'
    [0x00,0x63,0x33,0x18,0x0C,0x66,0x63,0x00], // '%'
    [0x1C,0x36,0x1C,0x6E,0x3B,0x33,0x6E,0x00], // '&'
    [0x06,0x06,0x03,0x00,0x00,0x00,0x00,0x00], // '\''
    [0x18,0x0C,0x06,0x06,0x06,0x0C,0x18,0x00], // '('
    [0x06,0x0C,0x18,0x18,0x18,0x0C,0x06,0x00], // ')'
    [0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00], // '*'
    [0x00,0x0C,0x0C,0x3F,0x0C,0x0C,0x00,0x00], // '+'
    [0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x06], // ','
    [0x00,0x00,0x00,0x3F,0x00,0x00,0x00,0x00], // '-'
    [0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x00], // '.'
    [0x60,0x30,0x18,0x0C,0x06,0x03,0x01,0x00], // '/'
    [0x3E,0x63,0x73,0x7B,0x6F,0x67,0x3E,0x00], // '0'
    [0x0C,0x0E,0x0C,0x0C,0x0C,0x0C,0x3F,0x00], // '1'
    [0x1E,0x33,0x30,0x1C,0x06,0x33,0x3F,0x00], // '2'
    [0x1E,0x33,0x30,0x1C,0x30,0x33,0x1E,0x00], // '3'
    [0x38,0x3C,0x36,0x33,0x7F,0x30,0x78,0x00], // '4'
    [0x3F,0x03,0x1F,0x30,0x30,0x33,0x1E,0x00], // '5'
    [0x1C,0x06,0x03,0x1F,0x33,0x33,0x1E,0x00], // '6'
    [0x3F,0x33,0x30,0x18,0x0C,0x0C,0x0C,0x00], // '7'
    [0x1E,0x33,0x33,0x1E,0x33,0x33,0x1E,0x00], // '8'
    [0x1E,0x33,0x33,0x3E,0x30,0x18,0x0E,0x00], // '9'
    [0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x00], // ':'
    [0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x06], // ';'
    [0x18,0x0C,0x06,0x03,0x06,0x0C,0x18,0x00], // '<'
    [0x00,0x00,0x3F,0x00,0x00,0x3F,0x00,0x00], // '='
    [0x06,0x0C,0x18,0x30,0x18,0x0C,0x06,0x00], // '>'
    [0x1E,0x33,0x30,0x18,0x0C,0x00,0x0C,0x00], // '?'
    [0x3E,0x63,0x7B,0x7B,0x7B,0x03,0x1E,0x00], // '@'
    [0x0C,0x1E,0x33,0x33,0x3F,0x33,0x33,0x00], // 'A'
    [0x3F,0x66,0x66,0x3E,0x66,0x66,0x3F,0x00], // 'B'
    [0x3C,0x66,0x03,0x03,0x03,0x66,0x3C,0x00], // 'C'
    [0x1F,0x36,0x66,0x66,0x66,0x36,0x1F,0x00], // 'D'
    [0x7F,0x46,0x16,0x1E,0x16,0x46,0x7F,0x00], // 'E'
    [0x7F,0x46,0x16,0x1E,0x16,0x06,0x0F,0x00], // 'F'
    [0x3C,0x66,0x03,0x03,0x73,0x66,0x7C,0x00], // 'G'
    [0x33,0x33,0x33,0x3F,0x33,0x33,0x33,0x00], // 'H'
    [0x1E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00], // 'I'
    [0x78,0x30,0x30,0x30,0x33,0x33,0x1E,0x00], // 'J'
    [0x67,0x66,0x36,0x1E,0x36,0x66,0x67,0x00], // 'K'
    [0x0F,0x06,0x06,0x06,0x46,0x66,0x7F,0x00], // 'L'
    [0x63,0x77,0x7F,0x7F,0x6B,0x63,0x63,0x00], // 'M'
    [0x63,0x67,0x6F,0x7B,0x73,0x63,0x63,0x00], // 'N'
    [0x1C,0x36,0x63,0x63,0x63,0x36,0x1C,0x00], // 'O'
    [0x3F,0x66,0x66,0x3E,0x06,0x06,0x0F,0x00], // 'P'
    [0x1E,0x33,0x33,0x33,0x3B,0x1E,0x38,0x00], // 'Q'
    [0x3F,0x66,0x66,0x3E,0x36,0x66,0x67,0x00], // 'R'
    [0x1E,0x33,0x07,0x0E,0x38,0x33,0x1E,0x00], // 'S'
    [0x3F,0x2D,0x0C,0x0C,0x0C,0x0C,0x1E,0x00], // 'T'
    [0x33,0x33,0x33,0x33,0x33,0x33,0x3F,0x00], // 'U'
    [0x33,0x33,0x33,0x33,0x33,0x1E,0x0C,0x00], // 'V'
    [0x63,0x63,0x63,0x6B,0x7F,0x77,0x63,0x00], // 'W'
    [0x63,0x63,0x36,0x1C,0x1C,0x36,0x63,0x00], // 'X'
    [0x33,0x33,0x33,0x1E,0x0C,0x0C,0x1E,0x00], // 'Y'
    [0x7F,0x63,0x31,0x18,0x4C,0x66,0x7F,0x00], // 'Z'
    [0x1E,0x06,0x06,0x06,0x06,0x06,0x1E,0x00], // '['
    [0x03,0x06,0x0C,0x18,0x30,0x60,0x40,0x00], // '\\'
    [0x1E,0x18,0x18,0x18,0x18,0x18,0x1E,0x00], // ']'
    [0x08,0x1C,0x36,0x63,0x00,0x00,0x00,0x00], // '^'
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFF], // '_'
    [0x0C,0x0C,0x18,0x00,0x00,0x00,0x00,0x00], // '`'
    [0x00,0x00,0x1E,0x30,0x3E,0x33,0x6E,0x00], // 'a'
    [0x07,0x06,0x06,0x3E,0x66,0x66,0x3B,0x00], // 'b'
    [0x00,0x00,0x1E,0x33,0x03,0x33,0x1E,0x00], // 'c'
    [0x38,0x30,0x30,0x3E,0x33,0x33,0x6E,0x00], // 'd'
    [0x00,0x00,0x1E,0x33,0x3F,0x03,0x1E,0x00], // 'e'
    [0x1C,0x36,0x06,0x0F,0x06,0x06,0x0F,0x00], // 'f'
    [0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x1F], // 'g'
    [0x07,0x06,0x36,0x6E,0x66,0x66,0x67,0x00], // 'h'
    [0x0C,0x00,0x0E,0x0C,0x0C,0x0C,0x1E,0x00], // 'i'
    [0x30,0x00,0x30,0x30,0x30,0x33,0x33,0x1E], // 'j'
    [0x07,0x06,0x66,0x36,0x1E,0x36,0x67,0x00], // 'k'
    [0x0E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00], // 'l'
    [0x00,0x00,0x33,0x7F,0x7F,0x6B,0x63,0x00], // 'm'
    [0x00,0x00,0x1F,0x33,0x33,0x33,0x33,0x00], // 'n'
    [0x00,0x00,0x1E,0x33,0x33,0x33,0x1E,0x00], // 'o'
    [0x00,0x00,0x3B,0x66,0x66,0x3E,0x06,0x0F], // 'p'
    [0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x78], // 'q'
    [0x00,0x00,0x3B,0x6E,0x66,0x06,0x0F,0x00], // 'r'
    [0x00,0x00,0x3E,0x03,0x1E,0x30,0x1F,0x00], // 's'
    [0x08,0x0C,0x3E,0x0C,0x0C,0x2C,0x18,0x00], // 't'
    [0x00,0x00,0x33,0x33,0x33,0x33,0x6E,0x00], // 'u'
    [0x00,0x00,0x33,0x33,0x33,0x1E,0x0C,0x00], // 'v'
    [0x00,0x00,0x63,0x6B,0x7F,0x7F,0x36,0x00], // 'w'
    [0x00,0x00,0x63,0x36,0x1C,0x36,0x63,0x00], // 'x'
    [0x00,0x00,0x33,0x33,0x33,0x3E,0x30,0x1F], // 'y'
    [0x00,0x00,0x3F,0x19,0x0C,0x26,0x3F,0x00], // 'z'
    [0x38,0x0C,0x0C,0x07,0x0C,0x0C,0x38,0x00], // '{'
    [0x18,0x18,0x18,0x00,0x18,0x18,0x18,0x00], // '|'
    [0x07,0x0C,0x0C,0x38,0x0C,0x0C,0x07,0x00], // '}'
    [0x6E,0x3B,0x00,0x00,0x00,0x00,0x00,0x00], // '~'
];

fn draw_glyph_8x14(surf: &mut SoftwareSurface, x: i32, y: i32, ch: char, rgb: u32) {
    if ch == ' ' {
        return;
    }
    let ascii = match ch {
        '—' | '–' | '─' => '-',
        '·' | '•' => '.',
        '▸' | '›' | '→' => '>',
        '▾' => 'v',
        '✓' => '+',
        c if (' '..='~').contains(&c) => c,
        _ => '?',
    };
    let glyph = &FONT8X8_BASIC[(ascii as usize) - 0x20];
    for (row, &bits) in glyph.iter().enumerate() {
        if bits == 0 {
            continue;
        }
        // Vertically center inside 14px line box
        let py = y + 2 + row as i32;
        for col in 0..8 {
            if (bits >> col) & 1 != 0 {
                surf.fill_rect(x + col, py, 1, 1, rgb);
            }
        }
    }
}

type Display = c_void;
type Window = c_ulong;
type GC = *mut c_void;
type KeySym = c_ulong;
type Atom = c_ulong;

#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut c_char,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    obdata: *mut c_char,
    _f: [*mut c_void; 6],
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XKeyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: u32,
    keycode: u32,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XButtonEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: u32,
    button: u32,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XMotionEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: u32,
    is_hint: c_char,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XConfigureEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    event: Window,
    window: Window,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
}

#[repr(C)]
union XEvent {
    type_: c_int,
    xkey: XKeyEvent,
    xbutton: XButtonEvent,
    xmotion: XMotionEvent,
    xconfigure: XConfigureEvent,
    pad: [c_long; 24],
}

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    #[link_name = "free"]
    fn libc_free(p: *mut c_void);
}

unsafe fn load_sym<T: Copy>(lib: *mut c_void, name: &[u8]) -> Option<T> {
    let raw = dlsym(lib, name.as_ptr() as *const c_char);
    if raw.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy(&raw))
    }
}

fn translate_keysym(ks: KeySym) -> Option<KeyCode> {
    match ks & 0xFFFF {
        0xFF0D | 0xFF8D => Some(KeyCode::Enter),
        0xFF1B => Some(KeyCode::Escape),
        0xFF08 => Some(KeyCode::Backspace),
        0xFFFF => Some(KeyCode::Delete),
        0xFF09 | 0xFE20 => Some(KeyCode::Tab),
        0xFF51 => Some(KeyCode::Left),
        0xFF52 => Some(KeyCode::Up),
        0xFF53 => Some(KeyCode::Right),
        0xFF54 => Some(KeyCode::Down),
        0xFF50 => Some(KeyCode::Home),
        0xFF57 => Some(KeyCode::End),
        0xFF55 => Some(KeyCode::PageUp),
        0xFF56 => Some(KeyCode::PageDown),
        0xFFBE => Some(KeyCode::Other(0x70)), // F1
        0xFFC0 => Some(KeyCode::F3),
        0xFFC2 => Some(KeyCode::F5),
        0x002F => Some(KeyCode::Other(0xBF)), // '/' -> matches VK_OEM_2 for Ctrl+/
        c if (0x0041..=0x005A).contains(&c) => Some(KeyCode::Letter((c as u8) as char)),
        c if (0x0061..=0x007A).contains(&c) => {
            Some(KeyCode::Letter(((c as u8) - 32) as char))
        }
        _ => None,
    }
}

pub fn run_native_window(title: &str, width: i32, height: i32, mut app: Box<dyn DesktopAppHandler>) {
    type FnOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut Display;
    type FnDefaultScreen = unsafe extern "C" fn(*mut Display) -> c_int;
    type FnRootWindow = unsafe extern "C" fn(*mut Display, c_int) -> Window;
    type FnDefaultVisual = unsafe extern "C" fn(*mut Display, c_int) -> *mut c_void;
    type FnDefaultDepth = unsafe extern "C" fn(*mut Display, c_int) -> c_int;
    type FnDefaultGC = unsafe extern "C" fn(*mut Display, c_int) -> GC;
    type FnCreateSimpleWindow = unsafe extern "C" fn(
        *mut Display,
        Window,
        c_int,
        c_int,
        u32,
        u32,
        u32,
        c_ulong,
        c_ulong,
    ) -> Window;
    type FnStoreName = unsafe extern "C" fn(*mut Display, Window, *const c_char) -> c_int;
    type FnSelectInput = unsafe extern "C" fn(*mut Display, Window, c_long) -> c_int;
    type FnMapWindow = unsafe extern "C" fn(*mut Display, Window) -> c_int;
    type FnPending = unsafe extern "C" fn(*mut Display) -> c_int;
    type FnNextEvent = unsafe extern "C" fn(*mut Display, *mut XEvent) -> c_int;
    type FnLookupString = unsafe extern "C" fn(
        *mut XKeyEvent,
        *mut c_char,
        c_int,
        *mut KeySym,
        *mut c_void,
    ) -> c_int;
    type FnInternAtom = unsafe extern "C" fn(*mut Display, *const c_char, c_int) -> Atom;
    type FnSetWMProtocols = unsafe extern "C" fn(*mut Display, Window, *mut Atom, c_int) -> c_int;
    type FnCreateImage = unsafe extern "C" fn(
        *mut Display,
        *mut c_void,
        u32,
        c_int,
        c_int,
        *mut c_char,
        u32,
        u32,
        c_int,
        c_int,
    ) -> *mut XImage;
    type FnPutImage = unsafe extern "C" fn(
        *mut Display,
        Window,
        GC,
        *mut XImage,
        c_int,
        c_int,
        c_int,
        c_int,
        u32,
        u32,
    ) -> c_int;
    type FnFlush = unsafe extern "C" fn(*mut Display) -> c_int;

    unsafe {
        let lib = dlopen(b"libX11.so.6\0".as_ptr() as *const c_char, 2); // RTLD_NOW
        if lib.is_null() {
            eprintln!("AirCode: libX11.so.6 not found (run inside a Linux desktop session).");
            return;
        }
        let (
            Some(x_open_display),
            Some(x_default_screen),
            Some(x_root_window),
            Some(x_default_visual),
            Some(x_default_depth),
            Some(x_default_gc),
            Some(x_create_simple_window),
            Some(x_store_name),
            Some(x_select_input),
            Some(x_map_window),
            Some(x_pending),
            Some(x_next_event),
            Some(x_lookup_string),
            Some(x_create_image),
            Some(x_put_image),
            Some(x_flush),
        ) = (
            load_sym::<FnOpenDisplay>(lib, b"XOpenDisplay\0"),
            load_sym::<FnDefaultScreen>(lib, b"XDefaultScreen\0"),
            load_sym::<FnRootWindow>(lib, b"XRootWindow\0"),
            load_sym::<FnDefaultVisual>(lib, b"XDefaultVisual\0"),
            load_sym::<FnDefaultDepth>(lib, b"XDefaultDepth\0"),
            load_sym::<FnDefaultGC>(lib, b"XDefaultGC\0"),
            load_sym::<FnCreateSimpleWindow>(lib, b"XCreateSimpleWindow\0"),
            load_sym::<FnStoreName>(lib, b"XStoreName\0"),
            load_sym::<FnSelectInput>(lib, b"XSelectInput\0"),
            load_sym::<FnMapWindow>(lib, b"XMapWindow\0"),
            load_sym::<FnPending>(lib, b"XPending\0"),
            load_sym::<FnNextEvent>(lib, b"XNextEvent\0"),
            load_sym::<FnLookupString>(lib, b"XLookupString\0"),
            load_sym::<FnCreateImage>(lib, b"XCreateImage\0"),
            load_sym::<FnPutImage>(lib, b"XPutImage\0"),
            load_sym::<FnFlush>(lib, b"XFlush\0"),
        ) else {
            eprintln!("AirCode: failed resolving X11 symbols.");
            return;
        };

        let dpy = x_open_display(ptr::null());
        if dpy.is_null() {
            eprintln!("AirCode: X11 display not available (run inside a Linux desktop session).");
            return;
        }
        let scr = x_default_screen(dpy);
        let root = x_root_window(dpy, scr);
        let win = x_create_simple_window(
            dpy,
            root,
            40,
            40,
            width as u32,
            height as u32,
            0,
            0x16161E,
            0x16161E,
        );
        if let Ok(ctitle) = CString::new(title) {
            x_store_name(dpy, win, ctitle.as_ptr());
        }

        if let (Some(x_intern), Some(x_set_wm)) = (
            load_sym::<FnInternAtom>(lib, b"XInternAtom\0"),
            load_sym::<FnSetWMProtocols>(lib, b"XSetWMProtocols\0"),
        ) {
            let mut wm_del = x_intern(dpy, b"WM_DELETE_WINDOW\0".as_ptr() as *const c_char, 0);
            if wm_del != 0 {
                x_set_wm(dpy, win, &mut wm_del, 1);
            }
        }

        // KeyPressMask | ButtonPressMask | ButtonReleaseMask | PointerMotionMask | ExposureMask | StructureNotifyMask
        let mask: c_long = (1 << 0) | (1 << 2) | (1 << 3) | (1 << 6) | (1 << 15) | (1 << 17);
        x_select_input(dpy, win, mask);
        x_map_window(dpy, win);

        let mut surf = SoftwareSurface::new(width, height);
        let visual = x_default_visual(dpy, scr);
        let depth = x_default_depth(dpy, scr) as u32;
        let gc = x_default_gc(dpy, scr);
        let mut left_down = false;

        loop {
            while x_pending(dpy) > 0 {
                let mut ev = XEvent { type_: 0 };
                x_next_event(dpy, &mut ev);
                match ev.type_ {
                    2 => {
                        // KeyPress
                        let mut kev = ev.xkey;
                        let mods = Modifiers {
                            shift: (kev.state & 1) != 0,
                            ctrl: (kev.state & 4) != 0,
                            alt: (kev.state & 8) != 0,
                        };
                        let mut buf = [0 as c_char; 16];
                        let mut ks: KeySym = 0;
                        let n = x_lookup_string(&mut kev, buf.as_mut_ptr(), 15, &mut ks, ptr::null_mut());
                        if let Some(kc) = translate_keysym(ks) {
                            app.on_event(WindowEvent::KeyDown { key: kc, mods });
                        }
                        if !mods.ctrl && !mods.alt && n > 0 {
                            let bytes: Vec<u8> = buf[..(n as usize)]
                                .iter()
                                .map(|&c| c as u8)
                                .collect();
                            if let Ok(s) = std::str::from_utf8(&bytes) {
                                for ch in s.chars() {
                                    if ch >= ' ' && ch != '\x7f' {
                                        app.on_event(WindowEvent::CharInput(ch));
                                    }
                                }
                            }
                        }
                    }
                    4 => {
                        // ButtonPress
                        let bev = ev.xbutton;
                        let mods = Modifiers {
                            shift: (bev.state & 1) != 0,
                            ctrl: (bev.state & 4) != 0,
                            alt: (bev.state & 8) != 0,
                        };
                        match bev.button {
                            1 => {
                                left_down = true;
                                app.on_event(WindowEvent::MouseDown {
                                    x: bev.x,
                                    y: bev.y,
                                    button: MouseBtn::Left,
                                    mods,
                                });
                            }
                            2 => {
                                app.on_event(WindowEvent::MouseDown {
                                    x: bev.x,
                                    y: bev.y,
                                    button: MouseBtn::Middle,
                                    mods,
                                });
                            }
                            3 => {
                                app.on_event(WindowEvent::MouseDown {
                                    x: bev.x,
                                    y: bev.y,
                                    button: MouseBtn::Right,
                                    mods,
                                });
                            }
                            4 => {
                                app.on_event(WindowEvent::MouseWheel {
                                    x: bev.x,
                                    y: bev.y,
                                    delta_lines: 3,
                                    shift: mods.shift,
                                });
                            }
                            5 => {
                                app.on_event(WindowEvent::MouseWheel {
                                    x: bev.x,
                                    y: bev.y,
                                    delta_lines: -3,
                                    shift: mods.shift,
                                });
                            }
                            _ => {}
                        }
                    }
                    5 => {
                        // ButtonRelease
                        let bev = ev.xbutton;
                        let mods = Modifiers {
                            shift: (bev.state & 1) != 0,
                            ctrl: (bev.state & 4) != 0,
                            alt: (bev.state & 8) != 0,
                        };
                        if bev.button == 1 {
                            left_down = false;
                            app.on_event(WindowEvent::MouseUp {
                                x: bev.x,
                                y: bev.y,
                                button: MouseBtn::Left,
                                mods,
                            });
                        } else if bev.button == 3 {
                            app.on_event(WindowEvent::MouseUp {
                                x: bev.x,
                                y: bev.y,
                                button: MouseBtn::Right,
                                mods,
                            });
                        }
                    }
                    6 => {
                        let mev = ev.xmotion;
                        app.on_event(WindowEvent::MouseMove {
                            x: mev.x,
                            y: mev.y,
                            left_down,
                        });
                    }
                    22 => {
                        let cev = ev.xconfigure;
                        if cev.width > 64 && cev.height > 64 {
                            surf.ensure_size(cev.width, cev.height);
                            app.on_event(WindowEvent::Resize {
                                width: cev.width,
                                height: cev.height,
                            });
                        }
                    }
                    33 => {
                        // ClientMessage (WM_DELETE_WINDOW)
                        return;
                    }
                    _ => {}
                }
            }

            app.on_event(WindowEvent::Tick);
            app.render(&mut surf);

            let ximg = x_create_image(
                dpy,
                visual,
                depth,
                2,
                0,
                surf.pixels.as_mut_ptr() as *mut c_char,
                surf.width as u32,
                surf.height as u32,
                32,
                surf.width * 4,
            );
            if !ximg.is_null() {
                x_put_image(
                    dpy,
                    win,
                    gc,
                    ximg,
                    0,
                    0,
                    0,
                    0,
                    surf.width as u32,
                    surf.height as u32,
                );
                (*ximg).data = ptr::null_mut();
                libc_free(ximg as *mut c_void);
            }
            x_flush(dpy);
            std::thread::sleep(Duration::from_millis(16));
        }
    }
}
