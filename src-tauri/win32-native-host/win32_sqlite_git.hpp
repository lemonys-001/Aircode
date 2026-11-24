#pragma once
#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif
#ifndef NTDDI_VERSION
#define NTDDI_VERSION 0x0A000008
#endif
#define NOMINMAX
#define WIN32_LEAN_AND_MEAN

#include <windows.h>
#include <wincon.h>
#include <commdlg.h>
#include <dwmapi.h>
#include <objbase.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shobjidl.h>
#include <winhttp.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <filesystem>
#include <fstream>
#include <functional>
#include <map>
#include <mutex>
#include <random>
#include <set>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

#include "vendor/json.hpp"
#include "vendor/sqlite3.h"

namespace fs = std::filesystem;
using json = nlohmann::json;

inline std::wstring utf8_to_wide(const std::string& s) {
    if (s.empty()) return L"";
    int len = MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), nullptr, 0);
    if (len <= 0) return L"";
    std::wstring out(len, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), out.data(), len);
    return out;
}

inline std::string wide_to_utf8(const std::wstring& w) {
    if (w.empty()) return "";
    int len = WideCharToMultiByte(CP_UTF8, 0, w.data(), (int)w.size(), nullptr, 0, nullptr, nullptr);
    if (len <= 0) return "";
    std::string out(len, '\0');
    WideCharToMultiByte(CP_UTF8, 0, w.data(), (int)w.size(), out.data(), len, nullptr, nullptr);
    return out;
}

inline std::string iso_now_utc() {
    std::time_t t = std::time(nullptr);
    std::tm tm_utc{};
    gmtime_s(&tm_utc, &t);
    char buf[32];
    std::strftime(buf, sizeof(buf), "%Y-%m-%dT%H:%M:%SZ", &tm_utc);
    return std::string(buf);
}

inline std::string make_id(const std::string& prefix) {
    static std::mutex rng_mtx;
    static std::mt19937_64 rng(
        (uint64_t)std::chrono::high_resolution_clock::now().time_since_epoch().count() ^
        (uint64_t)GetCurrentProcessId());
    uint64_t a = 0, b = 0;
    {
        std::lock_guard<std::mutex> lk(rng_mtx);
        a = rng();
        b = rng();
    }
    char hex[36];
    std::snprintf(hex, sizeof(hex), "%016llx%08llx", (unsigned long long)a, (unsigned long long)(b & 0xffffffffULL));
    return prefix + "_" + hex;
}

inline fs::path get_local_app_data_dir() {
    PWSTR raw = nullptr;
    if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_LocalAppData, 0, nullptr, &raw)) && raw) {
        fs::path p(raw);
        CoTaskMemFree(raw);
        return p / L"AI-Coding-Agent";
    }
    wchar_t tmp[MAX_PATH];
    GetTempPathW(MAX_PATH, tmp);
    return fs::path(tmp) / L"AI-Coding-Agent";
}

inline bool unpack_embedded_ui(const uint8_t* start, const uint8_t* end, const fs::path& dest_dir) {
    size_t total = (size_t)(end - start);
    if (total < 8 || std::memcmp(start, "UIPK", 4) != 0) return false;

    uint32_t count = 0;
    std::memcpy(&count, start + 4, 4);
    size_t off = 8;

    std::error_code ec;
    fs::create_directories(dest_dir, ec);

    for (uint32_t i = 0; i < count; ++i) {
        if (off + 4 > total) return false;
        uint32_t plen = 0;
        std::memcpy(&plen, start + off, 4);
        off += 4;
        if (off + plen + 4 > total) return false;
        std::string rel((const char*)(start + off), plen);
        off += plen;
        uint32_t dlen = 0;
        std::memcpy(&dlen, start + off, 4);
        off += 4;
        if (off + dlen > total) return false;

        fs::path out_file = dest_dir / utf8_to_wide(rel);
        fs::create_directories(out_file.parent_path(), ec);

        // Write if missing or size differs
        bool need_write = true;
        if (fs::exists(out_file, ec) && fs::file_size(out_file, ec) == dlen) {
            need_write = false;
        }
        if (need_write) {
            std::ofstream ofs(out_file, std::ios::binary | std::ios::trunc);
            if (ofs) {
                ofs.write((const char*)(start + off), dlen);
            }
        }
        off += dlen;
    }
    return true;
}

// ============================================================================
// Persistent SQLite3 Database Engine
// ============================================================================
class NativeSqliteStore {
public:
    sqlite3* db = nullptr;
    std::mutex mtx;
    std::string db_path_utf8;

    bool open(const fs::path& app_dir) {
        std::error_code ec;
        fs::create_directories(app_dir, ec);
        fs::path db_file = app_dir / L"agent.db";
        db_path_utf8 = wide_to_utf8(db_file.wstring());
        if (sqlite3_open(db_path_utf8.c_str(), &db) != SQLITE_OK) {
            return false;
        }
        const char* schema = R"SQL(
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                json_blob TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS recent_projects (
                path TEXT PRIMARY KEY,
                visited_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                project_path TEXT NOT NULL,
                mode TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                thinking TEXT,
                mode TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                tools_json TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tool_executions (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_json TEXT NOT NULL,
                result_summary TEXT NOT NULL,
                status TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                diff_text TEXT,
                file_path TEXT,
                created_at TEXT NOT NULL
            );
        )SQL";
        char* err = nullptr;
        sqlite3_exec(db, schema, nullptr, nullptr, &err);
        if (err) sqlite3_free(err);
        return true;
    }

    ~NativeSqliteStore() {
        if (db) sqlite3_close(db);
    }

    static json default_settings() {
        return {
            {"activeProvider", "ollama"},
            {"activeModel", "qwen2.5-coder:7b"},
            {"defaultMode", "autonomous"},
            {"openaiApiKey", ""},
            {"anthropicApiKey", ""},
            {"geminiApiKey", ""},
            {"openrouterApiKey", ""},
            {"ollamaBaseUrl", "http://127.0.0.1:11434"},
            {"customBaseUrl", ""},
            {"customApiKey", ""},
            {"requireShellApproval", true},
            {"requireDeleteConfirmation", true},
            {"restrictToWorkspace", true},
            {"maxAgentIterations", 15},
            {"defaultShell", "powershell"},
            {"editorFontSize", 13}
        };
    }

    json load_settings() {
        std::lock_guard<std::mutex> lk(mtx);
        json base = default_settings();
        sqlite3_stmt* st = nullptr;
        if (sqlite3_prepare_v2(db, "SELECT json_blob FROM settings WHERE id = 1", -1, &st, nullptr) == SQLITE_OK) {
            if (sqlite3_step(st) == SQLITE_ROW) {
                const char* txt = (const char*)sqlite3_column_text(st, 0);
                if (txt) {
                    json parsed = json::parse(txt, nullptr, false);
                    if (parsed.is_object()) {
                        for (auto it = parsed.begin(); it != parsed.end(); ++it) {
                            base[it.key()] = it.value();
                        }
                        sqlite3_finalize(st);
                        return base;
                    }
                }
            }
            sqlite3_finalize(st);
        }
        std::string dumped = base.dump();
        if (sqlite3_prepare_v2(db, "INSERT OR REPLACE INTO settings (id, json_blob) VALUES (1, ?1)", -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, dumped.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }
        return base;
    }

    json save_settings(const json& incoming) {
        json merged = default_settings();
        if (incoming.is_object()) {
            for (auto it = incoming.begin(); it != incoming.end(); ++it) {
                merged[it.key()] = it.value();
            }
        }
        std::lock_guard<std::mutex> lk(mtx);
        sqlite3_stmt* st = nullptr;
        std::string dumped = merged.dump();
        if (sqlite3_prepare_v2(db, "INSERT OR REPLACE INTO settings (id, json_blob) VALUES (1, ?1)", -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, dumped.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }
        return merged;
    }

    void record_recent_project(const std::string& path) {
        std::lock_guard<std::mutex> lk(mtx);
        sqlite3_stmt* st = nullptr;
        std::string now = iso_now_utc();
        if (sqlite3_prepare_v2(db, "INSERT OR REPLACE INTO recent_projects (path, visited_at) VALUES (?1, ?2)", -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, path.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 2, now.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }
    }

    json list_recent_projects() {
        std::lock_guard<std::mutex> lk(mtx);
        json arr = json::array();
        sqlite3_stmt* st = nullptr;
        if (sqlite3_prepare_v2(db, "SELECT path FROM recent_projects ORDER BY visited_at DESC LIMIT 12", -1, &st, nullptr) == SQLITE_OK) {
            while (sqlite3_step(st) == SQLITE_ROW) {
                const char* p = (const char*)sqlite3_column_text(st, 0);
                if (p) arr.push_back(std::string(p));
            }
            sqlite3_finalize(st);
        }
        return arr;
    }

    json list_sessions() {
        std::lock_guard<std::mutex> lk(mtx);
        json arr = json::array();
        const char* sql =
            "SELECT s.id, s.title, s.project_path, s.mode, s.updated_at, "
            "(SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) "
            "FROM sessions s ORDER BY s.updated_at DESC";
        sqlite3_stmt* st = nullptr;
        if (sqlite3_prepare_v2(db, sql, -1, &st, nullptr) == SQLITE_OK) {
            while (sqlite3_step(st) == SQLITE_ROW) {
                arr.push_back({
                    {"id", (const char*)sqlite3_column_text(st, 0)},
                    {"title", (const char*)sqlite3_column_text(st, 1)},
                    {"projectPath", (const char*)sqlite3_column_text(st, 2)},
                    {"mode", (const char*)sqlite3_column_text(st, 3)},
                    {"updatedAt", (const char*)sqlite3_column_text(st, 4)},
                    {"messageCount", sqlite3_column_int(st, 5)}
                });
            }
            sqlite3_finalize(st);
        }
        return arr;
    }

    json create_session(const std::string& title, const std::string& project_path, const std::string& mode) {
        std::lock_guard<std::mutex> lk(mtx);
        std::string id = make_id("ses");
        std::string now = iso_now_utc();
        sqlite3_stmt* st = nullptr;
        if (sqlite3_prepare_v2(db,
            "INSERT INTO sessions (id, title, project_path, mode, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, id.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 2, title.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 3, project_path.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 4, mode.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 5, now.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 6, now.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }
        return {
            {"id", id},
            {"title", title},
            {"projectPath", project_path},
            {"mode", mode},
            {"createdAt", now},
            {"updatedAt", now},
            {"messages", json::array()}
        };
    }

    json load_session(const std::string& session_id) {
        std::lock_guard<std::mutex> lk(mtx);
        sqlite3_stmt* st = nullptr;
        json detail = nullptr;
        if (sqlite3_prepare_v2(db,
            "SELECT id, title, project_path, mode, created_at, updated_at FROM sessions WHERE id = ?1",
            -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, session_id.c_str(), -1, SQLITE_TRANSIENT);
            if (sqlite3_step(st) == SQLITE_ROW) {
                detail = {
                    {"id", (const char*)sqlite3_column_text(st, 0)},
                    {"title", (const char*)sqlite3_column_text(st, 1)},
                    {"projectPath", (const char*)sqlite3_column_text(st, 2)},
                    {"mode", (const char*)sqlite3_column_text(st, 3)},
                    {"createdAt", (const char*)sqlite3_column_text(st, 4)},
                    {"updatedAt", (const char*)sqlite3_column_text(st, 5)},
                    {"messages", json::array()}
                };
            }
            sqlite3_finalize(st);
        }
        if (detail.is_null()) return nullptr;

        if (sqlite3_prepare_v2(db,
            "SELECT id, session_id, role, content, thinking, mode, provider, model, tools_json, created_at "
            "FROM messages WHERE session_id = ?1 ORDER BY created_at ASC",
            -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, session_id.c_str(), -1, SQLITE_TRANSIENT);
            while (sqlite3_step(st) == SQLITE_ROW) {
                const char* th = (const char*)sqlite3_column_text(st, 4);
                const char* prov = (const char*)sqlite3_column_text(st, 6);
                const char* mod = (const char*)sqlite3_column_text(st, 7);
                const char* tj = (const char*)sqlite3_column_text(st, 8);
                json tools = tj ? json::parse(tj, nullptr, false) : json::array();
                if (!tools.is_array()) tools = json::array();
                json m = {
                    {"id", (const char*)sqlite3_column_text(st, 0)},
                    {"sessionId", (const char*)sqlite3_column_text(st, 1)},
                    {"role", (const char*)sqlite3_column_text(st, 2)},
                    {"content", (const char*)sqlite3_column_text(st, 3)},
                    {"mode", (const char*)sqlite3_column_text(st, 5)},
                    {"tools", tools},
                    {"createdAt", (const char*)sqlite3_column_text(st, 9)}
                };
                if (th && th[0]) m["thinking"] = std::string(th);
                if (prov && prov[0]) m["provider"] = std::string(prov);
                if (mod && mod[0]) m["model"] = std::string(mod);
                detail["messages"].push_back(m);
            }
            sqlite3_finalize(st);
        }
        return detail;
    }

    json ensure_default_session(const std::string& project_path) {
        json list = list_sessions();
        if (!list.empty()) {
            json loaded = load_session(list[0]["id"].get<std::string>());
            if (!loaded.is_null()) return loaded;
        }
        return create_session("Initial Coding Session", project_path, "autonomous");
    }

    void append_message(const json& msg) {
        std::lock_guard<std::mutex> lk(mtx);
        sqlite3_stmt* st = nullptr;
        std::string id = msg.value("id", make_id("msg"));
        std::string sid = msg.value("sessionId", "");
        std::string role = msg.value("role", "user");
        std::string content = msg.value("content", "");
        std::string thinking = msg.value("thinking", "");
        std::string mode = msg.value("mode", "autonomous");
        std::string provider = msg.value("provider", "");
        std::string model = msg.value("model", "");
        std::string tools_str = msg.contains("tools") ? msg["tools"].dump() : "[]";
        std::string created_at = msg.value("createdAt", iso_now_utc());

        if (sqlite3_prepare_v2(db,
            "INSERT INTO messages (id, session_id, role, content, thinking, mode, provider, model, tools_json, created_at) "
            "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, id.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 2, sid.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 3, role.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 4, content.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 5, thinking.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 6, mode.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 7, provider.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 8, model.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 9, tools_str.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 10, created_at.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }

        if (role == "user") {
            std::string short_title = content.substr(0, 48);
            if (sqlite3_prepare_v2(db,
                "UPDATE sessions SET updated_at = ?1, title = CASE WHEN title IN ('Initial Coding Session', 'New Agent Session') THEN ?2 ELSE title END WHERE id = ?3",
                -1, &st, nullptr) == SQLITE_OK) {
                sqlite3_bind_text(st, 1, created_at.c_str(), -1, SQLITE_TRANSIENT);
                sqlite3_bind_text(st, 2, short_title.c_str(), -1, SQLITE_TRANSIENT);
                sqlite3_bind_text(st, 3, sid.c_str(), -1, SQLITE_TRANSIENT);
                sqlite3_step(st);
                sqlite3_finalize(st);
            }
        } else {
            if (sqlite3_prepare_v2(db, "UPDATE sessions SET updated_at = ?1 WHERE id = ?2", -1, &st, nullptr) == SQLITE_OK) {
                sqlite3_bind_text(st, 1, created_at.c_str(), -1, SQLITE_TRANSIENT);
                sqlite3_bind_text(st, 2, sid.c_str(), -1, SQLITE_TRANSIENT);
                sqlite3_step(st);
                sqlite3_finalize(st);
            }
        }
    }

    void delete_session(const std::string& sid) {
        std::lock_guard<std::mutex> lk(mtx);
        sqlite3_stmt* st = nullptr;
        if (sqlite3_prepare_v2(db, "DELETE FROM messages WHERE session_id = ?1", -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, sid.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }
        if (sqlite3_prepare_v2(db, "DELETE FROM sessions WHERE id = ?1", -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, sid.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }
    }

    void record_tool_execution(const json& rec) {
        std::lock_guard<std::mutex> lk(mtx);
        sqlite3_stmt* st = nullptr;
        std::string id = rec.value("id", make_id("tool"));
        std::string sid = rec.value("sessionId", "");
        std::string tname = rec.value("toolName", "");
        std::string args = rec.value("argsJson", "{}");
        std::string sum = rec.value("resultSummary", "");
        std::string status = rec.value("status", "ok");
        int64_t dur = rec.value("durationMs", 0);
        std::string diff = rec.value("diffText", "");
        std::string fpath = rec.value("filePath", "");
        std::string cat = rec.value("createdAt", iso_now_utc());

        if (sqlite3_prepare_v2(db,
            "INSERT OR REPLACE INTO tool_executions "
            "(id, session_id, tool_name, args_json, result_summary, status, duration_ms, diff_text, file_path, created_at) "
            "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_text(st, 1, id.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 2, sid.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 3, tname.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 4, args.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 5, sum.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 6, status.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_int64(st, 7, dur);
            sqlite3_bind_text(st, 8, diff.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 9, fpath.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_bind_text(st, 10, cat.c_str(), -1, SQLITE_TRANSIENT);
            sqlite3_step(st);
            sqlite3_finalize(st);
        }
    }

    json list_tool_history(int limit = 150) {
        std::lock_guard<std::mutex> lk(mtx);
        json arr = json::array();
        sqlite3_stmt* st = nullptr;
        if (sqlite3_prepare_v2(db,
            "SELECT id, session_id, tool_name, args_json, result_summary, status, duration_ms, diff_text, file_path, created_at "
            "FROM tool_executions ORDER BY created_at DESC LIMIT ?1",
            -1, &st, nullptr) == SQLITE_OK) {
            sqlite3_bind_int(st, 1, limit);
            while (sqlite3_step(st) == SQLITE_ROW) {
                const char* dt = (const char*)sqlite3_column_text(st, 7);
                const char* fp = (const char*)sqlite3_column_text(st, 8);
                json item = {
                    {"id", (const char*)sqlite3_column_text(st, 0)},
                    {"sessionId", (const char*)sqlite3_column_text(st, 1)},
                    {"toolName", (const char*)sqlite3_column_text(st, 2)},
                    {"argsJson", (const char*)sqlite3_column_text(st, 3)},
                    {"resultSummary", (const char*)sqlite3_column_text(st, 4)},
                    {"status", (const char*)sqlite3_column_text(st, 5)},
                    {"durationMs", sqlite3_column_int64(st, 6)},
                    {"createdAt", (const char*)sqlite3_column_text(st, 9)}
                };
                if (dt && dt[0]) item["diffText"] = std::string(dt);
                if (fp && fp[0]) item["filePath"] = std::string(fp);
                arr.push_back(item);
            }
            sqlite3_finalize(st);
        }
        return arr;
    }
};

// ============================================================================
// Windows Drives, Native Dialogs, Filesystem Tree & Watcher, Silent Subprocess
// ============================================================================
inline json query_windows_drives() {
    json arr = json::array();
    DWORD mask = GetLogicalDrives();
    for (int i = 0; i < 26; ++i) {
        if (mask & (1u << i)) {
            wchar_t mount[4] = { (wchar_t)(L'A' + i), L':', L'\\', L'\0' };
            UINT dtype = GetDriveTypeW(mount);
            const char* dtype_str = "Drive";
            switch (dtype) {
                case DRIVE_REMOVABLE: dtype_str = "Removable"; break;
                case DRIVE_FIXED: dtype_str = "Local Disk"; break;
                case DRIVE_REMOTE: dtype_str = "Network"; break;
                case DRIVE_CDROM: dtype_str = "Optical"; break;
                case DRIVE_RAMDISK: dtype_str = "RAM Disk"; break;
            }
            ULARGE_INTEGER avail{}, total{}, free_b{};
            GetDiskFreeSpaceExW(mount, &avail, &total, &free_b);
            char letter_s[3] = { (char)('A' + i), ':', '\0' };
            arr.push_back({
                {"letter", std::string(letter_s)},
                {"mountPath", wide_to_utf8(mount)},
                {"driveType", dtype_str},
                {"totalGb", (double)total.QuadPart / 1073741824.0},
                {"freeGb", (double)avail.QuadPart / 1073741824.0}
            });
        }
    }
    return arr;
}

inline std::string pick_windows_folder_dialog(HWND owner) {
    IFileOpenDialog* dlg = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&dlg));
    if (FAILED(hr) || !dlg) return "";
    DWORD opts = 0;
    if (SUCCEEDED(dlg->GetOptions(&opts))) {
        dlg->SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST);
    }
    dlg->SetTitle(L"Open Windows Project Workspace");
    std::string chosen;
    if (SUCCEEDED(dlg->Show(owner))) {
        IShellItem* item = nullptr;
        if (SUCCEEDED(dlg->GetResult(&item)) && item) {
            PWSTR psz = nullptr;
            if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &psz)) && psz) {
                chosen = wide_to_utf8(psz);
                CoTaskMemFree(psz);
            }
            item->Release();
        }
    }
    dlg->Release();
    return chosen;
}

inline bool is_ignored_dir_name(const std::string& name) {
    static const std::set<std::string> ign = {
        ".git", "node_modules", "target", "ui-dist", ".next", ".venv", "venv", "__pycache__", ".vs", "obj"
    };
    return ign.count(name) > 0;
}

inline void walk_project_dir(const fs::path& base, const fs::path& curr, json& out, size_t max_items) {
    if (out.size() >= max_items) return;
    std::error_code ec;
    std::vector<fs::directory_entry> items;
    for (auto it = fs::directory_iterator(curr, fs::directory_options::skip_permission_denied, ec);
         !ec && it != fs::directory_iterator(); it.increment(ec)) {
        items.push_back(*it);
    }
    std::sort(items.begin(), items.end(), [](const fs::directory_entry& a, const fs::directory_entry& b) {
        return a.path().filename().wstring() < b.path().filename().wstring();
    });

    for (const auto& entry : items) {
        if (out.size() >= max_items) break;
        std::string name = wide_to_utf8(entry.path().filename().wstring());
        bool is_dir = entry.is_directory(ec);
        if (is_dir && is_ignored_dir_name(name)) continue;

        fs::path rel = fs::relative(entry.path(), base, ec);
        std::string rel_str = wide_to_utf8(rel.wstring());
        std::replace(rel_str.begin(), rel_str.end(), '\\', '/');

        uint64_t sz = is_dir ? 0 : (uint64_t)entry.file_size(ec);
        DWORD attrs = GetFileAttributesW(entry.path().c_str());
        bool ro = (attrs != INVALID_FILE_ATTRIBUTES) && ((attrs & FILE_ATTRIBUTE_READONLY) != 0);

        out.push_back({
            {"name", name},
            {"path", wide_to_utf8(entry.path().wstring())},
            {"relPath", rel_str},
            {"isDir", is_dir},
            {"size", sz},
            {"readonly", ro}
        });

        if (is_dir) {
            walk_project_dir(base, entry.path(), out, max_items);
        }
    }
}

inline json scan_project_files(const fs::path& root, size_t max_items = 2000) {
    json out = json::array();
    walk_project_dir(root, root, out, max_items);
    return out;
}

inline bool resolve_safe_path(const fs::path& root, const std::string& candidate_utf8, bool restrict_ws, fs::path& out_path, std::string& err_out) {
    fs::path raw = utf8_to_wide(candidate_utf8);
    fs::path joined = raw.is_absolute() ? raw : (root / raw);
    std::error_code ec;
    fs::path canon_target = fs::weakly_canonical(joined, ec);
    if (ec) canon_target = joined;

    if (restrict_ws) {
        fs::path canon_root = fs::weakly_canonical(root, ec);
        if (ec) canon_root = root;
        std::wstring r_str = canon_root.wstring();
        std::wstring t_str = canon_target.wstring();
        std::transform(r_str.begin(), r_str.end(), r_str.begin(), ::towlower);
        std::transform(t_str.begin(), t_str.end(), t_str.begin(), ::towlower);
        if (t_str.rfind(r_str, 0) != 0) {
            err_out = "Workspace Sandbox Violation: Path '" + wide_to_utf8(canon_target.wstring()) +
                      "' is outside active workspace '" + wide_to_utf8(canon_root.wstring()) + "'";
            return false;
        }
    }
    out_path = canon_target;
    return true;
}

inline bool read_utf8_file(const fs::path& p, std::string& content_out) {
    std::ifstream ifs(p, std::ios::binary);
    if (!ifs) return false;
    std::ostringstream ss;
    ss << ifs.rdbuf();
    content_out = ss.str();
    return true;
}

inline bool write_utf8_file(const fs::path& p, const std::string& content) {
    std::error_code ec;
    if (p.has_parent_path()) fs::create_directories(p.parent_path(), ec);
    std::ofstream ofs(p, std::ios::binary | std::ios::trunc);
    if (!ofs) return false;
    ofs.write(content.data(), (std::streamsize)content.size());
    return ofs.good();
}

inline std::string build_unified_diff(const std::string& rel_path, const std::string& old_txt, const std::string& new_txt) {
    auto split_lines = [](const std::string& s) {
        std::vector<std::string> lines;
        std::istringstream iss(s);
        std::string ln;
        while (std::getline(iss, ln)) {
            if (!ln.empty() && ln.back() == '\r') ln.pop_back();
            lines.push_back(ln);
        }
        return lines;
    };
    auto old_l = split_lines(old_txt);
    auto new_l = split_lines(new_txt);
    size_t pre = 0;
    while (pre < old_l.size() && pre < new_l.size() && old_l[pre] == new_l[pre]) ++pre;
    size_t oe = old_l.size(), ne = new_l.size();
    while (oe > pre && ne > pre && old_l[oe - 1] == new_l[ne - 1]) { --oe; --ne; }
    if (pre == oe && pre == ne) return "";

    std::ostringstream out;
    out << "--- a/" << rel_path << "\n+++ b/" << rel_path << "\n@@ -"
        << (pre + 1) << "," << (oe - pre) << " +" << (pre + 1) << "," << (ne - pre) << " @@\n";
    for (size_t i = pre; i < oe; ++i) out << "-" << old_l[i] << "\n";
    for (size_t i = pre; i < ne; ++i) out << "+" << new_l[i] << "\n";
    return out.str();
}

inline json search_in_workspace(const fs::path& root, const std::string& query, bool case_sensitive, size_t max_hits = 100) {
    json hits = json::array();
    if (query.empty()) return hits;
    json tree = scan_project_files(root, 2000);
    std::string needle = query;
    if (!case_sensitive) std::transform(needle.begin(), needle.end(), needle.begin(), ::tolower);

    for (const auto& entry : tree) {
        if (entry["isDir"].get<bool>() || entry["size"].get<uint64_t>() > 1048576) continue;
        std::string content;
        if (!read_utf8_file(utf8_to_wide(entry["path"].get<std::string>()), content)) continue;
        std::istringstream iss(content);
        std::string line;
        size_t lno = 0;
        while (std::getline(iss, line)) {
            ++lno;
            std::string hay = line;
            if (!case_sensitive) std::transform(hay.begin(), hay.end(), hay.begin(), ::tolower);
            size_t pos = hay.find(needle);
            if (pos != std::string::npos) {
                hits.push_back({
                    {"file", entry["path"]},
                    {"relPath", entry["relPath"]},
                    {"line", lno},
                    {"column", pos + 1},
                    {"preview", line.substr(0, 200)}
                });
                if (hits.size() >= max_hits) return hits;
            }
        }
    }
    return hits;
}

// Real-time Windows ReadDirectoryChangesW Directory Watcher
class Win32DirectoryWatcher {
    HANDLE dir_handle = INVALID_HANDLE_VALUE;
    std::thread worker;
    std::atomic<bool> active{false};
    std::mutex mtx;

public:
    ~Win32DirectoryWatcher() { stop(); }

    void stop() {
        active.store(false);
        {
            std::lock_guard<std::mutex> lk(mtx);
            if (dir_handle != INVALID_HANDLE_VALUE) {
                CancelIoEx(dir_handle, nullptr);
                CloseHandle(dir_handle);
                dir_handle = INVALID_HANDLE_VALUE;
            }
        }
        if (worker.joinable()) worker.join();
    }

    void watch(const fs::path& root, std::function<void(const std::string&)> on_change) {
        stop();
        HANDLE h = CreateFileW(
            root.c_str(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            nullptr,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            nullptr);
        if (h == INVALID_HANDLE_VALUE) return;

        {
            std::lock_guard<std::mutex> lk(mtx);
            dir_handle = h;
        }
        active.store(true);
        worker = std::thread([this, h, on_change]() {
            alignas(DWORD) uint8_t buffer[8192];
            DWORD bytes = 0;
            while (active.load()) {
                BOOL ok = ReadDirectoryChangesW(
                    h, buffer, sizeof(buffer), TRUE,
                    FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                    &bytes, nullptr, nullptr);
                if (!ok || !active.load() || bytes == 0) break;

                auto* info = reinterpret_cast<FILE_NOTIFY_INFORMATION*>(buffer);
                std::wstring rel(info->FileName, info->FileNameLength / sizeof(WCHAR));
                std::string rel_u8 = wide_to_utf8(rel);
                if (rel_u8.find(".git") == std::string::npos &&
                    rel_u8.find("node_modules") == std::string::npos) {
                    on_change(rel_u8);
                }
            }
        });
    }
};

// Silent Windows Process Runner (CREATE_NO_WINDOW)
struct SubprocessResult {
    int exit_code = -1;
    std::string stdout_text;
    std::string stderr_text;
};

inline SubprocessResult run_process_silent(const std::wstring& cmdline, const fs::path& cwd, DWORD timeout_ms = 90000) {
    SubprocessResult res;
    SECURITY_ATTRIBUTES sa{sizeof(SECURITY_ATTRIBUTES), nullptr, TRUE};
    HANDLE out_r = nullptr, out_w = nullptr;
    HANDLE err_r = nullptr, err_w = nullptr;
    if (!CreatePipe(&out_r, &out_w, &sa, 0)) return res;
    SetHandleInformation(out_r, HANDLE_FLAG_INHERIT, 0);
    if (!CreatePipe(&err_r, &err_w, &sa, 0)) {
        CloseHandle(out_r); CloseHandle(out_w);
        return res;
    }
    SetHandleInformation(err_r, HANDLE_FLAG_INHERIT, 0);

    STARTUPINFOW si{};
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
    si.wShowWindow = SW_HIDE;
    si.hStdOutput = out_w;
    si.hStdError = err_w;
    si.hStdInput = GetStdHandle(STD_INPUT_HANDLE);

    PROCESS_INFORMATION pi{};
    std::wstring mutable_cmd = cmdline;
    std::wstring cwd_w = cwd.wstring();

    BOOL started = CreateProcessW(
        nullptr, mutable_cmd.data(), nullptr, nullptr, TRUE,
        CREATE_NO_WINDOW, nullptr, cwd_w.empty() ? nullptr : cwd_w.c_str(), &si, &pi);

    CloseHandle(out_w);
    CloseHandle(err_w);

    if (!started) {
        CloseHandle(out_r);
        CloseHandle(err_r);
        res.stderr_text = "Failed to launch process.";
        return res;
    }

    auto read_pipe = [](HANDLE h, std::string& target) {
        char buf[4096];
        DWORD n = 0;
        while (ReadFile(h, buf, sizeof(buf), &n, nullptr) && n > 0) {
            if (target.size() < 65536) target.append(buf, n);
        }
    };
    std::thread t_out(read_pipe, out_r, std::ref(res.stdout_text));
    std::thread t_err(read_pipe, err_r, std::ref(res.stderr_text));

    if (WaitForSingleObject(pi.hProcess, timeout_ms) == WAIT_TIMEOUT) {
        TerminateProcess(pi.hProcess, 124);
    }
    DWORD code = 0;
    GetExitCodeProcess(pi.hProcess, &code);
    res.exit_code = (int)code;

    t_out.join();
    t_err.join();
    CloseHandle(out_r);
    CloseHandle(err_r);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return res;
}

inline SubprocessResult run_git_cmd(const fs::path& root, const std::string& args_utf8) {
    std::wstring cmd = L"git.exe " + utf8_to_wide(args_utf8);
    return run_process_silent(cmd, root, 25000);
}

inline json query_git_status(const fs::path& root) {
    auto check = run_git_cmd(root, "rev-parse --is-inside-work-tree");
    if (check.exit_code != 0) {
        return {
            {"isRepo", false},
            {"branch", ""},
            {"branches", json::array()},
            {"files", json::array()},
            {"commits", json::array()}
        };
    }
    auto br = run_git_cmd(root, "rev-parse --abbrev-ref HEAD");
    std::string branch = br.stdout_text;
    while (!branch.empty() && (branch.back() == '\n' || branch.back() == '\r' || branch.back() == ' ')) branch.pop_back();

    json branches = json::array();
    auto br_list = run_git_cmd(root, "branch --format=%(refname:short)");
    {
        std::istringstream iss(br_list.stdout_text);
        std::string ln;
        while (std::getline(iss, ln)) {
            if (!ln.empty() && ln.back() == '\r') ln.pop_back();
            if (!ln.empty()) branches.push_back(ln);
        }
    }

    json files = json::array();
    auto st = run_git_cmd(root, "status --porcelain=v1");
    {
        std::istringstream iss(st.stdout_text);
        std::string ln;
        while (std::getline(iss, ln)) {
            if (!ln.empty() && ln.back() == '\r') ln.pop_back();
            if (ln.size() >= 3) {
                char idx = ln[0];
                char wt = ln[1];
                std::string p = ln.substr(3);
                bool staged = (idx != ' ' && idx != '?');
                std::string scode = (idx == '?') ? "U" : std::string(1, staged ? idx : wt);
                files.push_back({
                    {"path", p},
                    {"status", scode},
                    {"staged", staged}
                });
            }
        }
    }

    json commits = json::array();
    auto lg = run_git_cmd(root, "log -n 12 \"--pretty=format:%h\t%s\t%an\t%ar\"");
    {
        std::istringstream iss(lg.stdout_text);
        std::string ln;
        while (std::getline(iss, ln)) {
            if (!ln.empty() && ln.back() == '\r') ln.pop_back();
            size_t t1 = ln.find('\t');
            size_t t2 = (t1 == std::string::npos) ? std::string::npos : ln.find('\t', t1 + 1);
            size_t t3 = (t2 == std::string::npos) ? std::string::npos : ln.find('\t', t2 + 1);
            if (t3 != std::string::npos) {
                commits.push_back({
                    {"hash", ln.substr(0, t1)},
                    {"subject", ln.substr(t1 + 1, t2 - t1 - 1)},
                    {"author", ln.substr(t2 + 1, t3 - t2 - 1)},
                    {"date", ln.substr(t3 + 1)}
                });
            }
        }
    }

    return {
        {"isRepo", true},
        {"branch", branch},
        {"branches", branches},
        {"files", files},
        {"commits", commits}
    };
}
