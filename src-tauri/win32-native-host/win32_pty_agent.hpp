#pragma once
#include "win32_sqlite_git.hpp"

using EventEmitterFn = std::function<void(const std::string& event_name, const json& payload)>;

// ============================================================================
// Genuine Windows ConPTY Multi-Tab Terminal Manager (HPCON)
// ============================================================================
class Win32ConPtyManager {
    struct PtySession {
        std::string pty_id;
        std::string shell;
        std::string cwd;
        HPCON hpc = nullptr;
        HANDLE pipe_in_write = nullptr;
        HANDLE pipe_out_read = nullptr;
        HANDLE h_process = nullptr;
        HANDLE h_thread = nullptr;
        std::thread reader_thread;
        std::atomic<bool> alive{true};
    };

    std::mutex mtx;
    std::map<std::string, std::shared_ptr<PtySession>> sessions;

public:
    ~Win32ConPtyManager() {
        kill_all();
    }

    void kill_all() {
        std::vector<std::shared_ptr<PtySession>> copy;
        {
            std::lock_guard<std::mutex> lk(mtx);
            for (auto& kv : sessions) copy.push_back(kv.second);
            sessions.clear();
        }
        for (auto& s : copy) {
            shutdown_session(s);
        }
    }

    static void shutdown_session(const std::shared_ptr<PtySession>& s) {
        if (!s) return;
        s->alive.store(false);
        if (s->pipe_in_write) {
            CloseHandle(s->pipe_in_write);
            s->pipe_in_write = nullptr;
        }
        if (s->hpc) {
            ClosePseudoConsole(s->hpc);
            s->hpc = nullptr;
        }
        if (s->h_process) {
            TerminateProcess(s->h_process, 0);
            CloseHandle(s->h_process);
            s->h_process = nullptr;
        }
        if (s->h_thread) {
            CloseHandle(s->h_thread);
            s->h_thread = nullptr;
        }
        if (s->pipe_out_read) {
            CancelIoEx(s->pipe_out_read, nullptr);
            CloseHandle(s->pipe_out_read);
            s->pipe_out_read = nullptr;
        }
        if (s->reader_thread.joinable() && s->reader_thread.get_id() != std::this_thread::get_id()) {
            s->reader_thread.join();
        }
    }

    json spawn(const std::string& shell, const std::string& cwd_utf8, SHORT cols, SHORT rows,
               const json& custom_env, EventEmitterFn emit_fn) {
        HANDLE in_read = nullptr, in_write = nullptr;
        HANDLE out_read = nullptr, out_write = nullptr;

        if (!CreatePipe(&in_read, &in_write, nullptr, 0) ||
            !CreatePipe(&out_read, &out_write, nullptr, 0)) {
            if (in_read) CloseHandle(in_read);
            if (in_write) CloseHandle(in_write);
            throw std::runtime_error("ConPTY CreatePipe failed");
        }

        COORD coord{ std::max<SHORT>(cols, 30), std::max<SHORT>(rows, 10) };
        HPCON hpc = nullptr;
        HRESULT hr = CreatePseudoConsole(coord, in_read, out_write, 0, &hpc);
        CloseHandle(in_read);
        CloseHandle(out_write);
        if (FAILED(hr) || !hpc) {
            CloseHandle(in_write);
            CloseHandle(out_read);
            throw std::runtime_error("CreatePseudoConsole failed");
        }

        SIZE_T attr_size = 0;
        InitializeProcThreadAttributeList(nullptr, 1, 0, &attr_size);
        std::vector<uint8_t> attr_buf(attr_size);
        auto attr_list = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(attr_buf.data());
        if (!InitializeProcThreadAttributeList(attr_list, 1, 0, &attr_size) ||
            !UpdateProcThreadAttribute(attr_list, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, hpc, sizeof(HPCON), nullptr, nullptr)) {
            ClosePseudoConsole(hpc);
            CloseHandle(in_write);
            CloseHandle(out_read);
            throw std::runtime_error("InitializeProcThreadAttributeList for ConPTY failed");
        }

        // Build environment block inheriting current env + TERM=xterm-256color + custom_env
        std::map<std::wstring, std::wstring> env_map;
        LPWCH raw_env = GetEnvironmentStringsW();
        if (raw_env) {
            const wchar_t* p = raw_env;
            while (*p) {
                std::wstring entry(p);
                p += entry.size() + 1;
                size_t eq = entry.find(L'=', 1);
                if (eq != std::wstring::npos) {
                    env_map[entry.substr(0, eq)] = entry.substr(eq + 1);
                }
            }
            FreeEnvironmentStringsW(raw_env);
        }
        env_map[L"TERM"] = L"xterm-256color";
        if (custom_env.is_object()) {
            for (auto it = custom_env.begin(); it != custom_env.end(); ++it) {
                if (it.value().is_string()) {
                    env_map[utf8_to_wide(it.key())] = utf8_to_wide(it.value().get<std::string>());
                }
            }
        }
        std::wstring env_block;
        for (const auto& kv : env_map) {
            env_block += kv.first + L"=" + kv.second + L'\0';
        }
        env_block += L'\0';

        STARTUPINFOEXW si{};
        si.StartupInfo.cb = sizeof(STARTUPINFOEXW);
        si.lpAttributeList = attr_list;

        std::wstring cmdline = (shell == "cmd") ? L"cmd.exe" : L"powershell.exe -NoLogo";
        std::wstring cwd_w = utf8_to_wide(cwd_utf8);
        PROCESS_INFORMATION pi{};

        BOOL ok = CreateProcessW(
            nullptr,
            cmdline.data(),
            nullptr,
            nullptr,
            FALSE,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env_block.data(),
            cwd_w.empty() ? nullptr : cwd_w.c_str(),
            &si.StartupInfo,
            &pi);

        DeleteProcThreadAttributeList(attr_list);

        if (!ok) {
            ClosePseudoConsole(hpc);
            CloseHandle(in_write);
            CloseHandle(out_read);
            throw std::runtime_error("Failed to launch shell inside Windows ConPTY");
        }

        auto sess = std::make_shared<PtySession>();
        sess->pty_id = make_id("pty");
        sess->shell = shell;
        sess->cwd = cwd_utf8;
        sess->hpc = hpc;
        sess->pipe_in_write = in_write;
        sess->pipe_out_read = out_read;
        sess->h_process = pi.hProcess;
        sess->h_thread = pi.hThread;

        {
            std::lock_guard<std::mutex> lk(mtx);
            sessions[sess->pty_id] = sess;
        }

        sess->reader_thread = std::thread([sess, emit_fn]() {
            char buf[4096];
            DWORD n = 0;
            while (sess->alive.load() && sess->pipe_out_read &&
                   ReadFile(sess->pipe_out_read, buf, sizeof(buf), &n, nullptr) && n > 0) {
                std::string chunk(buf, n);
                emit_fn("pty://data", {
                    {"ptyId", sess->pty_id},
                    {"data", chunk}
                });
            }
            DWORD exit_code = 0;
            if (sess->h_process) {
                WaitForSingleObject(sess->h_process, 500);
                GetExitCodeProcess(sess->h_process, &exit_code);
            }
            sess->alive.store(false);
            emit_fn("pty://exit", {
                {"ptyId", sess->pty_id},
                {"exitCode", (int)exit_code}
            });
        });

        return {
            {"ptyId", sess->pty_id},
            {"shell", sess->shell},
            {"cwd", sess->cwd}
        };
    }

    void write(const std::string& pty_id, const std::string& data) {
        std::shared_ptr<PtySession> s;
        {
            std::lock_guard<std::mutex> lk(mtx);
            auto it = sessions.find(pty_id);
            if (it != sessions.end()) s = it->second;
        }
        if (s && s->pipe_in_write && !data.empty()) {
            DWORD written = 0;
            WriteFile(s->pipe_in_write, data.data(), (DWORD)data.size(), &written, nullptr);
        }
    }

    void resize(const std::string& pty_id, SHORT cols, SHORT rows) {
        std::shared_ptr<PtySession> s;
        {
            std::lock_guard<std::mutex> lk(mtx);
            auto it = sessions.find(pty_id);
            if (it != sessions.end()) s = it->second;
        }
        if (s && s->hpc) {
            COORD c{ std::max<SHORT>(cols, 20), std::max<SHORT>(rows, 5) };
            ResizePseudoConsole(s->hpc, c);
        }
    }

    void kill(const std::string& pty_id) {
        std::shared_ptr<PtySession> s;
        {
            std::lock_guard<std::mutex> lk(mtx);
            auto it = sessions.find(pty_id);
            if (it != sessions.end()) {
                s = it->second;
                sessions.erase(it);
            }
        }
        if (s) shutdown_session(s);
    }
};

// ============================================================================
// Native WinHTTP HTTPS / HTTP Client (Zero External TLS DLLs)
// ============================================================================
struct HttpResponse {
    int status = 0;
    std::string body;
    std::string error;
};

inline HttpResponse winhttp_request(const std::string& method,
                                    const std::string& url_utf8,
                                    const std::vector<std::pair<std::string, std::string>>& headers,
                                    const std::string& body_utf8,
                                    DWORD timeout_ms = 120000) {
    HttpResponse out;
    std::wstring wurl = utf8_to_wide(url_utf8);

    URL_COMPONENTS uc{};
    uc.dwStructSize = sizeof(uc);
    wchar_t host_buf[256] = {};
    wchar_t path_buf[2048] = {};
    uc.lpszHostName = host_buf;
    uc.dwHostNameLength = 255;
    uc.lpszUrlPath = path_buf;
    uc.dwUrlPathLength = 2047;

    if (!WinHttpCrackUrl(wurl.c_str(), (DWORD)wurl.size(), 0, &uc)) {
        out.error = "Invalid URL: " + url_utf8;
        return out;
    }

    HINTERNET h_sess = WinHttpOpen(L"AI-Coding-Agent-Windows/1.0",
                                   WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                                   WINHTTP_NO_PROXY_NAME,
                                   WINHTTP_NO_PROXY_BYPASS, 0);
    if (!h_sess) {
        out.error = "WinHttpOpen failed";
        return out;
    }
    WinHttpSetTimeouts(h_sess, 10000, 15000, timeout_ms, timeout_ms);

    HINTERNET h_conn = WinHttpConnect(h_sess, host_buf, uc.nPort, 0);
    if (!h_conn) {
        WinHttpCloseHandle(h_sess);
        out.error = "WinHttpConnect failed for " + wide_to_utf8(host_buf);
        return out;
    }

    DWORD flags = (uc.nScheme == INTERNET_SCHEME_HTTPS) ? WINHTTP_FLAG_SECURE : 0;
    std::wstring wmethod = utf8_to_wide(method);
    HINTERNET h_req = WinHttpOpenRequest(h_conn, wmethod.c_str(), path_buf,
                                         nullptr, WINHTTP_NO_REFERER,
                                         WINHTTP_DEFAULT_ACCEPT_TYPES, flags);
    if (!h_req) {
        WinHttpCloseHandle(h_conn);
        WinHttpCloseHandle(h_sess);
        out.error = "WinHttpOpenRequest failed";
        return out;
    }

    for (const auto& kv : headers) {
        std::wstring hline = utf8_to_wide(kv.first + ": " + kv.second);
        WinHttpAddRequestHeaders(h_req, hline.c_str(), (DWORD)hline.size(), WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE);
    }

    BOOL sent = WinHttpSendRequest(
        h_req,
        WINHTTP_NO_ADDITIONAL_HEADERS, 0,
        body_utf8.empty() ? WINHTTP_NO_REQUEST_DATA : (LPVOID)body_utf8.data(),
        (DWORD)body_utf8.size(),
        (DWORD)body_utf8.size(),
        0);

    if (!sent || !WinHttpReceiveResponse(h_req, nullptr)) {
        DWORD err = GetLastError();
        WinHttpCloseHandle(h_req);
        WinHttpCloseHandle(h_conn);
        WinHttpCloseHandle(h_sess);
        out.error = "HTTP connection failed (WinHTTP code " + std::to_string(err) + ") targeting " + url_utf8;
        return out;
    }

    DWORD status_code = 0;
    DWORD sz = sizeof(status_code);
    WinHttpQueryHeaders(h_req, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                        WINHTTP_HEADER_NAME_BY_INDEX, &status_code, &sz, WINHTTP_NO_HEADER_INDEX);
    out.status = (int)status_code;

    DWORD avail = 0;
    while (WinHttpQueryDataAvailable(h_req, &avail) && avail > 0) {
        std::string chunk(avail, '\0');
        DWORD read_n = 0;
        if (WinHttpReadData(h_req, chunk.data(), avail, &read_n) && read_n > 0) {
            chunk.resize(read_n);
            out.body += chunk;
        } else {
            break;
        }
    }

    WinHttpCloseHandle(h_req);
    WinHttpCloseHandle(h_conn);
    WinHttpCloseHandle(h_sess);
    return out;
}

// ============================================================================
// Interactive Permission Gate & Emergency Stop
// ============================================================================
class Win32PermissionGate {
    struct PendingDecision {
        bool resolved = false;
        bool approved = false;
    };
    std::mutex mtx;
    std::condition_variable cv;
    std::map<std::string, PendingDecision> pending;
    std::set<std::string> session_remembered;
    std::atomic<bool> stop_requested{false};

public:
    void reset_stop() { stop_requested.store(false); }
    bool is_stopped() const { return stop_requested.load(); }

    void emergency_stop() {
        stop_requested.store(true);
        std::lock_guard<std::mutex> lk(mtx);
        for (auto& kv : pending) {
            kv.second.resolved = true;
            kv.second.approved = false;
        }
        cv.notify_all();
    }

    void resolve(const std::string& req_id, bool approved, bool remember_session, const std::string& kind = "") {
        std::lock_guard<std::mutex> lk(mtx);
        if (approved && remember_session && !kind.empty()) {
            session_remembered.insert(kind);
        }
        auto it = pending.find(req_id);
        if (it != pending.end()) {
            it->second.resolved = true;
            it->second.approved = approved;
        }
        cv.notify_all();
    }

    bool request_approval(const std::string& kind,
                          const std::string& title,
                          const std::string& detail,
                          const std::string& command,
                          const std::string& target_path,
                          const std::string& cwd,
                          EventEmitterFn emit_fn) {
        if (stop_requested.load()) return false;
        {
            std::lock_guard<std::mutex> lk(mtx);
            if (session_remembered.count(kind)) return true;
        }
        std::string req_id = make_id("perm");
        {
            std::lock_guard<std::mutex> lk(mtx);
            pending[req_id] = PendingDecision{false, false};
        }
        json ev = {
            {"requestId", req_id},
            {"kind", kind},
            {"title", title},
            {"detail", detail}
        };
        if (!command.empty()) ev["command"] = command;
        if (!target_path.empty()) ev["targetPath"] = target_path;
        if (!cwd.empty()) ev["cwd"] = cwd;
        emit_fn("agent://permission_request", ev);

        std::unique_lock<std::mutex> lk(mtx);
        cv.wait(lk, [&]() {
            return stop_requested.load() || pending[req_id].resolved;
        });
        bool ok = !stop_requested.load() && pending[req_id].approved;
        pending.erase(req_id);
        return ok;
    }
};

// ============================================================================
// Multi-Provider 9-Tool Autonomous AI Coding Agent Engine
// ============================================================================
inline json build_agent_tool_schemas() {
    return json::parse(R"JSON([
      {
        "type": "function",
        "function": {
          "name": "read_file",
          "description": "Read a file in the project with optional 1-indexed start_line and end_line.",
          "parameters": {
            "type": "object",
            "properties": {
              "path": { "type": "string" },
              "start_line": { "type": "integer" },
              "end_line": { "type": "integer" }
            },
            "required": ["path"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "write_file",
          "description": "Create or overwrite a file in the project workspace.",
          "parameters": {
            "type": "object",
            "properties": {
              "path": { "type": "string" },
              "content": { "type": "string" }
            },
            "required": ["path", "content"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "edit_file",
          "description": "Surgically replace exact old_text with new_text in an existing file.",
          "parameters": {
            "type": "object",
            "properties": {
              "path": { "type": "string" },
              "old_text": { "type": "string" },
              "new_text": { "type": "string" }
            },
            "required": ["path", "old_text", "new_text"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "search_project",
          "description": "Search across project files for symbols, imports, or text.",
          "parameters": {
            "type": "object",
            "properties": {
              "query": { "type": "string" },
              "case_sensitive": { "type": "boolean" }
            },
            "required": ["query"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "execute_command",
          "description": "Execute a PowerShell command in the Windows workspace and capture stdout/stderr.",
          "parameters": {
            "type": "object",
            "properties": {
              "command": { "type": "string" },
              "cwd": { "type": "string" }
            },
            "required": ["command"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "install_dependencies",
          "description": "Install dependencies using npm, pnpm, yarn, cargo, or pip.",
          "parameters": {
            "type": "object",
            "properties": {
              "manager": { "type": "string", "enum": ["npm", "pnpm", "yarn", "cargo", "pip"] },
              "packages": { "type": "array", "items": { "type": "string" } },
              "dev": { "type": "boolean" }
            },
            "required": ["manager"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "run_tests",
          "description": "Run the project's test suite and return full pass/fail diagnostics.",
          "parameters": {
            "type": "object",
            "properties": {
              "command": { "type": "string" }
            }
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "diagnose_errors",
          "description": "Run TypeScript/Rust/compiler checks on the workspace to catch syntax or type errors.",
          "parameters": {
            "type": "object",
            "properties": {
              "target": { "type": "string" }
            }
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "git_operation",
          "description": "Run Git status, diff, log, branch, add, or commit.",
          "parameters": {
            "type": "object",
            "properties": {
              "subcommand": { "type": "string", "enum": ["status", "diff", "log", "branch", "add", "commit"] },
              "args": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["subcommand"]
          }
        }
      }
    ])JSON");
}

struct AgentToolCall {
    std::string id;
    std::string name;
    json args;
};

struct AgentModelTurn {
    std::string text;
    std::string thinking;
    std::vector<AgentToolCall> tool_calls;
    std::string error;
};

inline AgentModelTurn invoke_llm_provider(const json& settings,
                                          const std::string& provider,
                                          const std::string& model,
                                          const std::string& sys_prompt,
                                          const json& conv) {
    AgentModelTurn out;
    json oa_tools = build_agent_tool_schemas();

    if (provider == "anthropic") {
        std::string key = settings.value("anthropicApiKey", "");
        if (key.empty()) {
            out.error = "Anthropic API key is not configured. Open Settings in the top-right bar to enter your API key, or switch to Ollama.";
            return out;
        }
        json anth_tools = json::array();
        for (const auto& t : oa_tools) {
            anth_tools.push_back({
                {"name", t["function"]["name"]},
                {"description", t["function"]["description"]},
                {"input_schema", t["function"]["parameters"]}
            });
        }
        json anth_msgs = json::array();
        for (const auto& m : conv) {
            std::string role = m.value("role", "user");
            if (role == "tool") {
                anth_msgs.push_back({
                    {"role", "user"},
                    {"content", json::array({{
                        {"type", "tool_result"},
                        {"tool_use_id", m["tool_call_id"]},
                        {"content", m["content"]}
                    }})}
                });
            } else if (role == "assistant") {
                json blocks = json::array();
                std::string txt = m.value("content", "");
                if (!txt.empty()) blocks.push_back({{"type", "text"}, {"text", txt}});
                if (m.contains("tool_calls") && m["tool_calls"].is_array()) {
                    for (const auto& tc : m["tool_calls"]) {
                        json inp = json::parse(tc["function"]["arguments"].get<std::string>(), nullptr, false);
                        if (!inp.is_object()) inp = json::object();
                        blocks.push_back({
                            {"type", "tool_use"},
                            {"id", tc["id"]},
                            {"name", tc["function"]["name"]},
                            {"input", inp}
                        });
                    }
                }
                if (!blocks.empty()) anth_msgs.push_back({{"role", "assistant"}, {"content", blocks}});
            } else {
                anth_msgs.push_back({{"role", "user"}, {"content", m.value("content", "")}});
            }
        }
        json payload = {
            {"model", model},
            {"max_tokens", 4096},
            {"system", sys_prompt},
            {"tools", anth_tools},
            {"messages", anth_msgs}
        };
        auto resp = winhttp_request("POST", "https://api.anthropic.com/v1/messages",
            {{"Content-Type", "application/json"}, {"x-api-key", key}, {"anthropic-version", "2023-06-01"}},
            payload.dump());
        if (!resp.error.empty()) { out.error = resp.error; return out; }
        if (resp.status < 200 || resp.status >= 300) {
            out.error = "Anthropic HTTP " + std::to_string(resp.status) + ": " + resp.body.substr(0, 500);
            return out;
        }
        json body = json::parse(resp.body, nullptr, false);
        if (body.contains("content") && body["content"].is_array()) {
            for (const auto& b : body["content"]) {
                std::string btype = b.value("type", "");
                if (btype == "text") out.text += b.value("text", "");
                else if (btype == "thinking") out.thinking += b.value("thinking", "");
                else if (btype == "tool_use") {
                    out.tool_calls.push_back({
                        b.value("id", make_id("call")),
                        b.value("name", ""),
                        b.value("input", json::object())
                    });
                }
            }
        }
        return out;
    }

    if (provider == "gemini") {
        std::string key = settings.value("geminiApiKey", "");
        if (key.empty()) {
            out.error = "Google Gemini API key is empty. Configure it in Settings.";
            return out;
        }
        json decls = json::array();
        for (const auto& t : oa_tools) {
            decls.push_back({
                {"name", t["function"]["name"]},
                {"description", t["function"]["description"]},
                {"parameters", t["function"]["parameters"]}
            });
        }
        json contents = json::array();
        for (const auto& m : conv) {
            std::string role = m.value("role", "user");
            if (role == "tool") {
                contents.push_back({
                    {"role", "user"},
                    {"parts", json::array({{
                        {"functionResponse", {
                            {"name", m.value("name", "tool")},
                            {"response", {{"result", m.value("content", "")}}}
                        }}
                    }})}
                });
            } else if (role == "assistant") {
                json parts = json::array();
                std::string txt = m.value("content", "");
                if (!txt.empty()) parts.push_back({{"text", txt}});
                if (m.contains("tool_calls") && m["tool_calls"].is_array()) {
                    for (const auto& tc : m["tool_calls"]) {
                        json inp = json::parse(tc["function"]["arguments"].get<std::string>(), nullptr, false);
                        if (!inp.is_object()) inp = json::object();
                        parts.push_back({{"functionCall", {{"name", tc["function"]["name"]}, {"args", inp}}}});
                    }
                }
                if (!parts.empty()) contents.push_back({{"role", "model"}, {"parts", parts}});
            } else {
                contents.push_back({{"role", "user"}, {"parts", json::array({{{"text", m.value("content", "")}}})}});
            }
        }
        std::string url = "https://generativelanguage.googleapis.com/v1beta/models/" + model + ":generateContent?key=" + key;
        json payload = {
            {"systemInstruction", {{"parts", json::array({{{"text", sys_prompt}}})}}},
            {"contents", contents},
            {"tools", json::array({{{"functionDeclarations", decls}}})}
        };
        auto resp = winhttp_request("POST", url, {{"Content-Type", "application/json"}}, payload.dump());
        if (!resp.error.empty()) { out.error = resp.error; return out; }
        if (resp.status < 200 || resp.status >= 300) {
            out.error = "Gemini HTTP " + std::to_string(resp.status) + ": " + resp.body.substr(0, 500);
            return out;
        }
        json body = json::parse(resp.body, nullptr, false);
        if (body.contains("candidates") && !body["candidates"].empty()) {
            auto parts = body["candidates"][0]["content"]["parts"];
            if (parts.is_array()) {
                for (const auto& p : parts) {
                    if (p.contains("text")) out.text += p["text"].get<std::string>();
                    if (p.contains("functionCall")) {
                        out.tool_calls.push_back({
                            make_id("gem"),
                            p["functionCall"].value("name", ""),
                            p["functionCall"].value("args", json::object())
                        });
                    }
                }
            }
        }
        return out;
    }

    // OpenAI-compatible protocol: openai, openrouter, ollama, custom
    std::string endpoint;
    std::string bearer;
    if (provider == "openai") {
        bearer = settings.value("openaiApiKey", "");
        if (bearer.empty()) {
            out.error = "OpenAI API key is empty. Enter it in Settings or switch to local Ollama.";
            return out;
        }
        endpoint = "https://api.openai.com/v1/chat/completions";
    } else if (provider == "openrouter") {
        bearer = settings.value("openrouterApiKey", "");
        if (bearer.empty()) {
            out.error = "OpenRouter API key is empty. Enter it in Settings.";
            return out;
        }
        endpoint = "https://openrouter.ai/api/v1/chat/completions";
    } else if (provider == "custom") {
        std::string base = settings.value("customBaseUrl", "");
        while (!base.empty() && base.back() == '/') base.pop_back();
        if (base.empty()) {
            out.error = "Custom Base URL is empty in Settings.";
            return out;
        }
        endpoint = base + "/chat/completions";
        bearer = settings.value("customApiKey", "");
    } else {
        std::string base = settings.value("ollamaBaseUrl", "http://127.0.0.1:11434");
        while (!base.empty() && base.back() == '/') base.pop_back();
        endpoint = base + "/v1/chat/completions";
    }

    json msgs = json::array({{{"role", "system"}, {"content", sys_prompt}}});
    for (const auto& m : conv) msgs.push_back(m);

    json payload = {
        {"model", model},
        {"messages", msgs},
        {"tools", oa_tools},
        {"tool_choice", "auto"}
    };
    std::vector<std::pair<std::string, std::string>> hdrs = {{"Content-Type", "application/json"}};
    if (!bearer.empty()) hdrs.push_back({"Authorization", "Bearer " + bearer});

    auto resp = winhttp_request("POST", endpoint, hdrs, payload.dump());
    if (!resp.error.empty()) { out.error = resp.error; return out; }
    if (resp.status < 200 || resp.status >= 300) {
        out.error = "Provider '" + provider + "' HTTP " + std::to_string(resp.status) + ": " + resp.body.substr(0, 500);
        return out;
    }
    json body = json::parse(resp.body, nullptr, false);
    if (body.contains("choices") && !body["choices"].empty()) {
        auto msg = body["choices"][0]["message"];
        if (msg.contains("content") && msg["content"].is_string()) {
            out.text = msg["content"].get<std::string>();
        }
        if (msg.contains("reasoning_content") && msg["reasoning_content"].is_string()) {
            out.thinking = msg["reasoning_content"].get<std::string>();
        }
        if (msg.contains("tool_calls") && msg["tool_calls"].is_array()) {
            for (const auto& tc : msg["tool_calls"]) {
                std::string raw_a = tc["function"].value("arguments", "{}");
                json parsed_a = json::parse(raw_a, nullptr, false);
                if (!parsed_a.is_object()) parsed_a = json::object();
                out.tool_calls.push_back({
                    tc.value("id", make_id("call")),
                    tc["function"].value("name", ""),
                    parsed_a
                });
            }
        }
    }
    return out;
}

inline std::pair<std::string, json> execute_agent_tool(
    const fs::path& project_root,
    const json& settings,
    Win32PermissionGate& gate,
    const std::string& session_id,
    const std::string& mode,
    const AgentToolCall& call,
    EventEmitterFn emit_fn) {

    auto t0 = std::chrono::steady_clock::now();
    std::string args_str = call.args.dump();
    bool restrict_ws = settings.value("restrictToWorkspace", true);
    bool req_shell = settings.value("requireShellApproval", true);

    bool is_mut_file = (call.name == "write_file" || call.name == "edit_file");
    bool is_shell = (call.name == "execute_command" || call.name == "install_dependencies" || call.name == "run_tests");
    bool is_mut_git = (call.name == "git_operation" &&
        (call.args.value("subcommand", "") == "add" || call.args.value("subcommand", "") == "commit"));

    auto make_rec = [&](const std::string& status, const std::string& summary,
                        const std::string& diff_txt = "", const std::string& fpath = "") -> json {
        auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::steady_clock::now() - t0).count();
        json r = {
            {"id", call.id},
            {"sessionId", session_id},
            {"toolName", call.name},
            {"argsJson", args_str},
            {"resultSummary", summary.substr(0, 220)},
            {"status", status},
            {"durationMs", ms},
            {"createdAt", iso_now_utc()}
        };
        if (!diff_txt.empty()) r["diffText"] = diff_txt;
        if (!fpath.empty()) r["filePath"] = fpath;
        return r;
    };

    if (mode == "plan" && (is_mut_file || is_shell || is_mut_git)) {
        std::string msg = "Blocked in Plan Mode: '" + call.name + "' cannot mutate files or execute shell commands. Return a structured architectural plan.";
        return {msg, make_rec("denied", msg)};
    }

    std::string root_u8 = wide_to_utf8(project_root.wstring());
    if (mode == "ask" && (is_mut_file || is_shell || is_mut_git)) {
        bool ok = gate.request_approval(
            is_shell ? "shell" : "outside_workspace",
            "Ask Mode Approval: " + call.name,
            "Ask Mode requires explicit approval before running mutating tools.",
            is_shell ? args_str : "",
            call.args.value("path", ""),
            root_u8,
            emit_fn);
        if (!ok) {
            std::string msg = "User denied execution of '" + call.name + "'.";
            return {msg, make_rec("denied", msg)};
        }
    } else if (is_shell && req_shell && call.name == "execute_command") {
        std::string cmd_s = call.args.value("command", "");
        bool ok = gate.request_approval(
            "shell",
            "Approve Shell Command Execution",
            "Autonomous agent requested PowerShell execution in your Windows workspace.",
            cmd_s,
            "",
            root_u8,
            emit_fn);
        if (!ok) {
            std::string msg = "User denied shell command execution.";
            return {msg, make_rec("denied", msg)};
        }
    }

    if (call.name == "read_file") {
        std::string p = call.args.value("path", "");
        fs::path resolved; std::string err;
        if (!resolve_safe_path(project_root, p, restrict_ws, resolved, err)) {
            return {err, make_rec("error", err)};
        }
        std::string content;
        if (!read_utf8_file(resolved, content)) {
            std::string m = "Cannot read file: " + p;
            return {m, make_rec("error", m)};
        }
        std::istringstream iss(content);
        std::vector<std::string> lines;
        std::string ln;
        while (std::getline(iss, ln)) lines.push_back(ln);
        int s = std::max(1, call.args.value("start_line", 1));
        int e = call.args.value("end_line", std::min<int>((int)lines.size(), s + 350));
        std::ostringstream out;
        out << "File: " << p << " (" << lines.size() << " lines)\n";
        for (int i = s; i <= e && i <= (int)lines.size(); ++i) {
            out << i << " | " << lines[i - 1] << "\n";
        }
        return {out.str(), make_rec("ok", "Read " + p, "", wide_to_utf8(resolved.wstring()))};
    }

    if (call.name == "write_file") {
        std::string p = call.args.value("path", "");
        std::string content = call.args.value("content", "");
        fs::path resolved; std::string err;
        if (!resolve_safe_path(project_root, p, restrict_ws, resolved, err)) {
            return {err, make_rec("error", err)};
        }
        std::string old_txt;
        read_utf8_file(resolved, old_txt);
        if (!write_utf8_file(resolved, content)) {
            std::string m = "Failed writing file: " + p;
            return {m, make_rec("error", m)};
        }
        std::string diff = build_unified_diff(p, old_txt, content);
        emit_fn("fs://changed", {{"kind", "write"}, {"path", p}});
        std::string m = "Wrote " + std::to_string(content.size()) + " bytes to " + p;
        return {m, make_rec("ok", m, diff, p)};
    }

    if (call.name == "edit_file") {
        std::string p = call.args.value("path", "");
        std::string old_t = call.args.value("old_text", "");
        std::string new_t = call.args.value("new_text", "");
        fs::path resolved; std::string err;
        if (!resolve_safe_path(project_root, p, restrict_ws, resolved, err)) {
            return {err, make_rec("error", err)};
        }
        std::string existing;
        if (!read_utf8_file(resolved, existing)) {
            std::string m = "Cannot read file: " + p;
            return {m, make_rec("error", m)};
        }
        size_t pos = existing.find(old_t);
        if (pos == std::string::npos) {
            std::string m = "old_text not found in " + p + ". Read the file first to verify exact context.";
            return {m, make_rec("error", m)};
        }
        std::string updated = existing.substr(0, pos) + new_t + existing.substr(pos + old_t.size());
        if (!write_utf8_file(resolved, updated)) {
            std::string m = "Failed writing updated file: " + p;
            return {m, make_rec("error", m)};
        }
        std::string diff = build_unified_diff(p, existing, updated);
        emit_fn("fs://changed", {{"kind", "edit"}, {"path", p}});
        std::string m = "Edited " + p + " successfully.";
        return {m, make_rec("ok", m, diff, p)};
    }

    if (call.name == "search_project") {
        std::string q = call.args.value("query", "");
        bool cs = call.args.value("case_sensitive", false);
        json hits = search_in_workspace(project_root, q, cs, 60);
        if (hits.empty()) {
            std::string m = "No matches for '" + q + "'.";
            return {m, make_rec("ok", m)};
        }
        std::ostringstream out;
        out << "Found " << hits.size() << " matches:\n";
        for (const auto& h : hits) {
            out << h["relPath"].get<std::string>() << ":" << h["line"].get<size_t>()
                << " -> " << h["preview"].get<std::string>() << "\n";
        }
        return {out.str(), make_rec("ok", "Found " + std::to_string(hits.size()) + " matches")};
    }

    if (call.name == "execute_command") {
        std::string cmd_s = call.args.value("command", "");
        std::wstring ps = L"powershell.exe -NoLogo -NoProfile -NonInteractive -Command " + utf8_to_wide(cmd_s);
        auto r = run_process_silent(ps, project_root);
        std::string out = "Exit Code: " + std::to_string(r.exit_code) + "\nSTDOUT:\n" + r.stdout_text + "\nSTDERR:\n" + r.stderr_text;
        return {out, make_rec(r.exit_code == 0 ? "ok" : "error", "Exit " + std::to_string(r.exit_code) + ": " + cmd_s)};
    }

    if (call.name == "install_dependencies") {
        std::string mgr = call.args.value("manager", "npm");
        std::string pkgs;
        if (call.args.contains("packages") && call.args["packages"].is_array()) {
            for (const auto& x : call.args["packages"]) {
                if (x.is_string()) pkgs += " " + x.get<std::string>();
            }
        }
        bool dev = call.args.value("dev", false);
        std::string cmd_s = "npm install";
        if (mgr == "cargo") cmd_s = pkgs.empty() ? "cargo fetch" : ("cargo add" + pkgs);
        else if (mgr == "pip") cmd_s = pkgs.empty() ? "python -m pip install -r requirements.txt" : ("python -m pip install" + pkgs);
        else if (mgr == "pnpm") cmd_s = std::string("pnpm add ") + (dev ? "-D" : "") + pkgs;
        else if (mgr == "yarn") cmd_s = std::string("yarn add ") + (dev ? "-D" : "") + pkgs;
        else if (!pkgs.empty()) cmd_s = std::string("npm install ") + (dev ? "--save-dev" : "") + pkgs;

        std::wstring ps = L"powershell.exe -NoLogo -NoProfile -NonInteractive -Command " + utf8_to_wide(cmd_s);
        auto r = run_process_silent(ps, project_root, 180000);
        std::string out = "Command: " + cmd_s + "\nExit Code: " + std::to_string(r.exit_code) + "\n" + r.stdout_text + "\n" + r.stderr_text;
        return {out, make_rec(r.exit_code == 0 ? "ok" : "error", cmd_s)};
    }

    if (call.name == "run_tests") {
        std::string cmd_s = call.args.value("command", "");
        if (cmd_s.empty()) {
            if (fs::exists(project_root / "Cargo.toml")) cmd_s = "cargo test";
            else if (fs::exists(project_root / "package.json")) cmd_s = "npm test -- --watchAll=false";
            else cmd_s = "pytest";
        }
        std::wstring ps = L"powershell.exe -NoLogo -NoProfile -NonInteractive -Command " + utf8_to_wide(cmd_s);
        auto r = run_process_silent(ps, project_root, 180000);
        std::string out = "Test Command: " + cmd_s + "\nExit Code: " + std::to_string(r.exit_code) + "\n" + r.stdout_text + "\n" + r.stderr_text;
        return {out, make_rec(r.exit_code == 0 ? "ok" : "error", "Tests (" + cmd_s + ") exit " + std::to_string(r.exit_code))};
    }

    if (call.name == "diagnose_errors") {
        std::string report;
        bool any_err = false;
        if (fs::exists(project_root / "tsconfig.json")) {
            auto r = run_process_silent(L"powershell.exe -NoLogo -NoProfile -NonInteractive -Command npx tsc --noEmit", project_root);
            report += "[TypeScript tsc --noEmit] exit=" + std::to_string(r.exit_code) + "\n" + r.stdout_text + r.stderr_text + "\n";
            if (r.exit_code != 0) any_err = true;
        }
        if (fs::exists(project_root / "Cargo.toml")) {
            auto r = run_process_silent(L"powershell.exe -NoLogo -NoProfile -NonInteractive -Command cargo check --message-format=short", project_root);
            report += "[Rust cargo check] exit=" + std::to_string(r.exit_code) + "\n" + r.stdout_text + r.stderr_text + "\n";
            if (r.exit_code != 0) any_err = true;
        }
        if (report.empty()) report = "No tsconfig.json or Cargo.toml at workspace root.";
        return {report, make_rec(any_err ? "error" : "ok", any_err ? "Diagnostics reported errors" : "Diagnostics clean")};
    }

    if (call.name == "git_operation") {
        std::string sub = call.args.value("subcommand", "status");
        std::string arg_line = sub;
        if (call.args.contains("args") && call.args["args"].is_array()) {
            for (const auto& a : call.args["args"]) {
                if (a.is_string()) arg_line += " " + a.get<std::string>();
            }
        }
        auto r = run_git_cmd(project_root, arg_line);
        std::string out = "git " + arg_line + " -> exit " + std::to_string(r.exit_code) + "\n" + r.stdout_text + r.stderr_text;
        return {out, make_rec(r.exit_code == 0 ? "ok" : "error", "git " + arg_line)};
    }

    return {"Unknown tool: " + call.name, make_rec("error", "Unknown tool")};
}

inline void run_win32_agent_loop(
    NativeSqliteStore& store,
    Win32PermissionGate& gate,
    const fs::path& project_root,
    const std::string& session_id,
    const std::string& prompt,
    const std::string& mode,
    const std::string& provider_in,
    const std::string& model_in,
    EventEmitterFn emit_fn) {

    gate.reset_stop();
    json settings = store.load_settings();
    std::string provider = provider_in.empty() ? settings.value("activeProvider", "ollama") : provider_in;
    std::string model = model_in.empty() ? settings.value("activeModel", "qwen2.5-coder:7b") : model_in;

    store.append_message({
        {"id", make_id("msg")},
        {"sessionId", session_id},
        {"role", "user"},
        {"content", prompt},
        {"mode", mode},
        {"provider", provider},
        {"model", model},
        {"tools", json::array()},
        {"createdAt", iso_now_utc()}
    });

    json conv = json::array();
    json loaded = store.load_session(session_id);
    if (loaded.is_object() && loaded.contains("messages")) {
        auto& msgs = loaded["messages"];
        size_t start_i = msgs.size() > 24 ? (msgs.size() - 24) : 0;
        for (size_t i = start_i; i < msgs.size(); ++i) {
            std::string r = msgs[i].value("role", "user");
            if (r == "user" || r == "assistant") {
                conv.push_back({{"role", r}, {"content", msgs[i].value("content", "")}});
            }
        }
    }

    json sample_tree = scan_project_files(project_root, 80);
    std::ostringstream tree_ss;
    for (const auto& f : sample_tree) {
        tree_ss << f["relPath"].get<std::string>() << (f["isDir"].get<bool>() ? "/" : "") << "\n";
    }
    std::string mode_desc = (mode == "plan")
        ? "MODE: PLAN (Read-Only). Inspect files and return a thorough file-by-file architectural plan without modifying files."
        : (mode == "ask")
            ? "MODE: ASK. Mutating tools will prompt the user for confirmation."
            : "MODE: AUTONOMOUS. Proactively inspect files, implement complete multi-file changes, run diagnose_errors / run_tests, fix any errors, and deliver verified results.";

    std::string sys_prompt =
        "You are AI Coding Agent, a native Windows x64 desktop software engineering agent.\n"
        "Active Workspace: " + wide_to_utf8(project_root.wstring()) + "\n" +
        mode_desc + "\n\nWorkspace Files:\n" + tree_ss.str();

    int max_iters = std::clamp(settings.value("maxAgentIterations", 15), 1, 30);
    json collected_tools = json::array();
    std::string final_text;
    std::string final_thinking;

    for (int step = 1; step <= max_iters; ++step) {
        if (gate.is_stopped()) break;

        emit_fn("agent://stream", {
            {"sessionId", session_id},
            {"kind", "status"},
            {"statusText", "Step " + std::to_string(step) + "/" + std::to_string(max_iters) + ": Querying " + provider + " (" + model + ")"}
        });

        AgentModelTurn turn = invoke_llm_provider(settings, provider, model, sys_prompt, conv);
        if (!turn.error.empty()) {
            emit_fn("agent://stream", {
                {"sessionId", session_id},
                {"kind", "error"},
                {"text", turn.error}
            });
            store.append_message({
                {"id", make_id("msg")},
                {"sessionId", session_id},
                {"role", "assistant"},
                {"content", "⚠️ " + turn.error},
                {"mode", mode},
                {"provider", provider},
                {"model", model},
                {"tools", collected_tools},
                {"createdAt", iso_now_utc()}
            });
            emit_fn("agent://stream", {
                {"sessionId", session_id},
                {"kind", "done"},
                {"session", store.load_session(session_id)}
            });
            return;
        }

        if (!turn.thinking.empty()) {
            final_thinking += turn.thinking;
            emit_fn("agent://stream", {
                {"sessionId", session_id},
                {"kind", "thinking"},
                {"text", turn.thinking}
            });
        }

        if (!turn.text.empty()) {
            if (!final_text.empty()) final_text += "\n\n";
            final_text += turn.text;
            emit_fn("agent://stream", {
                {"sessionId", session_id},
                {"kind", "text"},
                {"text", turn.text}
            });
        }

        if (turn.tool_calls.empty()) {
            break;
        }

        json oa_tcs = json::array();
        for (const auto& tc : turn.tool_calls) {
            oa_tcs.push_back({
                {"id", tc.id},
                {"type", "function"},
                {"function", {{"name", tc.name}, {"arguments", tc.args.dump()}}}
            });
        }
        conv.push_back({
            {"role", "assistant"},
            {"content", turn.text},
            {"tool_calls", oa_tcs}
        });

        for (const auto& tc : turn.tool_calls) {
            if (gate.is_stopped()) break;
            json start_rec = {
                {"id", tc.id},
                {"sessionId", session_id},
                {"toolName", tc.name},
                {"argsJson", tc.args.dump()},
                {"resultSummary", "Running..."},
                {"status", "running"},
                {"durationMs", 0},
                {"createdAt", iso_now_utc()}
            };
            emit_fn("agent://stream", {
                {"sessionId", session_id},
                {"kind", "tool_start"},
                {"tool", start_rec}
            });

            auto [tool_out, end_rec] = execute_agent_tool(
                project_root, settings, gate, session_id, mode, tc, emit_fn);

            store.record_tool_execution(end_rec);
            collected_tools.push_back(end_rec);

            emit_fn("agent://stream", {
                {"sessionId", session_id},
                {"kind", "tool_end"},
                {"tool", end_rec}
            });

            conv.push_back({
                {"role", "tool"},
                {"tool_call_id", tc.id},
                {"name", tc.name},
                {"content", tool_out}
            });
        }
    }

    if (final_text.empty()) {
        final_text = gate.is_stopped()
            ? "Execution halted immediately via Emergency Stop."
            : "Completed autonomous execution cycle.";
    }

    store.append_message({
        {"id", make_id("msg")},
        {"sessionId", session_id},
        {"role", "assistant"},
        {"content", final_text},
        {"thinking", final_thinking},
        {"mode", mode},
        {"provider", provider},
        {"model", model},
        {"tools", collected_tools},
        {"createdAt", iso_now_utc()}
    });

    emit_fn("agent://stream", {
        {"sessionId", session_id},
        {"kind", "done"},
        {"session", store.load_session(session_id)}
    });
}
