#include "agent_server.hpp"

namespace aircode {
void run_aircode_desktop_window(const std::string& url_utf8, const std::wstring& webview_data_dir, const std::wstring& window_title);
}

int WINAPI WinMain(HINSTANCE, HINSTANCE, LPSTR, int) {
    WSADATA wsa{};
    WSAStartup(MAKEWORD(2, 2), &wsa);

    std::string initial_workspace;
    int argc = 0;
    LPWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (argv) {
        if (argc >= 2) {
            std::wstring arg1 = argv[1];
            std::error_code ec;
            std::filesystem::path p(arg1);
            if (std::filesystem::exists(p, ec)) {
                if (std::filesystem::is_regular_file(p, ec)) {
                    p = p.parent_path();
                }
                initial_workspace = aircode::wide_to_utf8(p.wstring());
            }
        }
        LocalFree(argv);
    }

    if (initial_workspace.empty()) {
        std::error_code ec;
        auto cwd = std::filesystem::current_path(ec);
        std::string cwd_str = aircode::wide_to_utf8(cwd.wstring());
        std::string low = aircode::to_lower_ascii(cwd_str);
        if (low.find("\\windows\\system32") != std::string::npos) {
            wchar_t prof[MAX_PATH] = {0};
            if (GetEnvironmentVariableW(L"USERPROFILE", prof, MAX_PATH) > 0) {
                initial_workspace = aircode::wide_to_utf8(prof);
            }
        } else {
            initial_workspace = cwd_str;
        }
    }

    aircode::AircodeEngine engine(initial_workspace);
    int port = engine.start_background_server();

    std::filesystem::path wv_data = aircode::get_appdata_dir() / L"WebView2Data";
    std::error_code ec;
    std::filesystem::create_directories(wv_data, ec);

    std::string url = "http://127.0.0.1:" + std::to_string(port) + "/";
    std::wstring title = L"Aircode Studio — Ultra-Fast AI Coding IDE";

    aircode::run_aircode_desktop_window(url, wv_data.wstring(), title);

    engine.stop();
    WSACleanup();
    return 0;
}
