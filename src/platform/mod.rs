//! Cross-platform OS abstraction layer (Windows native Win32/ConPTY/WinHTTP/GDI + Linux X11/POSIX).

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(not(target_os = "windows"))]
pub mod linux;

#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(not(target_os = "windows"))]
pub use linux::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontRole {
    UiSmall,
    UiRegular,
    UiSemibold,
    UiTitle,
    CodeMono,
    TerminalMono,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Arrow,
    IBeam,
    Hand,
    ResizeH,
    ResizeV,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseBtn {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F3,
    F5,
    Letter(char),
    Other(u32),
}

#[derive(Clone, Debug)]
pub enum WindowEvent {
    Resize { width: i32, height: i32 },
    MouseDown { x: i32, y: i32, button: MouseBtn, mods: Modifiers },
    MouseUp { x: i32, y: i32, button: MouseBtn, mods: Modifiers },
    MouseMove { x: i32, y: i32, left_down: bool },
    MouseWheel { x: i32, y: i32, delta_lines: i32, shift: bool },
    KeyDown { key: KeyCode, mods: Modifiers },
    CharInput(char),
    Tick,
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Exited(i32),
}

pub trait SurfaceCanvas {
    fn width(&self) -> i32;
    fn height(&self) -> i32;
    fn clear(&mut self, rgb: u32);
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32);
    fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, rgb: u32);
    fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32);
    fn stroke_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, rgb: u32);
    fn draw_hline(&mut self, x: i32, y: i32, w: i32, rgb: u32);
    fn draw_vline(&mut self, x: i32, y: i32, h: i32, rgb: u32);
    fn draw_text(&mut self, x: i32, y: i32, text: &str, font: FontRole, rgb: u32);
    fn draw_text_clipped(
        &mut self,
        x: i32,
        y: i32,
        max_w: i32,
        text: &str,
        font: FontRole,
        rgb: u32,
    );
    fn measure_text_width(&mut self, text: &str, font: FontRole) -> i32;
    fn mono_cell_size(&mut self, font: FontRole) -> (i32, i32);
    fn push_clip(&mut self, x: i32, y: i32, w: i32, h: i32);
    fn pop_clip(&mut self);
}

pub trait DesktopAppHandler {
    fn on_event(&mut self, event: WindowEvent) -> CursorShape;
    fn render(&mut self, canvas: &mut dyn SurfaceCanvas);
    fn set_code_font_size(&self) -> i32;
}
