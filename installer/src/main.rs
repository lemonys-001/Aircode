#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! Pure-Rust Native Windows x64 Setup Wizard & Uninstaller (`aircode-setup.exe`).
//! Embeds `dist/aircode.exe` directly — zero external frameworks, zero WebView, zero UAC prompt required.

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("aircode-setup is the Windows x64 native installer target.");
}

#[cfg(target_os = "windows")]
mod win_setup {
    use std::ffi::c_void;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr;

    static APP_PAYLOAD: &[u8] = include_bytes!("../../dist/aircode.exe");

    type HWND = *mut c_void;
    type HINSTANCE = *mut c_void;
    type HDC = *mut c_void;
    type HFONT = *mut c_void;
    type HBRUSH = *mut c_void;
    type HICON = *mut c_void;
    type HCURSOR = *mut c_void;
    type HGDIOBJ = *mut c_void;
    type HKEY = *mut c_void;
    type LPITEMIDLIST = *mut c_void;
    type BOOL = i32;
    type UINT = u32;
    type DWORD = u32;
    type WORD = u16;
    type LONG = i32;
    type WPARAM = usize;
    type LPARAM = isize;
    type LRESULT = isize;
    type HRESULT = i32;

    const TRUE: BOOL = 1;
    const HKEY_CURRENT_USER: HKEY = 0x80000001usize as HKEY;
    const KEY_WRITE: DWORD = 0x20006;
    const REG_SZ: DWORD = 1;
    const REG_DWORD: DWORD = 4;

    const WM_CREATE: UINT = 0x0001;
    const WM_DESTROY: UINT = 0x0002;
    const WM_PAINT: UINT = 0x000F;
    const WM_COMMAND: UINT = 0x0111;
    const WM_CTLCOLORSTATIC: UINT = 0x0138;
    const WM_CTLCOLORBTN: UINT = 0x0135;
    const WM_SETFONT: UINT = 0x0030;

    const WS_OVERLAPPED: DWORD = 0x00000000;
    const WS_CAPTION: DWORD = 0x00C00000;
    const WS_SYSMENU: DWORD = 0x00080000;
    const WS_MINIMIZEBOX: DWORD = 0x00020000;
    const WS_VISIBLE: DWORD = 0x10000000;
    const WS_CHILD: DWORD = 0x40000000;
    const WS_TABSTOP: DWORD = 0x00010000;
    const WS_BORDER: DWORD = 0x00800000;
    const ES_AUTOHSCROLL: DWORD = 0x0080;
    const BS_AUTOCHECKBOX: DWORD = 0x0003;
    const BS_PUSHBUTTON: DWORD = 0x0000;
    const BS_DEFPUSHBUTTON: DWORD = 0x0001;
    const BM_GETCHECK: UINT = 0x00F0;
    const BM_SETCHECK: UINT = 0x00F1;
    const BST_CHECKED: usize = 1;

    const MB_OK: UINT = 0x00000000;
    const MB_YESNO: UINT = 0x00000004;
    const MB_ICONINFORMATION: UINT = 0x00000040;
    const MB_ICONQUESTION: UINT = 0x00000020;
    const MB_ICONERROR: UINT = 0x00000010;
    const IDYES: i32 = 6;

    const ID_EDIT_DIR: usize = 101;
    const ID_BTN_BROWSE: usize = 102;
    const ID_CHK_DESKTOP: usize = 103;
    const ID_CHK_STARTMENU: usize = 104;
    const ID_CHK_CONTEXT: usize = 105;
    const ID_CHK_LAUNCH: usize = 106;
    const ID_BTN_INSTALL: usize = 107;
    const ID_BTN_CANCEL: usize = 108;
    const ID_LBL_STATUS: usize = 109;

    #[repr(C)]
    struct GUID {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const CLSID_SHELL_LINK: GUID = GUID {
        data1: 0x00021401,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_ISHELL_LINK_W: GUID = GUID {
        data1: 0x000214F9,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_IPERSIST_FILE: GUID = GUID {
        data1: 0x0000010B,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    #[repr(C)]
    struct IShellLinkWVtbl {
        query_interface: unsafe extern "system" fn(*mut IShellLinkW, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut IShellLinkW) -> u32,
        release: unsafe extern "system" fn(*mut IShellLinkW) -> u32,
        get_path: *const c_void,
        get_id_list: *const c_void,
        set_id_list: *const c_void,
        get_description: *const c_void,
        set_description: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> HRESULT,
        get_working_directory: *const c_void,
        set_working_directory: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> HRESULT,
        get_arguments: *const c_void,
        set_arguments: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> HRESULT,
        get_hotkey: *const c_void,
        set_hotkey: *const c_void,
        get_show_cmd: *const c_void,
        set_show_cmd: *const c_void,
        get_icon_location: *const c_void,
        set_icon_location: unsafe extern "system" fn(*mut IShellLinkW, *const u16, i32) -> HRESULT,
        set_relative_path: *const c_void,
        resolve: *const c_void,
        set_path: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> HRESULT,
    }

    #[repr(C)]
    struct IShellLinkW {
        lp_vtbl: *const IShellLinkWVtbl,
    }

    #[repr(C)]
    struct IPersistFileVtbl {
        query_interface: *const c_void,
        add_ref: *const c_void,
        release: unsafe extern "system" fn(*mut IPersistFile) -> u32,
        get_class_id: *const c_void,
        is_dirty: *const c_void,
        load: *const c_void,
        save: unsafe extern "system" fn(*mut IPersistFile, *const u16, BOOL) -> HRESULT,
        save_completed: *const c_void,
        get_cur_file: *const c_void,
    }

    #[repr(C)]
    struct IPersistFile {
        lp_vtbl: *const IPersistFileVtbl,
    }

    #[repr(C)]
    struct RECT {
        left: LONG,
        top: LONG,
        right: LONG,
        bottom: LONG,
    }

    #[repr(C)]
    struct PAINTSTRUCT {
        hdc: HDC,
        f_erase: BOOL,
        rc_paint: RECT,
        f_restore: BOOL,
        f_inc_update: BOOL,
        rgb_reserved: [u8; 32],
    }

    #[repr(C)]
    struct POINT {
        x: LONG,
        y: LONG,
    }

    #[repr(C)]
    struct MSG {
        hwnd: HWND,
        message: UINT,
        w_param: WPARAM,
        l_param: LPARAM,
        time: DWORD,
        pt: POINT,
    }

    #[repr(C)]
    struct WNDCLASSEXW {
        cb_size: UINT,
        style: UINT,
        lpfn_wnd_proc: unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: HINSTANCE,
        h_icon: HICON,
        h_cursor: HCURSOR,
        hbr_background: HBRUSH,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: HICON,
    }

    #[repr(C)]
    struct BROWSEINFOW {
        hwnd_owner: HWND,
        pidl_root: LPITEMIDLIST,
        psz_display_name: *mut u16,
        lpsz_title: *const u16,
        ul_flags: UINT,
        lpfn: *const c_void,
        l_param: LPARAM,
        i_image: i32,
    }

    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
        fn LoadIconW(h_inst: HINSTANCE, name: *const u16) -> HICON;
        fn LoadCursorW(h_inst: HINSTANCE, name: *const u16) -> HCURSOR;
        fn RegisterClassExW(wc: *const WNDCLASSEXW) -> WORD;
        fn CreateWindowExW(
            ex_style: DWORD,
            class_name: *const u16,
            window_name: *const u16,
            style: DWORD,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: HWND,
            menu: *mut c_void,
            inst: HINSTANCE,
            param: *mut c_void,
        ) -> HWND;
        fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
        fn UpdateWindow(hwnd: HWND) -> BOOL;
        fn GetMessageW(msg: *mut MSG, hwnd: HWND, min: UINT, max: UINT) -> BOOL;
        fn TranslateMessage(msg: *const MSG) -> BOOL;
        fn DispatchMessageW(msg: *const MSG) -> LRESULT;
        fn DefWindowProcW(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT;
        fn PostQuitMessage(code: i32);
        fn DestroyWindow(hwnd: HWND) -> BOOL;
        fn SendMessageW(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT;
        fn GetDlgItem(dlg: HWND, id: i32) -> HWND;
        fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
        fn SetWindowTextW(hwnd: HWND, text: *const u16) -> BOOL;
        fn MessageBoxW(hwnd: HWND, text: *const u16, cap: *const u16, typ: UINT) -> i32;
        fn GetSystemMetrics(idx: i32) -> i32;
        fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> HDC;
        fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
        fn FillRect(hdc: HDC, rc: *const RECT, br: HBRUSH) -> i32;

        fn CreateSolidBrush(color: DWORD) -> HBRUSH;
        fn CreateFontW(
            h: i32,
            w: i32,
            esc: i32,
            ori: i32,
            weight: i32,
            italic: DWORD,
            underline: DWORD,
            strike: DWORD,
            charset: DWORD,
            out_prec: DWORD,
            clip_prec: DWORD,
            quality: DWORD,
            pitch: DWORD,
            face: *const u16,
        ) -> HFONT;
        fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
        fn SetBkMode(hdc: HDC, mode: i32) -> i32;
        fn SetBkColor(hdc: HDC, col: DWORD) -> DWORD;
        fn SetTextColor(hdc: HDC, col: DWORD) -> DWORD;
        fn TextOutW(hdc: HDC, x: i32, y: i32, s: *const u16, len: i32) -> BOOL;

        fn DwmSetWindowAttribute(hwnd: HWND, attr: DWORD, val: *const c_void, sz: DWORD) -> HRESULT;

        fn CoInitializeEx(pv: *mut c_void, coinit: DWORD) -> HRESULT;
        fn CoCreateInstance(
            rclsid: *const GUID,
            outer: *mut c_void,
            ctx: DWORD,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> HRESULT;
        fn CoTaskMemFree(pv: *mut c_void);

        fn SHBrowseForFolderW(bi: *mut BROWSEINFOW) -> LPITEMIDLIST;
        fn SHGetPathFromIDListW(pidl: LPITEMIDLIST, path: *mut u16) -> BOOL;

        fn RegCreateKeyExW(
            hkey: HKEY,
            subkey: *const u16,
            reserved: DWORD,
            class: *mut u16,
            options: DWORD,
            sam: DWORD,
            sec: *const c_void,
            phk: *mut HKEY,
            disp: *mut DWORD,
        ) -> LONG;
        fn RegSetValueExW(
            hkey: HKEY,
            name: *const u16,
            reserved: DWORD,
            typ: DWORD,
            data: *const u8,
            cb: DWORD,
        ) -> LONG;
        fn RegCloseKey(hkey: HKEY) -> LONG;
        fn RegDeleteTreeW(hkey: HKEY, subkey: *const u16) -> LONG;
    }

    static mut FONT_UI: HFONT = ptr::null_mut();
    static mut FONT_BOLD: HFONT = ptr::null_mut();
    static mut FONT_TITLE: HFONT = ptr::null_mut();
    static mut BRUSH_BG: HBRUSH = ptr::null_mut();
    static mut BRUSH_HEADER: HBRUSH = ptr::null_mut();

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn rgb_bgr(rgb: u32) -> DWORD {
        let r = (rgb >> 16) & 0xFF;
        let g = (rgb >> 8) & 0xFF;
        let b = rgb & 0xFF;
        (b << 16) | (g << 8) | r
    }

    fn default_install_dir() -> PathBuf {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            PathBuf::from(local).join("Programs").join("aircode")
        } else {
            PathBuf::from(r"C:\aircode")
        }
    }

    fn desktop_shortcut_path() -> Option<PathBuf> {
        std::env::var("USERPROFILE")
            .ok()
            .map(|p| PathBuf::from(p).join("Desktop").join("AirCode.lnk"))
    }

    fn start_menu_shortcut_path() -> Option<PathBuf> {
        std::env::var("APPDATA").ok().map(|p| {
            PathBuf::from(p)
                .join(r"Microsoft\Windows\Start Menu\Programs\AirCode.lnk")
        })
    }

    fn create_shell_shortcut(lnk_path: &Path, target_exe: &Path, work_dir: &Path, desc: &str) {
        if let Some(parent) = lnk_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        unsafe {
            let mut psl: *mut IShellLinkW = ptr::null_mut();
            let hr = CoCreateInstance(
                &CLSID_SHELL_LINK,
                ptr::null_mut(),
                1, // CLSCTX_INPROC_SERVER
                &IID_ISHELL_LINK_W,
                &mut psl as *mut _ as *mut *mut c_void,
            );
            if hr < 0 || psl.is_null() {
                return;
            }
            let vtbl = &*(*psl).lp_vtbl;
            let w_target = wide(&target_exe.to_string_lossy());
            let w_work = wide(&work_dir.to_string_lossy());
            let w_desc = wide(desc);
            (vtbl.set_path)(psl, w_target.as_ptr());
            (vtbl.set_working_directory)(psl, w_work.as_ptr());
            (vtbl.set_description)(psl, w_desc.as_ptr());
            (vtbl.set_icon_location)(psl, w_target.as_ptr(), 0);

            let mut ppf: *mut IPersistFile = ptr::null_mut();
            if (vtbl.query_interface)(
                psl,
                &IID_IPERSIST_FILE,
                &mut ppf as *mut _ as *mut *mut c_void,
            ) >= 0
                && !ppf.is_null()
            {
                let pf_vtbl = &*(*ppf).lp_vtbl;
                let w_lnk = wide(&lnk_path.to_string_lossy());
                (pf_vtbl.save)(ppf, w_lnk.as_ptr(), TRUE);
                (pf_vtbl.release)(ppf);
            }
            (vtbl.release)(psl);
        }
    }

    unsafe fn reg_set_sz(hkey: HKEY, name: Option<&str>, val: &str) {
        let wname = name.map(wide);
        let wval = wide(val);
        RegSetValueExW(
            hkey,
            wname.as_ref().map(|v| v.as_ptr()).unwrap_or(ptr::null()),
            0,
            REG_SZ,
            wval.as_ptr() as *const u8,
            (wval.len() * 2) as DWORD,
        );
    }

    unsafe fn reg_set_dword(hkey: HKEY, name: &str, val: DWORD) {
        let wname = wide(name);
        RegSetValueExW(
            hkey,
            wname.as_ptr(),
            0,
            REG_DWORD,
            &val as *const DWORD as *const u8,
            4,
        );
    }

    fn register_uninstall_and_context_menu(
        install_dir: &Path,
        exe_path: &Path,
        uninst_path: &Path,
        add_context_menu: bool,
    ) {
        unsafe {
            let sub = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\AirCode");
            let mut hk: HKEY = ptr::null_mut();
            let mut disp: DWORD = 0;
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                ptr::null_mut(),
                0,
                KEY_WRITE,
                ptr::null(),
                &mut hk,
                &mut disp,
            ) == 0
            {
                reg_set_sz(hk, Some("DisplayName"), "AirCode");
                // Derived, not written out: a hardcoded string here silently
                // disagrees with the binary the moment the version is bumped.
                reg_set_sz(hk, Some("DisplayVersion"), env!("CARGO_PKG_VERSION"));
                reg_set_sz(hk, Some("Publisher"), "Aircode Desktop");
                reg_set_sz(hk, Some("InstallLocation"), &install_dir.to_string_lossy());
                reg_set_sz(hk, Some("DisplayIcon"), &exe_path.to_string_lossy());
                reg_set_sz(
                    hk,
                    Some("UninstallString"),
                    &format!("\"{}\" --uninstall", uninst_path.display()),
                );
                reg_set_dword(hk, "NoModify", 1);
                reg_set_dword(hk, "NoRepair", 1);
                reg_set_dword(hk, "EstimatedSize", (APP_PAYLOAD.len() / 1024) as DWORD);
                RegCloseKey(hk);
            }

            if add_context_menu {
                for base in [
                    r"Software\Classes\Directory\shell\AirCode",
                    r"Software\Classes\Directory\Background\shell\AirCode",
                ] {
                    let k_sub = wide(base);
                    let mut hk_ctx: HKEY = ptr::null_mut();
                    if RegCreateKeyExW(
                        HKEY_CURRENT_USER,
                        k_sub.as_ptr(),
                        0,
                        ptr::null_mut(),
                        0,
                        KEY_WRITE,
                        ptr::null(),
                        &mut hk_ctx,
                        &mut disp,
                    ) == 0
                    {
                        reg_set_sz(hk_ctx, None, "Open with AirCode");
                        reg_set_sz(hk_ctx, Some("Icon"), &exe_path.to_string_lossy());
                        RegCloseKey(hk_ctx);
                    }

                    let cmd_sub = wide(&format!(r"{}\command", base));
                    let mut hk_cmd: HKEY = ptr::null_mut();
                    if RegCreateKeyExW(
                        HKEY_CURRENT_USER,
                        cmd_sub.as_ptr(),
                        0,
                        ptr::null_mut(),
                        0,
                        KEY_WRITE,
                        ptr::null(),
                        &mut hk_cmd,
                        &mut disp,
                    ) == 0
                    {
                        let arg_tok = if base.contains("Background") { "%V" } else { "%1" };
                        reg_set_sz(
                            hk_cmd,
                            None,
                            &format!("\"{}\" \"{}\"", exe_path.display(), arg_tok),
                        );
                        RegCloseKey(hk_cmd);
                    }
                }
            }
        }
    }

    fn perform_installation(
        install_dir: &Path,
        make_desktop: bool,
        make_start_menu: bool,
        add_context: bool,
        launch_after: bool,
    ) -> Result<PathBuf, String> {
        fs::create_dir_all(install_dir)
            .map_err(|e| format!("Failed creating {}: {}", install_dir.display(), e))?;

        let exe_path = install_dir.join("aircode.exe");
        fs::write(&exe_path, APP_PAYLOAD).map_err(|e| {
            format!(
                "Failed writing {} (close any running aircode.exe first): {}",
                exe_path.display(),
                e
            )
        })?;

        let uninst_path = install_dir.join("Uninstall.exe");
        if let Ok(self_exe) = std::env::current_exe() {
            let _ = fs::copy(&self_exe, &uninst_path);
        }

        if make_desktop {
            if let Some(lnk) = desktop_shortcut_path() {
                create_shell_shortcut(
                    &lnk,
                    &exe_path,
                    install_dir,
                    "AirCode",
                );
            }
        }

        if make_start_menu {
            if let Some(lnk) = start_menu_shortcut_path() {
                create_shell_shortcut(
                    &lnk,
                    &exe_path,
                    install_dir,
                    "AirCode",
                );
            }
        }

        register_uninstall_and_context_menu(install_dir, &exe_path, &uninst_path, add_context);

        if launch_after {
            let _ = Command::new(&exe_path).current_dir(install_dir).spawn();
        }

        Ok(exe_path)
    }

    fn perform_uninstall(silent: bool) {
        unsafe {
            if !silent {
                let ans = MessageBoxW(
                    ptr::null_mut(),
                    wide("Uninstall AirCode from your computer?\n\n(Your local SQLite sessions in %LOCALAPPDATA%\\aircode will be preserved.)").as_ptr(),
                    wide("AirCode Uninstaller").as_ptr(),
                    MB_YESNO | MB_ICONQUESTION,
                );
                if ans != IDYES {
                    return;
                }
            }

            if let Some(d) = desktop_shortcut_path() {
                let _ = fs::remove_file(d);
            }
            if let Some(s) = start_menu_shortcut_path() {
                let _ = fs::remove_file(s);
            }

            for key in [
                r"Software\Microsoft\Windows\CurrentVersion\Uninstall\AirCode",
                r"Software\Classes\Directory\shell\AirCode",
                r"Software\Classes\Directory\Background\shell\AirCode",
            ] {
                RegDeleteTreeW(HKEY_CURRENT_USER, wide(key).as_ptr());
            }

            if let Ok(self_exe) = std::env::current_exe() {
                if let Some(dir) = self_exe.parent() {
                    let app_exe = dir.join("aircode.exe");
                    let _ = fs::remove_file(&app_exe);
                    // Schedule self-cleanup via cmd ping delay
                    let _ = Command::new("cmd.exe")
                        .args([
                            "/C",
                            &format!(
                                "ping 127.0.0.1 -n 2 > nul & del /f /q \"{}\" & rmdir \"{}\"",
                                self_exe.display(),
                                dir.display()
                            ),
                        ])
                        .spawn();
                }
            }

            if !silent {
                MessageBoxW(
                    ptr::null_mut(),
                    wide("AirCode has been uninstalled cleanly.").as_ptr(),
                    wide("Uninstall Complete").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
    }

    unsafe extern "system" fn setup_wnd_proc(
        hwnd: HWND,
        msg: UINT,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CREATE => {
                let inst = GetModuleHandleW(ptr::null());
                let def_dir = default_install_dir().to_string_lossy().to_string();

                let edit = CreateWindowExW(
                    0,
                    wide("EDIT").as_ptr(),
                    wide(&def_dir).as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                    28,
                    132,
                    410,
                    28,
                    hwnd,
                    ID_EDIT_DIR as *mut c_void,
                    inst,
                    ptr::null_mut(),
                );
                SendMessageW(edit, WM_SETFONT, FONT_UI as WPARAM, 1);

                let btn_browse = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide("Browse...").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                    448,
                    132,
                    96,
                    28,
                    hwnd,
                    ID_BTN_BROWSE as *mut c_void,
                    inst,
                    ptr::null_mut(),
                );
                SendMessageW(btn_browse, WM_SETFONT, FONT_UI as WPARAM, 1);

                let checks = [
                    (ID_CHK_DESKTOP, "Create Desktop shortcut", 180),
                    (ID_CHK_STARTMENU, "Create Start Menu shortcut", 210),
                    (
                        ID_CHK_CONTEXT,
                        "Add \"Open with AirCode\" to Windows Explorer folders",
                        240,
                    ),
                    (
                        ID_CHK_LAUNCH,
                        "Launch AirCode immediately after installation",
                        270,
                    ),
                ];
                for (cid, label, y) in checks {
                    let chk = CreateWindowExW(
                        0,
                        wide("BUTTON").as_ptr(),
                        wide(label).as_ptr(),
                        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
                        28,
                        y,
                        516,
                        24,
                        hwnd,
                        cid as *mut c_void,
                        inst,
                        ptr::null_mut(),
                    );
                    SendMessageW(chk, WM_SETFONT, FONT_UI as WPARAM, 1);
                    SendMessageW(chk, BM_SETCHECK, BST_CHECKED, 0);
                }

                let lbl_status = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Ready to install 100% Pure-Rust Native Studio (no admin rights required).")
                        .as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    28,
                    312,
                    516,
                    22,
                    hwnd,
                    ID_LBL_STATUS as *mut c_void,
                    inst,
                    ptr::null_mut(),
                );
                SendMessageW(lbl_status, WM_SETFONT, FONT_UI as WPARAM, 1);

                let btn_install = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide("Install Now").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                    316,
                    352,
                    120,
                    34,
                    hwnd,
                    ID_BTN_INSTALL as *mut c_void,
                    inst,
                    ptr::null_mut(),
                );
                SendMessageW(btn_install, WM_SETFONT, FONT_BOLD as WPARAM, 1);

                let btn_cancel = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide("Cancel").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                    446,
                    352,
                    98,
                    34,
                    hwnd,
                    ID_BTN_CANCEL as *mut c_void,
                    inst,
                    ptr::null_mut(),
                );
                SendMessageW(btn_cancel, WM_SETFONT, FONT_UI as WPARAM, 1);
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let header_rc = RECT {
                    left: 0,
                    top: 0,
                    right: 580,
                    bottom: 88,
                };
                FillRect(hdc, &header_rc, BRUSH_HEADER);
                let body_rc = RECT {
                    left: 0,
                    top: 88,
                    right: 580,
                    bottom: 430,
                };
                FillRect(hdc, &body_rc, BRUSH_BG);

                SetBkMode(hdc, 1);
                SelectObject(hdc, FONT_TITLE as HGDIOBJ);
                SetTextColor(hdc, rgb_bgr(0xF8FAFC));
                let t1 = wide(&format!("AirCode v{} — Setup Wizard", env!("CARGO_PKG_VERSION")));
                TextOutW(hdc, 28, 18, t1.as_ptr(), (t1.len() - 1) as i32);

                SelectObject(hdc, FONT_UI as HGDIOBJ);
                SetTextColor(hdc, rgb_bgr(0x38BDF8));
                let sub = format!(
                    "100% Pure-Rust Native Windows x64 GUI Studio  ({:.2} MB self-contained)",
                    APP_PAYLOAD.len() as f64 / (1024.0 * 1024.0)
                );
                let wsub = wide(&sub);
                TextOutW(hdc, 28, 52, wsub.as_ptr(), (wsub.len() - 1) as i32);

                SelectObject(hdc, FONT_BOLD as HGDIOBJ);
                SetTextColor(hdc, rgb_bgr(0xCBD5E1));
                let lbl = wide("Destination Folder:");
                TextOutW(hdc, 28, 106, lbl.as_ptr(), (lbl.len() - 1) as i32);

                EndPaint(hwnd, &ps);
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                let hdc = wp as HDC;
                SetTextColor(hdc, rgb_bgr(0xE2E8F0));
                SetBkColor(hdc, rgb_bgr(0x0F172A));
                BRUSH_BG as LRESULT
            }
            WM_COMMAND => {
                let id = wp & 0xFFFF;
                match id {
                    ID_BTN_CANCEL => {
                        DestroyWindow(hwnd);
                    }
                    ID_BTN_BROWSE => {
                        let mut disp_buf = [0u16; 260];
                        let title = wide("Choose Installation Directory");
                        let mut bi = BROWSEINFOW {
                            hwnd_owner: hwnd,
                            pidl_root: ptr::null_mut(),
                            psz_display_name: disp_buf.as_mut_ptr(),
                            lpsz_title: title.as_ptr(),
                            ul_flags: 0x00000001 | 0x00000040,
                            lpfn: ptr::null(),
                            l_param: 0,
                            i_image: 0,
                        };
                        let pidl = SHBrowseForFolderW(&mut bi);
                        if !pidl.is_null() {
                            let mut path_buf = [0u16; 520];
                            if SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr()) != 0 {
                                let len = path_buf
                                    .iter()
                                    .position(|&c| c == 0)
                                    .unwrap_or(path_buf.len());
                                let chosen = PathBuf::from(String::from_utf16_lossy(&path_buf[..len]))
                                    .join("aircode");
                                let edit = GetDlgItem(hwnd, ID_EDIT_DIR as i32);
                                SetWindowTextW(edit, wide(&chosen.to_string_lossy()).as_ptr());
                            }
                            CoTaskMemFree(pidl);
                        }
                    }
                    ID_BTN_INSTALL => {
                        let edit = GetDlgItem(hwnd, ID_EDIT_DIR as i32);
                        let mut buf = [0u16; 520];
                        let n = GetWindowTextW(edit, buf.as_mut_ptr(), 518).max(0) as usize;
                        let dir_str = String::from_utf16_lossy(&buf[..n]).trim().to_string();
                        let target_dir = if dir_str.is_empty() {
                            default_install_dir()
                        } else {
                            PathBuf::from(dir_str)
                        };

                        let chk = |cid: usize| -> bool {
                            let h = GetDlgItem(hwnd, cid as i32);
                            SendMessageW(h, BM_GETCHECK, 0, 0) as usize == BST_CHECKED
                        };

                        let lbl = GetDlgItem(hwnd, ID_LBL_STATUS as i32);
                        SetWindowTextW(lbl, wide("Installing aircode.exe...").as_ptr());

                        match perform_installation(
                            &target_dir,
                            chk(ID_CHK_DESKTOP),
                            chk(ID_CHK_STARTMENU),
                            chk(ID_CHK_CONTEXT),
                            chk(ID_CHK_LAUNCH),
                        ) {
                            Ok(exe) => {
                                MessageBoxW(
                                    hwnd,
                                    wide(&format!(
                                        "AirCode was installed successfully!\n\nLocation:\n{}",
                                        exe.display()
                                    ))
                                    .as_ptr(),
                                    wide("Installation Complete").as_ptr(),
                                    MB_OK | MB_ICONINFORMATION,
                                );
                                DestroyWindow(hwnd);
                            }
                            Err(err) => {
                                SetWindowTextW(lbl, wide(&err).as_ptr());
                                MessageBoxW(
                                    hwnd,
                                    wide(&err).as_ptr(),
                                    wide("Installation Failed").as_ptr(),
                                    MB_OK | MB_ICONERROR,
                                );
                            }
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub fn entry() {
        unsafe {
            CoInitializeEx(ptr::null_mut(), 0x2);
        }

        let args: Vec<String> = std::env::args().skip(1).collect();
        let is_silent = args
            .iter()
            .any(|a| a.eq_ignore_ascii_case("/S") || a.eq_ignore_ascii_case("--silent"));
        let is_uninstall = args
            .iter()
            .any(|a| a.eq_ignore_ascii_case("--uninstall") || a.eq_ignore_ascii_case("/U"));

        if is_uninstall {
            perform_uninstall(is_silent);
            return;
        }

        if is_silent {
            let dir = default_install_dir();
            if let Err(e) = perform_installation(&dir, true, true, true, false) {
                eprintln!("{}", e);
                std::process::exit(1);
            }
            return;
        }

        unsafe {
            FONT_UI = CreateFontW(
                -15,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                wide("Segoe UI").as_ptr(),
            );
            FONT_BOLD = CreateFontW(
                -15,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                wide("Segoe UI").as_ptr(),
            );
            FONT_TITLE = CreateFontW(
                -22,
                0,
                0,
                0,
                700,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                wide("Segoe UI").as_ptr(),
            );
            BRUSH_BG = CreateSolidBrush(rgb_bgr(0x0F172A));
            BRUSH_HEADER = CreateSolidBrush(rgb_bgr(0x060B14));

            let inst = GetModuleHandleW(ptr::null());
            let icon = LoadIconW(inst, 1usize as *const u16);
            let cls = wide("AirCodeRustSetupWnd");

            let wc = WNDCLASSEXW {
                cb_size: std::mem::size_of::<WNDCLASSEXW>() as UINT,
                style: 3,
                lpfn_wnd_proc: setup_wnd_proc,
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: inst,
                h_icon: icon,
                h_cursor: LoadCursorW(ptr::null_mut(), 32512usize as *const u16),
                hbr_background: BRUSH_BG,
                lpsz_menu_name: ptr::null(),
                lpsz_class_name: cls.as_ptr(),
                h_icon_sm: icon,
            };
            RegisterClassExW(&wc);

            let win_w = 580;
            let win_h = 440;
            let sx = (GetSystemMetrics(0) - win_w) / 2;
            let sy = (GetSystemMetrics(1) - win_h) / 2;

            let hwnd = CreateWindowExW(
                0,
                cls.as_ptr(),
                wide("AirCode Setup (Pure-Rust Windows x64)").as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                sx.max(40),
                sy.max(40),
                win_w,
                win_h,
                ptr::null_mut(),
                ptr::null_mut(),
                inst,
                ptr::null_mut(),
            );

            let dark: BOOL = TRUE;
            DwmSetWindowAttribute(hwnd, 20, &dark as *const _ as *const c_void, 4);

            ShowWindow(hwnd, 5);
            UpdateWindow(hwnd);

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    win_setup::entry();
}
