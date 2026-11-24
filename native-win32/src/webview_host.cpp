#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0A00
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif

#include <windows.h>
#include <dwmapi.h>
#include <shellapi.h>
#include <string>

#include "webview/webview.h"

namespace aircode {

static LRESULT CALLBACK FallbackWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    switch (msg) {
        case WM_CREATE: {
            BOOL dark = TRUE;
            DwmSetWindowAttribute(hwnd, 20, &dark, sizeof(dark));
            break;
        }
        case WM_PAINT: {
            PAINTSTRUCT ps;
            HDC hdc = BeginPaint(hwnd, &ps);
            RECT rc;
            GetClientRect(hwnd, &rc);
            HBRUSH bg = CreateSolidBrush(RGB(13, 17, 23));
            FillRect(hdc, &rc, bg);
            DeleteObject(bg);
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, RGB(226, 232, 240));
            const wchar_t* text = L"Aircode Engine Running — UI opened in Desktop App Window.\nClose this controller window to exit Aircode.";
            RECT tr = {24, 28, rc.right - 24, rc.bottom - 24};
            DrawTextW(hdc, text, -1, &tr, DT_LEFT | DT_WORDBREAK);
            EndPaint(hwnd, &ps);
            return 0;
        }
        case WM_DESTROY:
            PostQuitMessage(0);
            return 0;
    }
    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

static void run_fallback_app_window(const std::wstring& url_w, HICON hIcon) {
    std::wstring app_arg = L"--app=" + url_w;
    HINSTANCE res = ShellExecuteW(nullptr, L"open", L"msedge.exe", app_arg.c_str(), nullptr, SW_SHOWNORMAL);
    if (reinterpret_cast<INT_PTR>(res) <= 32) {
        res = ShellExecuteW(nullptr, L"open", L"chrome.exe", app_arg.c_str(), nullptr, SW_SHOWNORMAL);
    }
    if (reinterpret_cast<INT_PTR>(res) <= 32) {
        ShellExecuteW(nullptr, L"open", url_w.c_str(), nullptr, nullptr, SW_SHOWNORMAL);
    }

    WNDCLASSEXW wc{};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = FallbackWndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.hIcon = hIcon;
    wc.hIconSm = hIcon;
    wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
    wc.lpszClassName = L"AircodeFallbackHost";
    RegisterClassExW(&wc);

    HWND hwnd = CreateWindowExW(
        0, L"AircodeFallbackHost", L"Aircode Studio Engine",
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        CW_USEDEFAULT, CW_USEDEFAULT, 460, 140,
        nullptr, nullptr, wc.hInstance, nullptr
    );
    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);

    MSG msg;
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

void run_aircode_desktop_window(const std::string& url_utf8, const std::wstring& webview_data_dir, const std::wstring& window_title) {
    SetEnvironmentVariableW(L"WEBVIEW2_USER_DATA_FOLDER", webview_data_dir.c_str());
    SetEnvironmentVariableW(L"WEBVIEW2_DEFAULT_BACKGROUND_COLOR", L"FF0B0F19");

    HICON hIcon = LoadIconW(GetModuleHandleW(nullptr), MAKEINTRESOURCEW(1));

    try {
        webview::webview w(true, nullptr);

        auto hwnd_res = w.window();
        if (hwnd_res.ok()) {
            HWND hwnd = static_cast<HWND>(hwnd_res.value());
            if (hwnd) {
                BOOL dark = TRUE;
                DwmSetWindowAttribute(hwnd, 20, &dark, sizeof(dark));
                if (hIcon) {
                    SendMessageW(hwnd, WM_SETICON, ICON_BIG, reinterpret_cast<LPARAM>(hIcon));
                    SendMessageW(hwnd, WM_SETICON, ICON_SMALL, reinterpret_cast<LPARAM>(hIcon));
                }
                SetWindowTextW(hwnd, window_title.c_str());
            }
        }

        w.set_size(1440, 900, WEBVIEW_HINT_NONE);
        w.set_size(980, 640, WEBVIEW_HINT_MIN);
        w.navigate(url_utf8);
        w.run();
    } catch (...) {
        int len = MultiByteToWideChar(CP_UTF8, 0, url_utf8.data(), static_cast<int>(url_utf8.size()), nullptr, 0);
        std::wstring wurl(len, L'\0');
        MultiByteToWideChar(CP_UTF8, 0, url_utf8.data(), static_cast<int>(url_utf8.size()), wurl.data(), len);
        run_fallback_app_window(wurl, hIcon);
    }
}

} // namespace aircode
