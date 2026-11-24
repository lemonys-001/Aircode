#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif
#define NOMINMAX
#define WIN32_LEAN_AND_MEAN

#include <windows.h>
#include <commctrl.h>
#include <dwmapi.h>
#include <objbase.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shobjidl.h>

#include <cstdint>
#include <filesystem>
#include <fstream>
#include <string>

namespace fs = std::filesystem;

extern "C" {
    extern const uint8_t g_app_exe_start[];
    extern const uint8_t g_app_exe_end[];
    extern const uint8_t g_app_ico_start[];
    extern const uint8_t g_app_ico_end[];
}

__asm__(
    ".section .rdata,\"dr\"\n"
    ".global g_app_exe_start\n"
    "g_app_exe_start:\n"
    ".incbin \"dist/AI-Coding-Agent.exe\"\n"
    ".global g_app_exe_end\n"
    "g_app_exe_end:\n"
    ".byte 0\n"
    ".global g_app_ico_start\n"
    "g_app_ico_start:\n"
    ".incbin \"src-tauri/icons/icon.ico\"\n"
    ".global g_app_ico_end\n"
    "g_app_ico_end:\n"
    ".byte 0\n"
    ".text\n"
);

static constexpr int ID_EDIT_DIR      = 101;
static constexpr int ID_BTN_BROWSE    = 102;
static constexpr int ID_CHK_DESKTOP   = 103;
static constexpr int ID_CHK_STARTMENU = 104;
static constexpr int ID_CHK_CTXMENU   = 105;
static constexpr int ID_CHK_LAUNCH    = 106;
static constexpr int ID_BTN_INSTALL   = 107;
static constexpr int ID_BTN_CANCEL    = 108;
static constexpr int ID_PROGRESS      = 109;
static constexpr int ID_LBL_STATUS    = 110;

static HWND g_edit_dir = nullptr;
static HWND g_chk_desktop = nullptr;
static HWND g_chk_startmenu = nullptr;
static HWND g_chk_ctxmenu = nullptr;
static HWND g_chk_launch = nullptr;
static HWND g_btn_install = nullptr;
static HWND g_progress = nullptr;
static HWND g_lbl_status = nullptr;
static HFONT g_font_title = nullptr;
static HFONT g_font_ui = nullptr;

static fs::path get_known_folder(REFKNOWNFOLDERID rfid) {
    PWSTR raw = nullptr;
    if (SUCCEEDED(SHGetKnownFolderPath(rfid, 0, nullptr, &raw)) && raw) {
        fs::path p(raw);
        CoTaskMemFree(raw);
        return p;
    }
    return {};
}

static bool create_windows_shortcut(const fs::path& lnk_path,
                                    const fs::path& target_exe,
                                    const fs::path& icon_path,
                                    const wchar_t* desc) {
    IShellLinkW* psl = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&psl));
    if (FAILED(hr) || !psl) return false;

    psl->SetPath(target_exe.c_str());
    psl->SetWorkingDirectory(target_exe.parent_path().c_str());
    psl->SetIconLocation(icon_path.c_str(), 0);
    psl->SetDescription(desc);

    IPersistFile* ppf = nullptr;
    bool ok = false;
    if (SUCCEEDED(psl->QueryInterface(IID_PPV_ARGS(&ppf))) && ppf) {
        std::error_code ec;
        fs::create_directories(lnk_path.parent_path(), ec);
        ok = SUCCEEDED(ppf->Save(lnk_path.c_str(), TRUE));
        ppf->Release();
    }
    psl->Release();
    return ok;
}

static void write_reg_sz(HKEY root, const wchar_t* subkey, const wchar_t* name, const std::wstring& val) {
    HKEY hKey = nullptr;
    if (RegCreateKeyExW(root, subkey, 0, nullptr, 0, KEY_WRITE, nullptr, &hKey, nullptr) == ERROR_SUCCESS) {
        RegSetValueExW(hKey, name, 0, REG_SZ,
                       reinterpret_cast<const BYTE*>(val.c_str()),
                       (DWORD)((val.size() + 1) * sizeof(wchar_t)));
        RegCloseKey(hKey);
    }
}

static bool write_binary_blob(const fs::path& dest, const uint8_t* start, const uint8_t* end) {
    std::error_code ec;
    fs::create_directories(dest.parent_path(), ec);
    std::ofstream ofs(dest, std::ios::binary | std::ios::trunc);
    if (!ofs) return false;
    ofs.write(reinterpret_cast<const char*>(start), (std::streamsize)(end - start));
    return ofs.good();
}

static int run_uninstaller(bool silent) {
    wchar_t self_buf[MAX_PATH * 2] = {};
    GetModuleFileNameW(nullptr, self_buf, MAX_PATH * 2);
    fs::path install_dir = fs::path(self_buf).parent_path();

    if (!silent) {
        int ans = MessageBoxW(nullptr,
            L"Uninstall AI Coding Agent from your Windows account?\n\n"
            L"(Your SQLite session database in %LOCALAPPDATA%\\AI-Coding-Agent will be preserved.)",
            L"AI Coding Agent Uninstall",
            MB_ICONQUESTION | MB_YESNO);
        if (ans != IDYES) return 0;
    }

    std::error_code ec;
    fs::remove(install_dir / L"AI-Coding-Agent.exe", ec);
    fs::remove(install_dir / L"icon.ico", ec);

    fs::path desktop = get_known_folder(FOLDERID_Desktop);
    if (!desktop.empty()) fs::remove(desktop / L"AI Coding Agent.lnk", ec);

    fs::path programs = get_known_folder(FOLDERID_Programs);
    if (!programs.empty()) fs::remove_all(programs / L"AI Coding Agent", ec);

    RegDeleteTreeW(HKEY_CURRENT_USER, L"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AICodingAgent");
    RegDeleteTreeW(HKEY_CURRENT_USER, L"Software\\Aircode\\AI-Coding-Agent");
    RegDeleteTreeW(HKEY_CURRENT_USER, L"Software\\Classes\\Directory\\shell\\AICodingAgent");
    RegDeleteTreeW(HKEY_CURRENT_USER, L"Software\\Classes\\Directory\\Background\\shell\\AICodingAgent");

    if (!silent) {
        MessageBoxW(nullptr, L"AI Coding Agent has been uninstalled.", L"AI Coding Agent Setup", MB_ICONINFORMATION | MB_OK);
    }
    return 0;
}

static bool perform_installation(const fs::path& install_dir,
                                 bool make_desktop,
                                 bool make_startmenu,
                                 bool make_ctxmenu,
                                 bool launch_after,
                                 HWND hwnd_ui) {
    auto set_step = [&](int pct, const wchar_t* status) {
        if (g_progress) SendMessageW(g_progress, PBM_SETPOS, pct, 0);
        if (g_lbl_status) SetWindowTextW(g_lbl_status, status);
        if (hwnd_ui) UpdateWindow(hwnd_ui);
    };

    set_step(15, L"Extracting AI-Coding-Agent.exe (Windows x64)...");
    fs::path target_exe = install_dir / L"AI-Coding-Agent.exe";
    fs::path target_ico = install_dir / L"icon.ico";
    fs::path uninst_exe = install_dir / L"Uninstall.exe";

    if (!write_binary_blob(target_exe, g_app_exe_start, g_app_exe_end)) {
        if (hwnd_ui) {
            MessageBoxW(hwnd_ui,
                L"Could not write AI-Coding-Agent.exe to target folder.\n"
                L"Close any running AI-Coding-Agent.exe instance or choose another directory.",
                L"Installation Error", MB_ICONERROR | MB_OK);
        }
        return false;
    }
    write_binary_blob(target_ico, g_app_ico_start, g_app_ico_end);

    set_step(45, L"Creating uninstaller & Windows registry entries...");
    wchar_t self_path[MAX_PATH * 2] = {};
    GetModuleFileNameW(nullptr, self_path, MAX_PATH * 2);
    CopyFileW(self_path, uninst_exe.c_str(), FALSE);

    const wchar_t* uninst_key = L"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AICodingAgent";
    write_reg_sz(HKEY_CURRENT_USER, uninst_key, L"DisplayName", L"AI Coding Agent");
    write_reg_sz(HKEY_CURRENT_USER, uninst_key, L"DisplayVersion", L"1.0.0");
    write_reg_sz(HKEY_CURRENT_USER, uninst_key, L"Publisher", L"Aircode Desktop");
    write_reg_sz(HKEY_CURRENT_USER, uninst_key, L"DisplayIcon", target_exe.wstring());
    write_reg_sz(HKEY_CURRENT_USER, uninst_key, L"InstallLocation", install_dir.wstring());
    write_reg_sz(HKEY_CURRENT_USER, uninst_key, L"UninstallString", L"\"" + uninst_exe.wstring() + L"\" --uninstall");

    set_step(70, L"Creating Windows shortcuts...");
    if (make_desktop) {
        fs::path desktop = get_known_folder(FOLDERID_Desktop);
        if (!desktop.empty()) {
            create_windows_shortcut(desktop / L"AI Coding Agent.lnk", target_exe, target_ico,
                                    L"AI Coding Agent — Native Windows Desktop IDE & Autonomous Agent");
        }
    }
    if (make_startmenu) {
        fs::path programs = get_known_folder(FOLDERID_Programs);
        if (!programs.empty()) {
            create_windows_shortcut(programs / L"AI Coding Agent" / L"AI Coding Agent.lnk", target_exe, target_ico,
                                    L"AI Coding Agent — Native Windows Desktop IDE & Autonomous Agent");
        }
    }

    if (make_ctxmenu) {
        set_step(85, L"Registering Windows Explorer folder context menu...");
        const wchar_t* k1 = L"Software\\Classes\\Directory\\shell\\AICodingAgent";
        const wchar_t* k1c = L"Software\\Classes\\Directory\\shell\\AICodingAgent\\command";
        write_reg_sz(HKEY_CURRENT_USER, k1, nullptr, L"Open with AI Coding Agent");
        write_reg_sz(HKEY_CURRENT_USER, k1, L"Icon", target_exe.wstring());
        write_reg_sz(HKEY_CURRENT_USER, k1c, nullptr, L"\"" + target_exe.wstring() + L"\" \"%V\"");

        const wchar_t* k2 = L"Software\\Classes\\Directory\\Background\\shell\\AICodingAgent";
        const wchar_t* k2c = L"Software\\Classes\\Directory\\Background\\shell\\AICodingAgent\\command";
        write_reg_sz(HKEY_CURRENT_USER, k2, nullptr, L"Open with AI Coding Agent");
        write_reg_sz(HKEY_CURRENT_USER, k2, L"Icon", target_exe.wstring());
        write_reg_sz(HKEY_CURRENT_USER, k2c, nullptr, L"\"" + target_exe.wstring() + L"\" \"%V\"");
    }

    set_step(100, L"Installation complete!");

    if (launch_after) {
        ShellExecuteW(hwnd_ui, L"open", target_exe.c_str(), nullptr, install_dir.c_str(), SW_SHOWNORMAL);
    }
    return true;
}

static void browse_install_dir(HWND owner) {
    IFileOpenDialog* dlg = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&dlg))) && dlg) {
        DWORD opts = 0;
        dlg->GetOptions(&opts);
        dlg->SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
        dlg->SetTitle(L"Select Installation Folder for AI Coding Agent");
        if (SUCCEEDED(dlg->Show(owner))) {
            IShellItem* item = nullptr;
            if (SUCCEEDED(dlg->GetResult(&item)) && item) {
                PWSTR path = nullptr;
                if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &path)) && path) {
                    fs::path chosen = fs::path(path) / L"AI-Coding-Agent";
                    SetWindowTextW(g_edit_dir, chosen.c_str());
                    CoTaskMemFree(path);
                }
                item->Release();
            }
        }
        dlg->Release();
    }
}

LRESULT CALLBACK SetupWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    switch (msg) {
        case WM_CREATE: {
            g_font_title = CreateFontW(-20, 0, 0, 0, FW_BOLD, FALSE, FALSE, FALSE,
                                       DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                       CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE, L"Segoe UI");
            g_font_ui = CreateFontW(-14, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
                                    DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                    CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE, L"Segoe UI");

            HWND hTitle = CreateWindowExW(0, L"STATIC", L"AI Coding Agent v1.0.0 Setup (Windows x64)",
                WS_CHILD | WS_VISIBLE, 24, 20, 500, 28, hwnd, nullptr, nullptr, nullptr);
            SendMessageW(hTitle, WM_SETFONT, (WPARAM)g_font_title, TRUE);

            HWND hSub = CreateWindowExW(0, L"STATIC",
                L"Installs the native Windows x64 AI Coding Agent desktop application (Monaco Editor, ConPTY Terminal, SQLite & Multi-Provider Autonomous Agent).",
                WS_CHILD | WS_VISIBLE, 24, 52, 510, 38, hwnd, nullptr, nullptr, nullptr);
            SendMessageW(hSub, WM_SETFONT, (WPARAM)g_font_ui, TRUE);

            HWND hDirLbl = CreateWindowExW(0, L"STATIC", L"Destination Folder:",
                WS_CHILD | WS_VISIBLE, 24, 102, 200, 20, hwnd, nullptr, nullptr, nullptr);
            SendMessageW(hDirLbl, WM_SETFONT, (WPARAM)g_font_ui, TRUE);

            fs::path def_dir = get_known_folder(FOLDERID_LocalAppData) / L"Programs" / L"AI-Coding-Agent";
            g_edit_dir = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", def_dir.c_str(),
                WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL, 24, 124, 412, 26, hwnd, (HMENU)ID_EDIT_DIR, nullptr, nullptr);
            SendMessageW(g_edit_dir, WM_SETFONT, (WPARAM)g_font_ui, TRUE);

            HWND hBrowse = CreateWindowExW(0, L"BUTTON", L"Browse...",
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 444, 123, 90, 28, hwnd, (HMENU)ID_BTN_BROWSE, nullptr, nullptr);
            SendMessageW(hBrowse, WM_SETFONT, (WPARAM)g_font_ui, TRUE);

            g_chk_desktop = CreateWindowExW(0, L"BUTTON", L"Create Desktop shortcut",
                WS_CHILD | WS_VISIBLE | BS_AUTOCHECKBOX, 24, 166, 480, 22, hwnd, (HMENU)ID_CHK_DESKTOP, nullptr, nullptr);
            SendMessageW(g_chk_desktop, WM_SETFONT, (WPARAM)g_font_ui, TRUE);
            SendMessageW(g_chk_desktop, BM_SETCHECK, BST_CHECKED, 0);

            g_chk_startmenu = CreateWindowExW(0, L"BUTTON", L"Create Start Menu shortcut",
                WS_CHILD | WS_VISIBLE | BS_AUTOCHECKBOX, 24, 192, 480, 22, hwnd, (HMENU)ID_CHK_STARTMENU, nullptr, nullptr);
            SendMessageW(g_chk_startmenu, WM_SETFONT, (WPARAM)g_font_ui, TRUE);
            SendMessageW(g_chk_startmenu, BM_SETCHECK, BST_CHECKED, 0);

            g_chk_ctxmenu = CreateWindowExW(0, L"BUTTON", L"Add 'Open with AI Coding Agent' to Windows Explorer folders",
                WS_CHILD | WS_VISIBLE | BS_AUTOCHECKBOX, 24, 218, 480, 22, hwnd, (HMENU)ID_CHK_CTXMENU, nullptr, nullptr);
            SendMessageW(g_chk_ctxmenu, WM_SETFONT, (WPARAM)g_font_ui, TRUE);
            SendMessageW(g_chk_ctxmenu, BM_SETCHECK, BST_CHECKED, 0);

            g_chk_launch = CreateWindowExW(0, L"BUTTON", L"Launch AI Coding Agent immediately after installation",
                WS_CHILD | WS_VISIBLE | BS_AUTOCHECKBOX, 24, 244, 480, 22, hwnd, (HMENU)ID_CHK_LAUNCH, nullptr, nullptr);
            SendMessageW(g_chk_launch, WM_SETFONT, (WPARAM)g_font_ui, TRUE);
            SendMessageW(g_chk_launch, BM_SETCHECK, BST_CHECKED, 0);

            g_lbl_status = CreateWindowExW(0, L"STATIC", L"Ready to install.",
                WS_CHILD | WS_VISIBLE, 24, 280, 510, 20, hwnd, (HMENU)ID_LBL_STATUS, nullptr, nullptr);
            SendMessageW(g_lbl_status, WM_SETFONT, (WPARAM)g_font_ui, TRUE);

            g_progress = CreateWindowExW(0, PROGRESS_CLASSW, nullptr,
                WS_CHILD | WS_VISIBLE | PBS_SMOOTH, 24, 304, 510, 18, hwnd, (HMENU)ID_PROGRESS, nullptr, nullptr);
            SendMessageW(g_progress, PBM_SETRANGE, 0, MAKELPARAM(0, 100));

            g_btn_install = CreateWindowExW(0, L"BUTTON", L"Install Now",
                WS_CHILD | WS_VISIBLE | BS_DEFPUSHBUTTON, 324, 342, 106, 32, hwnd, (HMENU)ID_BTN_INSTALL, nullptr, nullptr);
            SendMessageW(g_btn_install, WM_SETFONT, (WPARAM)g_font_ui, TRUE);

            HWND hCancel = CreateWindowExW(0, L"BUTTON", L"Cancel",
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 438, 342, 96, 32, hwnd, (HMENU)ID_BTN_CANCEL, nullptr, nullptr);
            SendMessageW(hCancel, WM_SETFONT, (WPARAM)g_font_ui, TRUE);
            return 0;
        }
        case WM_COMMAND: {
            int id = LOWORD(wParam);
            if (id == ID_BTN_BROWSE) {
                browse_install_dir(hwnd);
                return 0;
            }
            if (id == ID_BTN_CANCEL) {
                DestroyWindow(hwnd);
                return 0;
            }
            if (id == ID_BTN_INSTALL) {
                EnableWindow(g_btn_install, FALSE);
                wchar_t dir_buf[MAX_PATH * 2] = {};
                GetWindowTextW(g_edit_dir, dir_buf, MAX_PATH * 2);
                bool d = SendMessageW(g_chk_desktop, BM_GETCHECK, 0, 0) == BST_CHECKED;
                bool sm = SendMessageW(g_chk_startmenu, BM_GETCHECK, 0, 0) == BST_CHECKED;
                bool ctx = SendMessageW(g_chk_ctxmenu, BM_GETCHECK, 0, 0) == BST_CHECKED;
                bool lnch = SendMessageW(g_chk_launch, BM_GETCHECK, 0, 0) == BST_CHECKED;

                if (perform_installation(fs::path(dir_buf), d, sm, ctx, lnch, hwnd)) {
                    if (!lnch) {
                        MessageBoxW(hwnd, L"AI Coding Agent v1.0.0 was installed successfully!", L"AI Coding Agent Setup", MB_ICONINFORMATION | MB_OK);
                    }
                    DestroyWindow(hwnd);
                } else {
                    EnableWindow(g_btn_install, TRUE);
                }
                return 0;
            }
            break;
        }
        case WM_DESTROY:
            if (g_font_title) DeleteObject(g_font_title);
            if (g_font_ui) DeleteObject(g_font_ui);
            PostQuitMessage(0);
            return 0;
    }
    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE, PWSTR lpCmdLine, int nCmdShow) {
    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    InitCommonControls();

    std::wstring cmdline = lpCmdLine ? lpCmdLine : L"";
    bool is_silent = (cmdline.find(L"--silent") != std::wstring::npos || cmdline.find(L"/S") != std::wstring::npos);
    if (cmdline.find(L"--uninstall") != std::wstring::npos) {
        int rc = run_uninstaller(is_silent);
        CoUninitialize();
        return rc;
    }

    if (is_silent) {
        fs::path def_dir = get_known_folder(FOLDERID_LocalAppData) / L"Programs" / L"AI-Coding-Agent";
        bool ok = perform_installation(def_dir, true, true, true, false, nullptr);
        CoUninitialize();
        return ok ? 0 : 1;
    }

    HICON icon = LoadIconW(hInstance, MAKEINTRESOURCEW(1));

    WNDCLASSEXW wc{};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = SetupWndProc;
    wc.hInstance = hInstance;
    wc.hIcon = icon;
    wc.hIconSm = icon;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.lpszClassName = L"AICodingAgentSetupWizard";
    RegisterClassExW(&wc);

    int sw = GetSystemMetrics(SM_CXSCREEN);
    int sh = GetSystemMetrics(SM_CYSCREEN);
    int w = 566, h = 424;

    HWND hwnd = CreateWindowExW(
        0,
        wc.lpszClassName,
        L"AI Coding Agent Setup (x64)",
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        (sw - w) / 2, (sh - h) / 2, w, h,
        nullptr, nullptr, hInstance, nullptr);

    ShowWindow(hwnd, nCmdShow);
    UpdateWindow(hwnd);

    MSG msg{};
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    CoUninitialize();
    return (int)msg.wParam;
}
