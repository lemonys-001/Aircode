#pragma once

#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0A00
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <shobjidl.h>
#include <shlobj.h>
#include <shellapi.h>
#include <shlwapi.h>
#include <dwmapi.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <mutex>
#include <random>
#include <regex>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

namespace aircode {

namespace fs = std::filesystem;

inline std::wstring utf8_to_wide(const std::string& str) {
    if (str.empty()) return L"";
    int needed = MultiByteToWideChar(CP_UTF8, 0, str.data(), static_cast<int>(str.size()), nullptr, 0);
    if (needed <= 0) return L"";
    std::wstring out(needed, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, str.data(), static_cast<int>(str.size()), out.data(), needed);
    return out;
}

inline std::string wide_to_utf8(const std::wstring& wstr) {
    if (wstr.empty()) return "";
    int needed = WideCharToMultiByte(CP_UTF8, 0, wstr.data(), static_cast<int>(wstr.size()), nullptr, 0, nullptr, nullptr);
    if (needed <= 0) return "";
    std::string out(needed, '\0');
    WideCharToMultiByte(CP_UTF8, 0, wstr.data(), static_cast<int>(wstr.size()), out.data(), needed, nullptr, nullptr);
    return out;
}

inline fs::path u8path_safe(const std::string& utf8) {
    return fs::path(utf8_to_wide(utf8));
}

inline std::string path_to_utf8(const fs::path& p) {
    std::string s = wide_to_utf8(p.wstring());
    std::replace(s.begin(), s.end(), '\\', '/');
    return s;
}

inline std::string path_to_native_utf8(const fs::path& p) {
    return wide_to_utf8(p.wstring());
}

inline std::string now_iso8601() {
    auto now = std::chrono::system_clock::now();
    std::time_t t = std::chrono::system_clock::to_time_t(now);
    std::tm tm_utc{};
    gmtime_s(&tm_utc, &t);
    std::ostringstream oss;
    oss << std::put_time(&tm_utc, "%Y-%m-%dT%H:%M:%SZ");
    return oss.str();
}

inline int64_t now_ms() {
    return std::chrono::duration_cast<std::chrono::milliseconds>(
               std::chrono::system_clock::now().time_since_epoch())
        .count();
}

inline std::string random_id(const std::string& prefix = "id") {
    static thread_local std::mt19937_64 rng{std::random_device{}()};
    static const char* hex = "0123456789abcdef";
    std::string out = prefix + "_";
    uint64_t v = rng();
    for (int i = 0; i < 12; ++i) {
        out.push_back(hex[(v >> (i * 4)) & 0xF]);
    }
    return out;
}

inline std::string get_exe_path_utf8() {
    wchar_t buf[MAX_PATH * 4] = {0};
    DWORD len = GetModuleFileNameW(nullptr, buf, MAX_PATH * 4);
    return wide_to_utf8(std::wstring(buf, len));
}

inline fs::path get_appdata_dir() {
    // Support portable mode if ".aircode-portable" exists next to Aircode.exe
    fs::path exe_dir = u8path_safe(get_exe_path_utf8()).parent_path();
    std::error_code ec;
    if (fs::exists(exe_dir / L".aircode-portable", ec)) {
        fs::path p = exe_dir / L".aircode-data";
        fs::create_directories(p, ec);
        return p;
    }
    PWSTR roaming = nullptr;
    if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_RoamingAppData, 0, nullptr, &roaming)) && roaming) {
        fs::path p = fs::path(roaming) / L"Aircode";
        CoTaskMemFree(roaming);
        fs::create_directories(p, ec);
        return p;
    }
    fs::path fallback = exe_dir / L".aircode-data";
    fs::create_directories(fallback, ec);
    return fallback;
}

inline std::string read_text_file(const fs::path& p, bool* ok = nullptr) {
    std::ifstream ifs(p, std::ios::binary);
    if (!ifs) {
        if (ok) *ok = false;
        return "";
    }
    if (ok) *ok = true;
    std::ostringstream ss;
    ss << ifs.rdbuf();
    return ss.str();
}

inline bool write_text_file(const fs::path& p, const std::string& content) {
    std::error_code ec;
    if (p.has_parent_path()) {
        fs::create_directories(p.parent_path(), ec);
    }
    std::ofstream ofs(p, std::ios::binary | std::ios::trunc);
    if (!ofs) return false;
    ofs.write(content.data(), static_cast<std::streamsize>(content.size()));
    return ofs.good();
}

struct ProcessResult {
    int exit_code = -1;
    std::string stdout_text;
    std::string stderr_text;
    int64_t duration_ms = 0;
    bool timed_out = false;
};

// Runs a process silently using CreateProcessW with CREATE_NO_WINDOW (zero console flash!)
inline ProcessResult run_process_silent(
    const std::string& command,
    const std::string& cwd_utf8,
    const std::string& shell_pref = "powershell",
    int timeout_ms = 30000,
    std::atomic<bool>* cancel_flag = nullptr
) {
    ProcessResult res;
    auto t0 = std::chrono::steady_clock::now();

    SECURITY_ATTRIBUTES sa{};
    sa.nLength = sizeof(SECURITY_ATTRIBUTES);
    sa.bInheritHandle = TRUE;
    sa.lpSecurityDescriptor = nullptr;

    HANDLE out_read = nullptr, out_write = nullptr;
    HANDLE err_read = nullptr, err_write = nullptr;

    if (!CreatePipe(&out_read, &out_write, &sa, 0) ||
        !CreatePipe(&err_read, &err_write, &sa, 0)) {
        res.stderr_text = "Failed to create process pipes.";
        return res;
    }
    SetHandleInformation(out_read, HANDLE_FLAG_INHERIT, 0);
    SetHandleInformation(err_read, HANDLE_FLAG_INHERIT, 0);

    std::wstring cmdline;
    if (shell_pref == "direct") {
        cmdline = utf8_to_wide(command);
    } else if (shell_pref == "cmd") {
        cmdline = L"cmd.exe /d /s /c \"" + utf8_to_wide(command) + L"\"";
    } else if (shell_pref == "bash") {
        cmdline = L"bash.exe -lc \"" + utf8_to_wide(command) + L"\"";
    } else {
        // PowerShell UTF-8 clean non-interactive invocation
        std::wstring wcmd = utf8_to_wide(command);
        // Escape double quotes for powershell -Command
        std::wstring escaped;
        escaped.reserve(wcmd.size() + 32);
        for (wchar_t c : wcmd) {
            if (c == L'"') escaped += L"\\\"";
            else escaped += c;
        }
        cmdline = L"powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \"[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; " + escaped + L"\"";
    }

    STARTUPINFOW si{};
    si.cb = sizeof(STARTUPINFOW);
    si.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
    si.wShowWindow = SW_HIDE;
    si.hStdOutput = out_write;
    si.hStdError = err_write;
    si.hStdInput = GetStdHandle(STD_INPUT_HANDLE);

    PROCESS_INFORMATION pi{};
    std::wstring wcwd = utf8_to_wide(cwd_utf8);
    std::vector<wchar_t> cmd_buf(cmdline.begin(), cmdline.end());
    cmd_buf.push_back(L'\0');

    BOOL created = CreateProcessW(
        nullptr,
        cmd_buf.data(),
        nullptr,
        nullptr,
        TRUE,
        CREATE_NO_WINDOW,
        nullptr,
        wcwd.empty() ? nullptr : wcwd.c_str(),
        &si,
        &pi
    );

    CloseHandle(out_write);
    CloseHandle(err_write);

    if (!created) {
        CloseHandle(out_read);
        CloseHandle(err_read);
        res.stderr_text = "Failed to spawn process (Win32 error " + std::to_string(GetLastError()) + ")";
        return res;
    }

    auto read_pipe = [](HANDLE h, std::string* dst) {
        char buf[4096];
        DWORD n = 0;
        while (ReadFile(h, buf, sizeof(buf), &n, nullptr) && n > 0) {
            if (dst->size() < 512 * 1024) {
                dst->append(buf, n);
            }
        }
    };

    std::thread t_out(read_pipe, out_read, &res.stdout_text);
    std::thread t_err(read_pipe, err_read, &res.stderr_text);

    int elapsed = 0;
    const int step = 40;
    DWORD wait_rc = WAIT_TIMEOUT;
    while (elapsed < timeout_ms) {
        if (cancel_flag && cancel_flag->load()) {
            TerminateProcess(pi.hProcess, 130);
            res.timed_out = true;
            break;
        }
        wait_rc = WaitForSingleObject(pi.hProcess, step);
        if (wait_rc == WAIT_OBJECT_0) break;
        elapsed += step;
    }
    if (wait_rc == WAIT_TIMEOUT && !res.timed_out) {
        TerminateProcess(pi.hProcess, 124);
        res.timed_out = true;
    }

    DWORD code = 0;
    if (GetExitCodeProcess(pi.hProcess, &code)) {
        res.exit_code = static_cast<int>(code);
    }

    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    if (t_out.joinable()) t_out.join();
    if (t_err.joinable()) t_err.join();
    CloseHandle(out_read);
    CloseHandle(err_read);

    res.duration_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                          std::chrono::steady_clock::now() - t0)
                          .count();
    return res;
}

// Opens modern Windows 10/11 Explorer folder picker dialog (IFileOpenDialog with FOS_PICKFOLDERS)
inline std::string pick_folder_dialog_win32(const std::string& initial_dir_utf8 = "") {
    std::string selected;
    std::thread dialog_thread([&]() {
        HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        IFileOpenDialog* pfd = nullptr;
        if (SUCCEEDED(CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&pfd))) && pfd) {
            DWORD opts = 0;
            if (SUCCEEDED(pfd->GetOptions(&opts))) {
                pfd->SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST);
            }
            pfd->SetTitle(L"Open Project Folder in Aircode");
            if (!initial_dir_utf8.empty()) {
                IShellItem* psi_init = nullptr;
                std::wstring winit = utf8_to_wide(initial_dir_utf8);
                std::replace(winit.begin(), winit.end(), L'/', L'\\');
                if (SUCCEEDED(SHCreateItemFromParsingName(winit.c_str(), nullptr, IID_PPV_ARGS(&psi_init))) && psi_init) {
                    pfd->SetFolder(psi_init);
                    psi_init->Release();
                }
            }
            if (SUCCEEDED(pfd->Show(GetForegroundWindow()))) {
                IShellItem* psi = nullptr;
                if (SUCCEEDED(pfd->GetResult(&psi)) && psi) {
                    PWSTR pszPath = nullptr;
                    if (SUCCEEDED(psi->GetDisplayName(SIGDN_FILESYSPATH, &pszPath)) && pszPath) {
                        selected = wide_to_utf8(pszPath);
                        CoTaskMemFree(pszPath);
                    }
                    psi->Release();
                }
            }
            pfd->Release();
        }
        if (SUCCEEDED(hr)) {
            CoUninitialize();
        }
    });
    dialog_thread.join();
    return selected;
}

// Registers or unregisters "Open in Aircode" in Windows Explorer context menu under HKCU (Zero Admin UAC needed!)
inline bool set_windows_context_menu(bool enable, std::string* err_out = nullptr) {
    std::wstring exe_w = utf8_to_wide(get_exe_path_utf8());
    const wchar_t* keys[] = {
        L"Software\\Classes\\Directory\\shell\\Aircode",
        L"Software\\Classes\\Directory\\Background\\shell\\Aircode"
    };
    const wchar_t* args[] = { L"\"%1\"", L"\"%V\"" };

    if (!enable) {
        for (const wchar_t* k : keys) {
            SHDeleteKeyW(HKEY_CURRENT_USER, k);
        }
        return true;
    }

    for (int i = 0; i < 2; ++i) {
        HKEY hKey = nullptr;
        if (RegCreateKeyExW(HKEY_CURRENT_USER, keys[i], 0, nullptr, 0, KEY_WRITE, nullptr, &hKey, nullptr) != ERROR_SUCCESS) {
            if (err_out) *err_out = "Failed to open HKCU registry key.";
            return false;
        }
        std::wstring label = L"Open in Aircode";
        RegSetValueExW(hKey, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(label.c_str()), static_cast<DWORD>((label.size() + 1) * sizeof(wchar_t)));
        RegSetValueExW(hKey, L"Icon", 0, REG_SZ, reinterpret_cast<const BYTE*>(exe_w.c_str()), static_cast<DWORD>((exe_w.size() + 1) * sizeof(wchar_t)));
        RegCloseKey(hKey);

        std::wstring cmd_key = std::wstring(keys[i]) + L"\\command";
        HKEY hCmd = nullptr;
        if (RegCreateKeyExW(HKEY_CURRENT_USER, cmd_key.c_str(), 0, nullptr, 0, KEY_WRITE, nullptr, &hCmd, nullptr) == ERROR_SUCCESS) {
            std::wstring cmd_val = L"\"" + exe_w + L"\" " + args[i];
            RegSetValueExW(hCmd, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(cmd_val.c_str()), static_cast<DWORD>((cmd_val.size() + 1) * sizeof(wchar_t)));
            RegCloseKey(hCmd);
        }
    }
    return true;
}

inline void apply_dark_titlebar(HWND hwnd) {
    if (!hwnd) return;
    BOOL use_dark = TRUE;
    // DWMWA_USE_IMMERSIVE_DARK_MODE = 20 (Win10 2004+ / Win11)
    DwmSetWindowAttribute(hwnd, 20, &use_dark, sizeof(use_dark));
}

inline std::vector<std::string> split_lines(const std::string& text) {
    std::vector<std::string> lines;
    std::istringstream ss(text);
    std::string line;
    while (std::getline(ss, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        lines.push_back(line);
    }
    return lines;
}

// Fast LCS-windowed unified diff generator for visual diff cards
inline std::string compute_unified_diff(const std::string& rel_path, const std::string& old_str, const std::string& new_str) {
    auto old_lines = split_lines(old_str);
    auto new_lines = split_lines(new_str);
    std::ostringstream out;
    out << "--- a/" << rel_path << "\n+++ b/" << rel_path << "\n";

    size_t prefix = 0;
    while (prefix < old_lines.size() && prefix < new_lines.size() && old_lines[prefix] == new_lines[prefix]) {
        ++prefix;
    }
    size_t old_end = old_lines.size();
    size_t new_end = new_lines.size();
    while (old_end > prefix && new_end > prefix && old_lines[old_end - 1] == new_lines[new_end - 1]) {
        --old_end;
        --new_end;
    }
    if (prefix == old_end && prefix == new_end) {
        return "";
    }
    size_t ctx_start = prefix > 3 ? prefix - 3 : 0;
    size_t old_ctx_end = std::min(old_lines.size(), old_end + 3);
    size_t new_ctx_end = std::min(new_lines.size(), new_end + 3);

    out << "@@ -" << (ctx_start + 1) << "," << (old_ctx_end - ctx_start)
        << " +" << (ctx_start + 1) << "," << (new_ctx_end - ctx_start) << " @@\n";
    for (size_t i = ctx_start; i < prefix; ++i) {
        out << " " << old_lines[i] << "\n";
    }
    for (size_t i = prefix; i < old_end; ++i) {
        out << "-" << old_lines[i] << "\n";
    }
    for (size_t i = prefix; i < new_end; ++i) {
        out << "+" << new_lines[i] << "\n";
    }
    for (size_t i = old_end; i < old_ctx_end; ++i) {
        out << " " << old_lines[i] << "\n";
    }
    return out.str();
}

} // namespace aircode
