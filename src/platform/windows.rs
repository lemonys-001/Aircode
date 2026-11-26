//! Pure-Rust Native Windows x64 platform backend:
//! - Win32 Double-Buffered 32-bit DIBSection + ClearType GDI Studio Surface (Zero WebView)
//! - Genuine Windows ConPTY (CreatePseudoConsole / ResizePseudoConsole / ClosePseudoConsole)
//! - Native WinHTTP TLS 1.2/1.3 HTTPS & HTTP client
//! - Logical Drives, SHBrowseForFolderW native dialog, Clipboard, Explorer reveal, ReadDirectoryChangesW watcher

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use crate::fs_ops::DriveInfo;
use crate::platform::{
    CursorShape, DesktopAppHandler, FontRole, HttpResponse, KeyCode, Modifiers, MouseBtn, PtyEvent,
    SurfaceCanvas, WindowEvent,
};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

type BOOL = i32;
type DWORD = u32;
type UINT = u32;
type WORD = u16;
type SHORT = i16;
type LONG = i32;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type HANDLE = *mut c_void;
type HWND = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type HFONT = *mut c_void;
type HRGN = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HPCON = *mut c_void;
type HINTERNET = *mut c_void;
type HRESULT = i32;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const FALSE: BOOL = 0;
const TRUE: BOOL = 1;

const CS_HREDRAW: UINT = 0x0002;
const CS_VREDRAW: UINT = 0x0001;
const WS_OVERLAPPEDWINDOW: DWORD = 0x00CF0000;
const WS_CLIPCHILDREN: DWORD = 0x02000000;
const CW_USEDEFAULT: i32 = 0x80000000u32 as i32;
const SW_SHOWDEFAULT: i32 = 10;
const SW_SHOWNORMAL: i32 = 1;

const WM_CREATE: UINT = 0x0001;
const WM_DESTROY: UINT = 0x0002;
const WM_SIZE: UINT = 0x0005;
const WM_PAINT: UINT = 0x000F;
const WM_ERASEBKGND: UINT = 0x0014;
const WM_SETCURSOR: UINT = 0x0020;
const WM_GETMINMAXINFO: UINT = 0x0024;
const WM_KEYDOWN: UINT = 0x0100;
const WM_SYSKEYDOWN: UINT = 0x0104;
const WM_CHAR: UINT = 0x0102;
const WM_TIMER: UINT = 0x0113;
const WM_MOUSEMOVE: UINT = 0x0200;
const WM_LBUTTONDOWN: UINT = 0x0201;
const WM_LBUTTONUP: UINT = 0x0202;
const WM_RBUTTONDOWN: UINT = 0x0204;
const WM_RBUTTONUP: UINT = 0x0205;
const WM_MBUTTONDOWN: UINT = 0x0207;
const WM_MBUTTONUP: UINT = 0x0208;
const WM_MOUSEWHEEL: UINT = 0x020A;

const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_ESCAPE: u32 = 0x1B;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_DELETE: u32 = 0x2E;
const VK_F3: u32 = 0x72;
const VK_F5: u32 = 0x74;

const IDC_ARROW_ID: usize = 32512;
// GetSystemMetrics indices for the two icon sizes.
const SM_CXICON: i32 = 11;
const SM_CYICON: i32 = 12;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;
// LoadImageW flags / types.
const IMAGE_ICON: u32 = 1;
const LR_SHARED: u32 = 0x8000;

/// The resource id the pure-Rust `.res` writer registers the app icon under.
const APP_ICON_RES_ID: usize = 1;
const IDC_IBEAM_ID: usize = 32513;
const IDC_SIZEWE_ID: usize = 32644;
const IDC_SIZENS_ID: usize = 32645;
const IDC_HAND_ID: usize = 32649;

const TRANSPARENT: i32 = 1;
const SRCCOPY: DWORD = 0x00CC0020;
const DIB_RGB_COLORS: UINT = 0;
const BI_RGB: DWORD = 0;
const CLEARTYPE_QUALITY: DWORD = 5;
const DEFAULT_CHARSET: DWORD = 1;

const CF_UNICODETEXT: UINT = 13;
const GMEM_MOVEABLE: UINT = 0x0002;

const EXTENDED_STARTUPINFO_PRESENT: DWORD = 0x00080000;
const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x00000400;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x00020016;

const FILE_LIST_DIRECTORY: DWORD = 0x0001;
const FILE_SHARE_ALL: DWORD = 0x0001 | 0x0002 | 0x0004;
const OPEN_EXISTING: DWORD = 3;
const FILE_FLAG_BACKUP_SEMANTICS: DWORD = 0x02000000;
const FILE_NOTIFY_MASK: DWORD = 0x0001 | 0x0002 | 0x0010;

const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: DWORD = 4;
const WINHTTP_FLAG_SECURE: DWORD = 0x00800000;
const WINHTTP_ADDREQ_FLAG_ADD_REPLACE: DWORD = 0x20000000 | 0x80000000;
const WINHTTP_QUERY_STATUS_CODE: DWORD = 19;
const WINHTTP_QUERY_FLAG_NUMBER: DWORD = 0x20000000;
const INTERNET_SCHEME_HTTPS: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct POINT {
    x: LONG,
    y: LONG,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SIZE {
    cx: LONG,
    cy: LONG,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: LONG,
    top: LONG,
    right: LONG,
    bottom: LONG,
}

#[repr(C)]
struct MINMAXINFO {
    ptReserved: POINT,
    ptMaxSize: POINT,
    ptMaxPosition: POINT,
    ptMinTrackSize: POINT,
    ptMaxTrackSize: POINT,
}

#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    fErase: BOOL,
    rcPaint: RECT,
    fRestore: BOOL,
    fIncUpdate: BOOL,
    rgbReserved: [u8; 32],
}

#[repr(C)]
struct WNDCLASSEXW {
    cbSize: UINT,
    style: UINT,
    lpfnWndProc: unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HINSTANCE,
    hIcon: HICON,
    hCursor: HCURSOR,
    hbrBackground: HBRUSH,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
    hIconSm: HICON,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: UINT,
    wParam: WPARAM,
    lParam: LPARAM,
    time: DWORD,
    pt: POINT,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    biSize: DWORD,
    biWidth: LONG,
    biHeight: LONG,
    biPlanes: WORD,
    biBitCount: WORD,
    biCompression: DWORD,
    biSizeImage: DWORD,
    biXPelsPerMeter: LONG,
    biYPelsPerMeter: LONG,
    biClrUsed: DWORD,
    biClrImportant: DWORD,
}

#[repr(C)]
struct BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER,
    bmiColors: [u32; 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct COORD {
    X: SHORT,
    Y: SHORT,
}

#[repr(C)]
struct SECURITY_ATTRIBUTES {
    nLength: DWORD,
    lpSecurityDescriptor: *mut c_void,
    bInheritHandle: BOOL,
}

#[repr(C)]
struct STARTUPINFOW {
    cb: DWORD,
    lpReserved: *mut u16,
    lpDesktop: *mut u16,
    lpTitle: *mut u16,
    dwX: DWORD,
    dwY: DWORD,
    dwXSize: DWORD,
    dwYSize: DWORD,
    dwXCountChars: DWORD,
    dwYCountChars: DWORD,
    dwFillAttribute: DWORD,
    dwFlags: DWORD,
    wShowWindow: WORD,
    cbReserved2: WORD,
    lpReserved2: *mut u8,
    hStdInput: HANDLE,
    hStdOutput: HANDLE,
    hStdError: HANDLE,
}

#[repr(C)]
struct STARTUPINFOEXW {
    StartupInfo: STARTUPINFOW,
    lpAttributeList: *mut c_void,
}

#[repr(C)]
struct PROCESS_INFORMATION {
    hProcess: HANDLE,
    hThread: HANDLE,
    dwProcessId: DWORD,
    dwThreadId: DWORD,
}

#[repr(C)]
struct BROWSEINFOW {
    hwndOwner: HWND,
    pidlRoot: *const c_void,
    pszDisplayName: *mut u16,
    lpszTitle: *const u16,
    ulFlags: UINT,
    lpfn: *const c_void,
    lParam: LPARAM,
    iImage: i32,
}

#[repr(C)]
struct URL_COMPONENTS {
    dwStructSize: DWORD,
    lpszScheme: *mut u16,
    dwSchemeLength: DWORD,
    nScheme: i32,
    lpszHostName: *mut u16,
    dwHostNameLength: DWORD,
    nPort: WORD,
    lpszUserName: *mut u16,
    dwUserNameLength: DWORD,
    lpszPassword: *mut u16,
    dwPasswordLength: DWORD,
    lpszUrlPath: *mut u16,
    dwUrlPathLength: DWORD,
    lpszExtraInfo: *mut u16,
    dwExtraInfoLength: DWORD,
}

extern "system" {
    fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    fn GetSystemMetrics(nIndex: i32) -> i32;
    fn LoadIconW(hInstance: HINSTANCE, lpIconName: *const u16) -> HICON;
    fn LoadImageW(
        hInstance: HINSTANCE,
        name: *const u16,
        image_type: u32,
        desired_width: i32,
        desired_height: i32,
        load_flags: u32,
    ) -> HICON;
    fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
    fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> WORD;
    fn CreateWindowExW(
        dwExStyle: DWORD,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: DWORD,
        X: i32,
        Y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: HWND,
        hMenu: HMENU,
        hInstance: HINSTANCE,
        lpParam: *mut c_void,
    ) -> HWND;
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
    fn UpdateWindow(hWnd: HWND) -> BOOL;
    fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: BOOL) -> BOOL;
    fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> BOOL;
    fn DefWindowProcW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn PostQuitMessage(nExitCode: i32);
    fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: UINT, wMsgFilterMax: UINT) -> BOOL;
    fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
    fn SetTimer(hWnd: HWND, nIDEvent: usize, uElapse: UINT, lpTimerFunc: *const c_void) -> usize;
    fn SetCapture(hWnd: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
    fn GetKeyState(nVirtKey: i32) -> SHORT;

    fn OpenClipboard(hWndNewOwner: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn GetClipboardData(uFormat: UINT) -> HANDLE;
    fn SetClipboardData(uFormat: UINT, hMem: HANDLE) -> HANDLE;
    fn GlobalAlloc(uFlags: UINT, dwBytes: usize) -> HANDLE;
    fn GlobalLock(hMem: HANDLE) -> *mut c_void;
    fn GlobalUnlock(hMem: HANDLE) -> BOOL;

    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(ho: HGDIOBJ) -> BOOL;
    fn CreateDIBSection(
        hdc: HDC,
        pbmi: *const BITMAPINFO,
        usage: UINT,
        ppvBits: *mut *mut c_void,
        hSection: HANDLE,
        offset: DWORD,
    ) -> HBITMAP;
    fn BitBlt(
        hdc: HDC,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: HDC,
        x1: i32,
        y1: i32,
        rop: DWORD,
    ) -> BOOL;
    fn GdiFlush() -> BOOL;
    fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    fn SetTextColor(hdc: HDC, color: DWORD) -> DWORD;
    fn CreateFontW(
        cHeight: i32,
        cWidth: i32,
        cEscapement: i32,
        cOrientation: i32,
        cWeight: i32,
        bItalic: DWORD,
        bUnderline: DWORD,
        bStrikeOut: DWORD,
        iCharSet: DWORD,
        iOutPrecision: DWORD,
        iClipPrecision: DWORD,
        iQuality: DWORD,
        iPitchAndFamily: DWORD,
        pszFaceName: *const u16,
    ) -> HFONT;
    fn ExtTextOutW(
        hdc: HDC,
        x: i32,
        y: i32,
        options: UINT,
        lprect: *const RECT,
        lpString: *const u16,
        c: UINT,
        lpDx: *const i32,
    ) -> BOOL;
    fn GetTextExtentPoint32W(hdc: HDC, lpString: *const u16, c: i32, psizl: *mut SIZE) -> BOOL;
    fn CreateRectRgn(x1: i32, y1: i32, x2: i32, y2: i32) -> HRGN;
    fn SelectClipRgn(hdc: HDC, hrgn: HRGN) -> i32;

    fn DwmSetWindowAttribute(
        hwnd: HWND,
        dwAttribute: DWORD,
        pvAttribute: *const c_void,
        cbAttribute: DWORD,
    ) -> HRESULT;

    fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: DWORD) -> HRESULT;
    fn CoTaskMemFree(pv: *mut c_void);
    fn SHBrowseForFolderW(lpbi: *mut BROWSEINFOW) -> *mut c_void;
    fn SHGetPathFromIDListW(pidl: *const c_void, pszPath: *mut u16) -> BOOL;
    fn ShellExecuteW(
        hwnd: HWND,
        lpOperation: *const u16,
        lpFile: *const u16,
        lpParameters: *const u16,
        lpDirectory: *const u16,
        nShowCmd: i32,
    ) -> isize;

    fn GetLogicalDrives() -> DWORD;
    fn GetDriveTypeW(lpRootPathName: *const u16) -> UINT;
    fn GetDiskFreeSpaceExW(
        lpDirectoryName: *const u16,
        lpFreeBytesAvailableToCaller: *mut u64,
        lpTotalNumberOfBytes: *mut u64,
        lpTotalNumberOfFreeBytes: *mut u64,
    ) -> BOOL;

    fn CreatePipe(
        hReadPipe: *mut HANDLE,
        hWritePipe: *mut HANDLE,
        lpPipeAttributes: *const SECURITY_ATTRIBUTES,
        nSize: DWORD,
    ) -> BOOL;
    fn ReadFile(
        hFile: HANDLE,
        lpBuffer: *mut u8,
        nNumberOfBytesToRead: DWORD,
        lpNumberOfBytesRead: *mut DWORD,
        lpOverlapped: *mut c_void,
    ) -> BOOL;
    fn WriteFile(
        hFile: HANDLE,
        lpBuffer: *const u8,
        nNumberOfBytesToWrite: DWORD,
        lpNumberOfBytesWritten: *mut DWORD,
        lpOverlapped: *mut c_void,
    ) -> BOOL;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
    fn CancelIoEx(hFile: HANDLE, lpOverlapped: *mut c_void) -> BOOL;

    fn CreatePseudoConsole(
        size: COORD,
        hInput: HANDLE,
        hOutput: HANDLE,
        dwFlags: DWORD,
        phPC: *mut HPCON,
    ) -> HRESULT;
    fn ResizePseudoConsole(hPC: HPCON, size: COORD) -> HRESULT;
    fn ClosePseudoConsole(hPC: HPCON);
    fn InitializeProcThreadAttributeList(
        lpAttributeList: *mut c_void,
        dwAttributeCount: DWORD,
        dwFlags: DWORD,
        lpSize: *mut usize,
    ) -> BOOL;
    fn UpdateProcThreadAttribute(
        lpAttributeList: *mut c_void,
        dwFlags: DWORD,
        Attribute: usize,
        lpValue: *mut c_void,
        cbSize: usize,
        lpPreviousValue: *mut c_void,
        lpReturnSize: *mut usize,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(lpAttributeList: *mut c_void);
    fn CreateProcessW(
        lpApplicationName: *const u16,
        lpCommandLine: *mut u16,
        lpProcessAttributes: *const c_void,
        lpThreadAttributes: *const c_void,
        bInheritHandles: BOOL,
        dwCreationFlags: DWORD,
        lpEnvironment: *const c_void,
        lpCurrentDirectory: *const u16,
        lpStartupInfo: *const STARTUPINFOW,
        lpProcessInformation: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn TerminateProcess(hProcess: HANDLE, uExitCode: UINT) -> BOOL;
    fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: DWORD) -> DWORD;
    fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut DWORD) -> BOOL;

    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: *const c_void,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn ReadDirectoryChangesW(
        hDirectory: HANDLE,
        lpBuffer: *mut c_void,
        nBufferLength: DWORD,
        bWatchSubtree: BOOL,
        dwNotifyFilter: DWORD,
        lpBytesReturned: *mut DWORD,
        lpOverlapped: *mut c_void,
        lpCompletionRoutine: *const c_void,
    ) -> BOOL;

    fn WinHttpOpen(
        pszAgentW: *const u16,
        dwAccessType: DWORD,
        pszProxyW: *const u16,
        pszProxyBypassW: *const u16,
        dwFlags: DWORD,
    ) -> HINTERNET;
    fn WinHttpSetTimeouts(
        hInternet: HINTERNET,
        nResolveTimeout: i32,
        nConnectTimeout: i32,
        nSendTimeout: i32,
        nReceiveTimeout: i32,
    ) -> BOOL;
    fn WinHttpCrackUrl(
        pwszUrl: *const u16,
        dwUrlLength: DWORD,
        dwFlags: DWORD,
        lpUrlComponents: *mut URL_COMPONENTS,
    ) -> BOOL;
    fn WinHttpConnect(
        hSession: HINTERNET,
        pswzServerName: *const u16,
        nServerPort: WORD,
        dwReserved: DWORD,
    ) -> HINTERNET;
    fn WinHttpOpenRequest(
        hConnect: HINTERNET,
        pwszVerb: *const u16,
        pwszObjectName: *const u16,
        pwszVersion: *const u16,
        pwszReferrer: *const u16,
        ppwszAcceptTypes: *const *const u16,
        dwFlags: DWORD,
    ) -> HINTERNET;
    fn WinHttpAddRequestHeaders(
        hRequest: HINTERNET,
        lpszHeaders: *const u16,
        dwHeadersLength: DWORD,
        dwModifiers: DWORD,
    ) -> BOOL;
    fn WinHttpSendRequest(
        hRequest: HINTERNET,
        lpszHeaders: *const u16,
        dwHeadersLength: DWORD,
        lpOptional: *const c_void,
        dwOptionalLength: DWORD,
        dwTotalLength: DWORD,
        dwContext: usize,
    ) -> BOOL;
    fn WinHttpReceiveResponse(hRequest: HINTERNET, lpReserved: *mut c_void) -> BOOL;
    fn WinHttpQueryHeaders(
        hRequest: HINTERNET,
        dwInfoLevel: DWORD,
        pwszName: *const u16,
        lpBuffer: *mut c_void,
        lpdwBufferLength: *mut DWORD,
        lpdwIndex: *mut DWORD,
    ) -> BOOL;
    fn WinHttpQueryDataAvailable(hRequest: HINTERNET, lpdwNumberOfBytesAvailable: *mut DWORD) -> BOOL;
    fn WinHttpReadData(
        hRequest: HINTERNET,
        lpBuffer: *mut c_void,
        dwNumberOfBytesToRead: DWORD,
        lpdwNumberOfBytesRead: *mut DWORD,
    ) -> BOOL;
    fn WinHttpCloseHandle(hInternet: HINTERNET) -> BOOL;
    fn GetLastError() -> DWORD;
}

fn wide_null(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

pub fn app_data_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("aircode");
    }
    std::env::temp_dir().join("aircode")
}

pub fn list_drives() -> Vec<DriveInfo> {
    let mut out = Vec::new();
    unsafe {
        let mask = GetLogicalDrives();
        for i in 0..26u32 {
            if (mask & (1 << i)) != 0 {
                let ch = (b'A' + i as u8) as char;
                let mount = format!("{}:\\", ch);
                let w = wide_null(&mount);
                let dtype = match GetDriveTypeW(w.as_ptr()) {
                    2 => "Removable",
                    3 => "Local Disk",
                    4 => "Network",
                    5 => "Optical",
                    _ => "Drive",
                };
                let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
                GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free);
                out.push(DriveInfo {
                    letter: format!("{}:", ch),
                    mount_path: mount,
                    drive_type: dtype.to_string(),
                    total_gb: (total as f64) / 1_073_741_824.0,
                    free_gb: (avail as f64) / 1_073_741_824.0,
                });
            }
        }
    }
    out
}

pub fn pick_folder_dialog(_current: &Path) -> Option<PathBuf> {
    unsafe {
        let mut display = [0u16; 260];
        let title = wide_null("Select Windows Project Folder");
        let mut bi = BROWSEINFOW {
            hwndOwner: ptr::null_mut(),
            pidlRoot: ptr::null(),
            pszDisplayName: display.as_mut_ptr(),
            lpszTitle: title.as_ptr(),
            ulFlags: 0x0001 | 0x0040, // BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE
            lpfn: ptr::null(),
            lParam: 0,
            iImage: 0,
        };
        let pidl = SHBrowseForFolderW(&mut bi);
        if pidl.is_null() {
            return None;
        }
        let mut path_buf = [0u16; 1024];
        let ok = SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr());
        CoTaskMemFree(pidl);
        if ok == FALSE {
            return None;
        }
        let len = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
        if len == 0 {
            None
        } else {
            Some(PathBuf::from(String::from_utf16_lossy(&path_buf[..len])))
        }
    }
}

pub fn reveal_in_file_manager(path: &Path) {
    unsafe {
        let op = wide_null("open");
        let exe = wide_null("explorer.exe");
        let param = wide_null(&format!("/select,\"{}\"", path.display()));
        ShellExecuteW(
            ptr::null_mut(),
            op.as_ptr(),
            exe.as_ptr(),
            param.as_ptr(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn clipboard_get_text() -> Option<String> {
    unsafe {
        if OpenClipboard(ptr::null_mut()) == FALSE {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT);
        if h.is_null() {
            CloseClipboard();
            return None;
        }
        let p = GlobalLock(h) as *const u16;
        let res = if !p.is_null() {
            let mut len = 0usize;
            while *p.add(len) != 0 && len < 2_000_000 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            GlobalUnlock(h);
            Some(s)
        } else {
            None
        };
        CloseClipboard();
        res
    }
}

pub fn clipboard_set_text(text: &str) {
    unsafe {
        if OpenClipboard(ptr::null_mut()) == FALSE {
            return;
        }
        EmptyClipboard();
        let wide = wide_null(text);
        let bytes = wide.len() * 2;
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if !h.is_null() {
            let dst = GlobalLock(h) as *mut u16;
            if !dst.is_null() {
                ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                GlobalUnlock(h);
                SetClipboardData(CF_UNICODETEXT, h);
            }
        }
        CloseClipboard();
    }
}

// ============================================================================
// Native WinHTTP TLS 1.2/1.3 Client
// ============================================================================
pub fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &str,
    timeout_ms: u32,
) -> HttpResponse {
    unsafe {
        let wurl = wide_null(url);
        let mut host_buf = [0u16; 256];
        let mut path_buf = [0u16; 2048];
        let mut uc = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as DWORD,
            lpszScheme: ptr::null_mut(),
            dwSchemeLength: 0,
            nScheme: 0,
            lpszHostName: host_buf.as_mut_ptr(),
            dwHostNameLength: 255,
            nPort: 0,
            lpszUserName: ptr::null_mut(),
            dwUserNameLength: 0,
            lpszPassword: ptr::null_mut(),
            dwPasswordLength: 0,
            lpszUrlPath: path_buf.as_mut_ptr(),
            dwUrlPathLength: 2047,
            lpszExtraInfo: ptr::null_mut(),
            dwExtraInfoLength: 0,
        };

        if WinHttpCrackUrl(wurl.as_ptr(), (wurl.len() - 1) as DWORD, 0, &mut uc) == FALSE {
            return HttpResponse {
                status: 0,
                body: String::new(),
                error: Some(format!("Invalid URL: {}", url)),
            };
        }

        let agent = wide_null("AirCode/2.2");
        let h_sess = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        );
        if h_sess.is_null() {
            return HttpResponse {
                status: 0,
                body: String::new(),
                error: Some("WinHttpOpen failed".to_string()),
            };
        }
        let t = timeout_ms as i32;
        WinHttpSetTimeouts(h_sess, 10_000, 15_000, t, t);

        let h_conn = WinHttpConnect(h_sess, host_buf.as_ptr(), uc.nPort, 0);
        if h_conn.is_null() {
            WinHttpCloseHandle(h_sess);
            return HttpResponse {
                status: 0,
                body: String::new(),
                error: Some("WinHttpConnect failed".to_string()),
            };
        }

        let flags = if uc.nScheme == INTERNET_SCHEME_HTTPS {
            WINHTTP_FLAG_SECURE
        } else {
            0
        };
        let wverb = wide_null(method);
        let h_req = WinHttpOpenRequest(
            h_conn,
            wverb.as_ptr(),
            path_buf.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            flags,
        );
        if h_req.is_null() {
            WinHttpCloseHandle(h_conn);
            WinHttpCloseHandle(h_sess);
            return HttpResponse {
                status: 0,
                body: String::new(),
                error: Some("WinHttpOpenRequest failed".to_string()),
            };
        }

        for (k, v) in headers {
            let line = wide_null(&format!("{}: {}", k, v));
            WinHttpAddRequestHeaders(
                h_req,
                line.as_ptr(),
                (line.len() - 1) as DWORD,
                WINHTTP_ADDREQ_FLAG_ADD_REPLACE,
            );
        }

        let body_bytes = body.as_bytes();
        let sent = WinHttpSendRequest(
            h_req,
            ptr::null(),
            0,
            if body_bytes.is_empty() {
                ptr::null()
            } else {
                body_bytes.as_ptr() as *const c_void
            },
            body_bytes.len() as DWORD,
            body_bytes.len() as DWORD,
            0,
        );

        if sent == FALSE || WinHttpReceiveResponse(h_req, ptr::null_mut()) == FALSE {
            let code = GetLastError();
            WinHttpCloseHandle(h_req);
            WinHttpCloseHandle(h_conn);
            WinHttpCloseHandle(h_sess);
            return HttpResponse {
                status: 0,
                body: String::new(),
                error: Some(format!("WinHTTP request failed (OS error {}) for {}", code, url)),
            };
        }

        let mut status_code: DWORD = 0;
        let mut sz = std::mem::size_of::<DWORD>() as DWORD;
        WinHttpQueryHeaders(
            h_req,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            &mut status_code as *mut _ as *mut c_void,
            &mut sz,
            ptr::null_mut(),
        );

        let mut out_bytes = Vec::new();
        let mut avail: DWORD = 0;
        while WinHttpQueryDataAvailable(h_req, &mut avail) != FALSE && avail > 0 {
            let mut chunk = vec![0u8; avail as usize];
            let mut read_n: DWORD = 0;
            if WinHttpReadData(
                h_req,
                chunk.as_mut_ptr() as *mut c_void,
                avail,
                &mut read_n,
            ) != FALSE
                && read_n > 0
            {
                chunk.truncate(read_n as usize);
                out_bytes.extend_from_slice(&chunk);
            } else {
                break;
            }
        }

        WinHttpCloseHandle(h_req);
        WinHttpCloseHandle(h_conn);
        WinHttpCloseHandle(h_sess);

        HttpResponse {
            status: status_code as u16,
            body: String::from_utf8_lossy(&out_bytes).into_owned(),
            error: None,
        }
    }
}

// ============================================================================
// Genuine Windows ConPTY (HPCON) Session
// ============================================================================
struct ConPtyHandles {
    hpc: HPCON,
    pipe_in_write: HANDLE,
    pipe_out_read: HANDLE,
    h_process: HANDLE,
    h_thread: HANDLE,
}
unsafe impl Send for ConPtyHandles {}

pub struct NativePtySession {
    inner: Arc<Mutex<ConPtyHandles>>,
    alive: Arc<AtomicBool>,
}

impl NativePtySession {
    pub fn spawn(
        shell: &str,
        cwd: &Path,
        cols: u16,
        rows: u16,
        extra_env: &[(String, String)],
    ) -> Result<(Self, Receiver<PtyEvent>), String> {
        unsafe {
            let (mut in_read, mut in_write) = (ptr::null_mut(), ptr::null_mut());
            let (mut out_read, mut out_write) = (ptr::null_mut(), ptr::null_mut());
            if CreatePipe(&mut in_read, &mut in_write, ptr::null(), 0) == FALSE
                || CreatePipe(&mut out_read, &mut out_write, ptr::null(), 0) == FALSE
            {
                return Err("ConPTY CreatePipe failed".to_string());
            }

            let mut hpc: HPCON = ptr::null_mut();
            let size = COORD {
                X: cols.max(30) as SHORT,
                Y: rows.max(8) as SHORT,
            };
            let hr = CreatePseudoConsole(size, in_read, out_write, 0, &mut hpc);
            CloseHandle(in_read);
            CloseHandle(out_write);
            if hr < 0 || hpc.is_null() {
                CloseHandle(in_write);
                CloseHandle(out_read);
                return Err(format!("CreatePseudoConsole failed (HRESULT 0x{:08x})", hr));
            }

            let mut attr_sz: usize = 0;
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_sz);
            let mut attr_buf = vec![0u8; attr_sz];
            let attr_ptr = attr_buf.as_mut_ptr() as *mut c_void;
            if InitializeProcThreadAttributeList(attr_ptr, 1, 0, &mut attr_sz) == FALSE
                || UpdateProcThreadAttribute(
                    attr_ptr,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    hpc,
                    std::mem::size_of::<HPCON>(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                ) == FALSE
            {
                ClosePseudoConsole(hpc);
                CloseHandle(in_write);
                CloseHandle(out_read);
                return Err("UpdateProcThreadAttribute(PSEUDOCONSOLE) failed".to_string());
            }

            let mut env_pairs: Vec<(String, String)> = std::env::vars().collect();
            env_pairs.push(("TERM".to_string(), "xterm-256color".to_string()));
            for (k, v) in extra_env {
                env_pairs.retain(|(ek, _)| !ek.eq_ignore_ascii_case(k));
                env_pairs.push((k.clone(), v.clone()));
            }
            env_pairs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
            let mut env_block: Vec<u16> = Vec::new();
            for (k, v) in env_pairs {
                if !k.is_empty() {
                    env_block.extend(format!("{}={}", k, v).encode_utf16());
                    env_block.push(0);
                }
            }
            env_block.push(0);

            let cmd_str = if shell.eq_ignore_ascii_case("cmd") {
                "cmd.exe"
            } else {
                "powershell.exe -NoLogo"
            };
            let mut cmd_wide = wide_null(cmd_str);
            let cwd_wide = wide_null(&cwd.to_string_lossy());

            let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
            si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
            si_ex.lpAttributeList = attr_ptr;
            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

            let created = CreateProcessW(
                ptr::null(),
                cmd_wide.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                FALSE,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                env_block.as_ptr() as *const c_void,
                cwd_wide.as_ptr(),
                &si_ex.StartupInfo,
                &mut pi,
            );
            DeleteProcThreadAttributeList(attr_ptr);

            if created == FALSE {
                ClosePseudoConsole(hpc);
                CloseHandle(in_write);
                CloseHandle(out_read);
                return Err(format!("Failed spawning {} in ConPTY", cmd_str));
            }

            let handles = Arc::new(Mutex::new(ConPtyHandles {
                hpc,
                pipe_in_write: in_write,
                pipe_out_read: out_read,
                h_process: pi.hProcess,
                h_thread: pi.hThread,
            }));
            let alive = Arc::new(AtomicBool::new(true));
            let (tx, rx) = mpsc::channel();

            let h_clone = handles.clone();
            let alive_clone = alive.clone();
            let read_handle = out_read as usize;
            let proc_handle = pi.hProcess as usize;

            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut n: DWORD = 0;
                while alive_clone.load(Ordering::Relaxed) {
                    let ok = ReadFile(
                        read_handle as HANDLE,
                        buf.as_mut_ptr(),
                        buf.len() as DWORD,
                        &mut n,
                        ptr::null_mut(),
                    );
                    if ok == FALSE || n == 0 {
                        break;
                    }
                    if tx.send(PtyEvent::Output(buf[..n as usize].to_vec())).is_err() {
                        break;
                    }
                }
                let mut code: DWORD = 0;
                WaitForSingleObject(proc_handle as HANDLE, 300);
                GetExitCodeProcess(proc_handle as HANDLE, &mut code);
                alive_clone.store(false, Ordering::SeqCst);
                let _ = tx.send(PtyEvent::Exited(code as i32));
                let _ = h_clone;
            });

            Ok((Self { inner: handles, alive }, rx))
        }
    }

    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Ok(g) = self.inner.lock() {
            if !g.pipe_in_write.is_null() {
                unsafe {
                    let mut written: DWORD = 0;
                    WriteFile(
                        g.pipe_in_write,
                        bytes.as_ptr(),
                        bytes.len() as DWORD,
                        &mut written,
                        ptr::null_mut(),
                    );
                }
            }
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Ok(g) = self.inner.lock() {
            if !g.hpc.is_null() {
                unsafe {
                    ResizePseudoConsole(
                        g.hpc,
                        COORD {
                            X: cols.max(20) as SHORT,
                            Y: rows.max(5) as SHORT,
                        },
                    );
                }
            }
        }
    }

    pub fn kill(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Ok(mut g) = self.inner.lock() {
            unsafe {
                if !g.pipe_in_write.is_null() {
                    CloseHandle(g.pipe_in_write);
                    g.pipe_in_write = ptr::null_mut();
                }
                if !g.hpc.is_null() {
                    ClosePseudoConsole(g.hpc);
                    g.hpc = ptr::null_mut();
                }
                if !g.h_process.is_null() {
                    TerminateProcess(g.h_process, 0);
                    CloseHandle(g.h_process);
                    g.h_process = ptr::null_mut();
                }
                if !g.h_thread.is_null() {
                    CloseHandle(g.h_thread);
                    g.h_thread = ptr::null_mut();
                }
                if !g.pipe_out_read.is_null() {
                    CancelIoEx(g.pipe_out_read, ptr::null_mut());
                    CloseHandle(g.pipe_out_read);
                    g.pipe_out_read = ptr::null_mut();
                }
            }
        }
    }
}

impl Drop for NativePtySession {
    fn drop(&mut self) {
        self.kill();
    }
}

// ============================================================================
// Real-Time Windows ReadDirectoryChangesW Watcher
// ============================================================================
pub struct DirectoryWatcher {
    active: Arc<AtomicBool>,
    handle_val: Arc<Mutex<usize>>,
}

impl DirectoryWatcher {
    pub fn start(root: &Path, changed_flag: Arc<AtomicBool>) -> Self {
        let active = Arc::new(AtomicBool::new(true));
        let handle_val = Arc::new(Mutex::new(0usize));

        unsafe {
            let wpath = wide_null(&root.to_string_lossy());
            let h = CreateFileW(
                wpath.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_ALL,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            );
            if h != INVALID_HANDLE_VALUE && !h.is_null() {
                let h_usize = h as usize;
                *handle_val.lock().unwrap() = h_usize;
                let act = active.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u32; 2048];
                    let mut bytes: DWORD = 0;
                    while act.load(Ordering::Relaxed) {
                        let ok = ReadDirectoryChangesW(
                            h_usize as HANDLE,
                            buf.as_mut_ptr() as *mut c_void,
                            (buf.len() * 4) as DWORD,
                            TRUE,
                            FILE_NOTIFY_MASK,
                            &mut bytes,
                            ptr::null_mut(),
                            ptr::null(),
                        );
                        if ok == FALSE || bytes == 0 || !act.load(Ordering::Relaxed) {
                            break;
                        }
                        changed_flag.store(true, Ordering::SeqCst);
                    }
                });
            }
        }

        Self { active, handle_val }
    }

    pub fn stop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        let mut g = self.handle_val.lock().unwrap();
        if *g != 0 {
            unsafe {
                let h = *g as HANDLE;
                CancelIoEx(h, ptr::null_mut());
                CloseHandle(h);
            }
            *g = 0;
        }
    }
}

impl Drop for DirectoryWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// Double-Buffered 32-bit DIBSection + ClearType GDI Studio Surface
// ============================================================================
struct Win32Backbuffer {
    mem_dc: HDC,
    dib: HBITMAP,
    old_bmp: HGDIOBJ,
    pixels: *mut u32,
    width: i32,
    height: i32,
    font_small: HFONT,
    font_regular: HFONT,
    font_semibold: HFONT,
    font_title: HFONT,
    font_code: HFONT,
    font_term: HFONT,
    code_font_px: i32,
    clip_stack: Vec<(i32, i32, i32, i32)>,
}

impl Win32Backbuffer {
    unsafe fn new() -> Self {
        let mem_dc = CreateCompatibleDC(ptr::null_mut());
        SetBkMode(mem_dc, TRANSPARENT);
        let mut s = Self {
            mem_dc,
            dib: ptr::null_mut(),
            old_bmp: ptr::null_mut(),
            pixels: ptr::null_mut(),
            width: 0,
            height: 0,
            font_small: make_font(-12, 400, "Segoe UI"),
            font_regular: make_font(-14, 400, "Segoe UI"),
            font_semibold: make_font(-14, 600, "Segoe UI Semibold"),
            font_title: make_font(-16, 700, "Segoe UI Semibold"),
            font_code: ptr::null_mut(),
            font_term: make_font(-14, 400, "Cascadia Code"),
            code_font_px: 0,
            clip_stack: Vec::new(),
        };
        s.update_code_font(15);
        s
    }

    unsafe fn update_code_font(&mut self, px: i32) {
        let clamped = px.clamp(11, 24);
        if self.code_font_px == clamped && !self.font_code.is_null() {
            return;
        }
        if !self.font_code.is_null() {
            DeleteObject(self.font_code);
        }
        self.font_code = make_font(-clamped, 400, "Cascadia Code");
        self.code_font_px = clamped;
    }

    unsafe fn ensure_size(&mut self, w: i32, h: i32) {
        let nw = w.max(64);
        let nh = h.max(64);
        if nw == self.width && nh == self.height && !self.dib.is_null() {
            return;
        }
        if !self.dib.is_null() {
            SelectObject(self.mem_dc, self.old_bmp);
            DeleteObject(self.dib);
            self.dib = ptr::null_mut();
            self.pixels = ptr::null_mut();
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as DWORD,
                biWidth: nw,
                biHeight: -nh, // Top-down DIBSection
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [0],
        };
        let mut bits: *mut c_void = ptr::null_mut();
        let hb = CreateDIBSection(
            self.mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            ptr::null_mut(),
            0,
        );
        if !hb.is_null() && !bits.is_null() {
            self.old_bmp = SelectObject(self.mem_dc, hb);
            self.dib = hb;
            self.pixels = bits as *mut u32;
            self.width = nw;
            self.height = nh;
        }
    }

    fn current_clip(&self) -> (i32, i32, i32, i32) {
        self.clip_stack
            .last()
            .copied()
            .unwrap_or((0, 0, self.width, self.height))
    }

    unsafe fn apply_gdi_clip(&self) {
        if let Some(&(x, y, w, h)) = self.clip_stack.last() {
            let rgn = CreateRectRgn(x, y, x + w, y + h);
            SelectClipRgn(self.mem_dc, rgn);
            DeleteObject(rgn);
        } else {
            SelectClipRgn(self.mem_dc, ptr::null_mut());
        }
    }

    unsafe fn select_font(&self, role: FontRole) {
        let f = match role {
            FontRole::UiSmall => self.font_small,
            FontRole::UiRegular => self.font_regular,
            FontRole::UiSemibold => self.font_semibold,
            FontRole::UiTitle => self.font_title,
            FontRole::CodeMono => self.font_code,
            FontRole::TerminalMono => self.font_term,
        };
        SelectObject(self.mem_dc, f);
    }
}

impl Drop for Win32Backbuffer {
    fn drop(&mut self) {
        unsafe {
            if !self.dib.is_null() {
                SelectObject(self.mem_dc, self.old_bmp);
                DeleteObject(self.dib);
            }
            for f in [
                self.font_small,
                self.font_regular,
                self.font_semibold,
                self.font_title,
                self.font_code,
                self.font_term,
            ] {
                if !f.is_null() {
                    DeleteObject(f);
                }
            }
            if !self.mem_dc.is_null() {
                DeleteDC(self.mem_dc);
            }
        }
    }
}

unsafe fn make_font(height: i32, weight: i32, face: &str) -> HFONT {
    let w = wide_null(face);
    CreateFontW(
        height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        0,
        0,
        CLEARTYPE_QUALITY,
        0,
        w.as_ptr(),
    )
}

fn rgb_to_colorref(rgb: u32) -> DWORD {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    (b << 16) | (g << 8) | r
}

impl SurfaceCanvas for Win32Backbuffer {
    fn width(&self) -> i32 {
        self.width
    }

    fn height(&self) -> i32 {
        self.height
    }

    fn clear(&mut self, rgb: u32) {
        if self.pixels.is_null() {
            return;
        }
        unsafe {
            GdiFlush();
            let slice = std::slice::from_raw_parts_mut(
                self.pixels,
                (self.width * self.height) as usize,
            );
            slice.fill(rgb);
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        if self.pixels.is_null() || w <= 0 || h <= 0 {
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
        unsafe {
            GdiFlush();
            let stride = self.width as usize;
            let buf = std::slice::from_raw_parts_mut(self.pixels, stride * self.height as usize);
            for py in y0..y1 {
                let row_off = py as usize * stride;
                buf[(row_off + x0 as usize)..(row_off + x1 as usize)].fill(rgb);
            }
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

        let r2 = r * r;
        for dy in 0..r {
            for dx in 0..r {
                let dist2 = (r - 1 - dx) * (r - 1 - dx) + (r - 1 - dy) * (r - 1 - dy);
                if dist2 <= r2 {
                    self.fill_rect(x + dx, y + dy, 1, 1, rgb);
                    self.fill_rect(x + w - 1 - dx, y + dy, 1, 1, rgb);
                    self.fill_rect(x + dx, y + h - 1 - dy, 1, 1, rgb);
                    self.fill_rect(x + w - 1 - dx, y + h - 1 - dy, 1, 1, rgb);
                }
            }
        }
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

    fn stroke_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, rgb: u32) {
        let r = radius.clamp(0, w.min(h) / 2);
        if r <= 1 {
            self.stroke_rect(x, y, w, h, rgb);
            return;
        }
        self.fill_rect(x + r, y, w - 2 * r, 1, rgb);
        self.fill_rect(x + r, y + h - 1, w - 2 * r, 1, rgb);
        self.fill_rect(x, y + r, 1, h - 2 * r, rgb);
        self.fill_rect(x + w - 1, y + r, 1, h - 2 * r, rgb);
        // Corner pixels
        self.fill_rect(x + 1, y + 1, 1, 1, rgb);
        self.fill_rect(x + w - 2, y + 1, 1, 1, rgb);
        self.fill_rect(x + 1, y + h - 2, 1, 1, rgb);
        self.fill_rect(x + w - 2, y + h - 2, 1, 1, rgb);
    }

    fn draw_hline(&mut self, x: i32, y: i32, w: i32, rgb: u32) {
        self.fill_rect(x, y, w, 1, rgb);
    }

    fn draw_vline(&mut self, x: i32, y: i32, h: i32, rgb: u32) {
        self.fill_rect(x, y, 1, h, rgb);
    }

    fn draw_text(&mut self, x: i32, y: i32, text: &str, font: FontRole, rgb: u32) {
        if text.is_empty() || self.dib.is_null() {
            return;
        }
        unsafe {
            self.select_font(font);
            SetTextColor(self.mem_dc, rgb_to_colorref(rgb));
            let wide: Vec<u16> = text.encode_utf16().collect();
            ExtTextOutW(
                self.mem_dc,
                x,
                y,
                0,
                ptr::null(),
                wide.as_ptr(),
                wide.len() as UINT,
                ptr::null(),
            );
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
        if text.is_empty() || max_w <= 4 || self.dib.is_null() {
            return;
        }
        unsafe {
            self.select_font(font);
            SetTextColor(self.mem_dc, rgb_to_colorref(rgb));
            let wide: Vec<u16> = text.encode_utf16().collect();
            let rc = RECT {
                left: x,
                top: y,
                right: x + max_w,
                bottom: y + 36,
            };
            ExtTextOutW(
                self.mem_dc,
                x,
                y,
                0x0004, // ETO_CLIPPED
                &rc,
                wide.as_ptr(),
                wide.len() as UINT,
                ptr::null(),
            );
        }
    }

    fn measure_text_width(&mut self, text: &str, font: FontRole) -> i32 {
        if text.is_empty() {
            return 0;
        }
        unsafe {
            self.select_font(font);
            let wide: Vec<u16> = text.encode_utf16().collect();
            let mut sz = SIZE::default();
            if GetTextExtentPoint32W(self.mem_dc, wide.as_ptr(), wide.len() as i32, &mut sz)
                != FALSE
            {
                sz.cx
            } else {
                (text.chars().count() * 8) as i32
            }
        }
    }

    fn mono_cell_size(&mut self, font: FontRole) -> (i32, i32) {
        unsafe {
            self.select_font(font);
            let sample = [b'M' as u16];
            let mut sz = SIZE::default();
            if GetTextExtentPoint32W(self.mem_dc, sample.as_ptr(), 1, &mut sz) != FALSE {
                (sz.cx.max(7), (sz.cy + 4).max(16))
            } else {
                (8, 18)
            }
        }
    }

    fn push_clip(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let (px, py, pw, ph) = self.current_clip();
        let nx0 = x.max(px);
        let ny0 = y.max(py);
        let nx1 = (x + w).min(px + pw);
        let ny1 = (y + h).min(py + ph);
        self.clip_stack
            .push((nx0, ny0, (nx1 - nx0).max(0), (ny1 - ny0).max(0)));
        unsafe {
            self.apply_gdi_clip();
        }
    }

    fn pop_clip(&mut self) {
        self.clip_stack.pop();
        unsafe {
            self.apply_gdi_clip();
        }
    }
}

struct WindowState {
    app: Box<dyn DesktopAppHandler>,
    backbuffer: Win32Backbuffer,
    cursor: CursorShape,
}

static mut GLOBAL_WND_STATE: *mut WindowState = ptr::null_mut();

fn current_mods() -> Modifiers {
    unsafe {
        Modifiers {
            ctrl: (GetKeyState(VK_CONTROL) as u16 & 0x8000) != 0,
            shift: (GetKeyState(VK_SHIFT) as u16 & 0x8000) != 0,
            alt: (GetKeyState(VK_MENU) as u16 & 0x8000) != 0,
        }
    }
}

fn translate_vk(vk: u32) -> KeyCode {
    match vk {
        VK_RETURN => KeyCode::Enter,
        VK_ESCAPE => KeyCode::Escape,
        VK_BACK => KeyCode::Backspace,
        VK_DELETE => KeyCode::Delete,
        VK_TAB => KeyCode::Tab,
        VK_LEFT => KeyCode::Left,
        VK_RIGHT => KeyCode::Right,
        VK_UP => KeyCode::Up,
        VK_DOWN => KeyCode::Down,
        VK_HOME => KeyCode::Home,
        VK_END => KeyCode::End,
        VK_PRIOR => KeyCode::PageUp,
        VK_NEXT => KeyCode::PageDown,
        VK_F3 => KeyCode::F3,
        VK_F5 => KeyCode::F5,
        c @ 0x41..=0x5A => KeyCode::Letter((c as u8 as char).to_ascii_lowercase()),
        other => KeyCode::Other(other),
    }
}

unsafe extern "system" fn studio_wnd_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GLOBAL_WND_STATE;
    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let st = &mut *state_ptr;

    match msg {
        WM_CREATE => 0,
        WM_ERASEBKGND => 1,
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 1040;
            mmi.ptMinTrackSize.y = 660;
            0
        }
        WM_SETCURSOR => {
            if (lparam & 0xFFFF) == 1 {
                let id = match st.cursor {
                    CursorShape::Arrow => IDC_ARROW_ID,
                    CursorShape::IBeam => IDC_IBEAM_ID,
                    CursorShape::Hand => IDC_HAND_ID,
                    CursorShape::ResizeH => IDC_SIZEWE_ID,
                    CursorShape::ResizeV => IDC_SIZENS_ID,
                };
                SetCursor(LoadCursorW(ptr::null_mut(), id as *const u16));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SIZE => {
            let w = (lparam & 0xFFFF) as i16 as i32;
            let h = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if w > 0 && h > 0 {
                st.backbuffer.ensure_size(w, h);
                st.app.on_event(WindowEvent::Resize {
                    width: w,
                    height: h,
                });
                InvalidateRect(hwnd, ptr::null(), FALSE);
            }
            0
        }
        WM_TIMER => {
            st.app.on_event(WindowEvent::Tick);
            InvalidateRect(hwnd, ptr::null(), FALSE);
            0
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            SetCapture(hwnd);
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            let button = match msg {
                WM_RBUTTONDOWN => MouseBtn::Right,
                WM_MBUTTONDOWN => MouseBtn::Middle,
                _ => MouseBtn::Left,
            };
            st.cursor = st.app.on_event(WindowEvent::MouseDown {
                x,
                y,
                button,
                mods: current_mods(),
            });
            InvalidateRect(hwnd, ptr::null(), FALSE);
            0
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
            ReleaseCapture();
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            let button = match msg {
                WM_RBUTTONUP => MouseBtn::Right,
                WM_MBUTTONUP => MouseBtn::Middle,
                _ => MouseBtn::Left,
            };
            st.cursor = st.app.on_event(WindowEvent::MouseUp {
                x,
                y,
                button,
                mods: current_mods(),
            });
            InvalidateRect(hwnd, ptr::null(), FALSE);
            0
        }
        WM_MOUSEMOVE => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            let left_down = (wparam & 0x0001) != 0;
            st.cursor = st.app.on_event(WindowEvent::MouseMove { x, y, left_down });
            if left_down {
                InvalidateRect(hwnd, ptr::null(), FALSE);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as i16 as i32;
            let lines = (delta / 40).clamp(-6, 6);
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc);
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            st.app.on_event(WindowEvent::MouseWheel {
                x,
                y,
                delta_lines: if lines == 0 {
                    if delta > 0 { 1 } else { -1 }
                } else {
                    lines
                },
                shift: current_mods().shift,
            });
            InvalidateRect(hwnd, ptr::null(), FALSE);
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let key = translate_vk(wparam as u32);
            st.app.on_event(WindowEvent::KeyDown {
                key,
                mods: current_mods(),
            });
            InvalidateRect(hwnd, ptr::null(), FALSE);
            0
        }
        WM_CHAR => {
            let code = wparam as u32;
            if let Some(ch) = char::from_u32(code) {
                let mods = current_mods();
                if !mods.ctrl && ch >= ' ' && ch != '\x7f' {
                    st.app.on_event(WindowEvent::CharInput(ch));
                    InvalidateRect(hwnd, ptr::null(), FALSE);
                }
            }
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc);
            let w = (rc.right - rc.left).max(64);
            let h = (rc.bottom - rc.top).max(64);
            st.backbuffer.ensure_size(w, h);
            st.backbuffer.update_code_font(st.app.set_code_font_size());
            st.app.render(&mut st.backbuffer);
            GdiFlush();
            BitBlt(
                hdc,
                0,
                0,
                w,
                h,
                st.backbuffer.mem_dc,
                0,
                0,
                SRCCOPY,
            );
            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn run_native_window(title: &str, width: i32, height: i32, app: Box<dyn DesktopAppHandler>) {
    unsafe {
        CoInitializeEx(ptr::null_mut(), 0x2); // COINIT_APARTMENTTHREADED

        let h_inst = GetModuleHandleW(ptr::null());
        // LoadIconW cannot read the PNG-compressed images that .ico files have
        // carried since Vista, so it returns NULL for ours and the window falls
        // back to the blank default Windows icon. LoadImageW handles them, so
        // ask for each size explicitly and keep LoadIconW only as a fallback.
        let res_id = APP_ICON_RES_ID as *const u16;
        let want = |w: i32, h: i32| -> HICON {
            unsafe { LoadImageW(h_inst, res_id, IMAGE_ICON, w, h, LR_SHARED) }
        };
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON)) };
        let (sx, sy) = unsafe {
            (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
        };
        let icon = if cx > 0 && cy > 0 {
            let big = want(cx, cy);
            if big.is_null() { LoadIconW(h_inst, res_id) } else { big }
        } else {
            LoadIconW(h_inst, res_id)
        };
        let icon_sm = if sx > 0 && sy > 0 {
            let small = want(sx, sy);
            if small.is_null() { icon } else { small }
        } else {
            icon
        };
        let class_name = wide_null("AirCodeWnd");

        let mut state = Box::new(WindowState {
            app,
            backbuffer: Win32Backbuffer::new(),
            cursor: CursorShape::Arrow,
        });
        GLOBAL_WND_STATE = &mut *state as *mut WindowState;

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as UINT,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: studio_wnd_proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_inst,
            hIcon: icon,
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW_ID as *const u16),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: icon_sm,
        };
        RegisterClassExW(&wc);

        let wtitle = wide_null(title);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wtitle.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            width,
            height,
            ptr::null_mut(),
            ptr::null_mut(),
            h_inst,
            ptr::null_mut(),
        );

        let dark: BOOL = TRUE;
        DwmSetWindowAttribute(
            hwnd,
            20, // DWMWA_USE_IMMERSIVE_DARK_MODE
            &dark as *const _ as *const c_void,
            std::mem::size_of::<BOOL>() as DWORD,
        );

        ShowWindow(hwnd, SW_SHOWDEFAULT);
        UpdateWindow(hwnd);
        SetTimer(hwnd, 1, 33, ptr::null());

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        GLOBAL_WND_STATE = ptr::null_mut();
    }
}
