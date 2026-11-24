#pragma once

#include "win_utils.hpp"
#include "workspace_tools.hpp"
#include "llm_client.hpp"
#include "embedded_ui.hpp"
#include "httplib.h"

namespace aircode {

inline json build_agent_tools_schema(bool read_only) {
    json tools = json::array();

    tools.push_back({
        {"type", "function"},
        {"function", {
            {"name", "list_files"},
            {"description", "List workspace files and structural Repo Map symbols."},
            {"parameters", {
                {"type", "object"},
                {"properties", json::object()}
            }}
        }}
    });

    tools.push_back({
        {"type", "function"},
        {"function", {
            {"name", "read_file"},
            {"description", "Read lines of a file from the workspace with line numbers."},
            {"parameters", {
                {"type", "object"},
                {"properties", {
                    {"path", {{"type", "string"}, {"description", "Relative path in workspace"}}},
                    {"offset", {{"type", "integer"}, {"description", "1-based start line"}}},
                    {"limit", {{"type", "integer"}, {"description", "Max lines to read (default 250)"}}}
                }},
                {"required", json::array({"path"})}
            }}
        }}
    });

    tools.push_back({
        {"type", "function"},
        {"function", {
            {"name", "grep_search"},
            {"description", "Fast search for an exact string or identifier across all codebase files."},
            {"parameters", {
                {"type", "object"},
                {"properties", {
                    {"query", {{"type", "string"}, {"description", "Search query"}}},
                    {"file_pattern", {{"type", "string"}, {"description", "Optional path substring filter"}}}
                }},
                {"required", json::array({"query"})}
            }}
        }}
    });

    tools.push_back({
        {"type", "function"},
        {"function", {
            {"name", "git_status"},
            {"description", "Check Git branch, changed files, and recent commits."},
            {"parameters", {{"type", "object"}, {"properties", json::object()}}}
        }}
    });

    tools.push_back({
        {"type", "function"},
        {"function", {
            {"name", "git_diff"},
            {"description", "View unified git diff for workspace or a single file."},
            {"parameters", {
                {"type", "object"},
                {"properties", {
                    {"path", {{"type", "string"}, {"description", "Optional file path"}}}
                }}
            }}
        }}
    });

    tools.push_back({
        {"type", "function"},
        {"function", {
            {"name", "spawn_subagent"},
            {"description", "Delegate a focused codebase exploration or verification sub-task to an isolated sub-agent."},
            {"parameters", {
                {"type", "object"},
                {"properties", {
                    {"task", {{"type", "string"}, {"description", "Focused sub-task description"}}},
                    {"target_query", {{"type", "string"}, {"description", "Optional symbol or path to inspect"}}}
                }},
                {"required", json::array({"task"})}
            }}
        }}
    });

    if (!read_only) {
        tools.push_back({
            {"type", "function"},
            {"function", {
                {"name", "write_file"},
                {"description", "Create a new file or overwrite an existing file with complete contents."},
                {"parameters", {
                    {"type", "object"},
                    {"properties", {
                        {"path", {{"type", "string"}, {"description", "Relative file path"}}},
                        {"content", {{"type", "string"}, {"description", "Complete file content"}}}
                    }},
                    {"required", json::array({"path", "content"})}
                }}
            }}
        });

        tools.push_back({
            {"type", "function"},
            {"function", {
                {"name", "edit_file"},
                {"description", "Surgically replace old_text with new_text in an existing file. Read the file first so old_text matches."},
                {"parameters", {
                    {"type", "object"},
                    {"properties", {
                        {"path", {{"type", "string"}, {"description", "Relative file path"}}},
                        {"old_text", {{"type", "string"}, {"description", "Exact existing lines to replace"}}},
                        {"new_text", {{"type", "string"}, {"description", "New replacement lines"}}}
                    }},
                    {"required", json::array({"path", "old_text", "new_text"})}
                }}
            }}
        });

        tools.push_back({
            {"type", "function"},
            {"function", {
                {"name", "run_command"},
                {"description", "Run a build, test, or CLI command inside the workspace directory."},
                {"parameters", {
                    {"type", "object"},
                    {"properties", {
                        {"command", {{"type", "string"}, {"description", "Command to execute"}}}
                    }},
                    {"required", json::array({"command"})}
                }}
            }}
        });
    }

    return tools;
}

class AircodeEngine {
public:
    explicit AircodeEngine(const std::string& initial_workspace)
        : workspace_(initial_workspace), appdata_(get_appdata_dir()) {
        load_config();
        load_sessions();
    }

    int start_background_server() {
        setup_routes();
        int port = server_.bind_to_any_port("127.0.0.1");
        port_ = port;
        server_thread_ = std::thread([this]() {
            server_.listen_after_bind();
        });
        return port;
    }

    void stop() {
        cancel_flag_.store(true);
        if (server_.is_running()) {
            server_.stop();
        }
        if (server_thread_.joinable()) {
            server_thread_.join();
        }
    }

    int port() const { return port_; }

private:
    WorkspaceManager workspace_;
    fs::path appdata_;
    mutable std::mutex state_mu_;
    json config_;
    json sessions_;
    std::atomic<bool> cancel_flag_{false};
    httplib::Server server_;
    std::thread server_thread_;
    int port_ = 0;

    fs::path config_path() const { return appdata_ / L"config.json"; }
    fs::path sessions_path() const { return appdata_ / L"sessions.json"; }

    void load_config() {
        std::lock_guard<std::mutex> lk(state_mu_);
        bool ok = false;
        std::string raw = read_text_file(config_path(), &ok);
        if (ok && !raw.empty()) {
            try {
                config_ = json::parse(raw);
            } catch (...) {
                config_ = json::object();
            }
        }
        if (!config_.is_object()) config_ = json::object();
        if (!config_.contains("provider")) config_["provider"] = "local-assist";
        if (!config_.contains("model")) config_["model"] = "aircode-local-instant";
        if (!config_.contains("baseUrl")) config_["baseUrl"] = "";
        if (!config_.contains("apiKeys") || !config_["apiKeys"].is_object()) {
            config_["apiKeys"] = json::object();
        }
        if (!config_.contains("permissionMode")) config_["permissionMode"] = "auto-edit";
        if (!config_.contains("maxSteps")) config_["maxSteps"] = 12;
        if (!config_.contains("theme")) config_["theme"] = "obsidian";
        if (!config_.contains("shell")) config_["shell"] = "powershell";
        if (!config_.contains("mcpServers") || !config_["mcpServers"].is_array()) {
            config_["mcpServers"] = json::array({
                {{"id", "mcp_fs"}, {"name", "Filesystem MCP"}, {"command", "npx -y @modelcontextprotocol/server-filesystem ."}, {"enabled", true}},
                {{"id", "mcp_git"}, {"name", "GitHub MCP"}, {"command", "npx -y @modelcontextprotocol/server-github"}, {"enabled", false}},
                {{"id", "mcp_fetch"}, {"name", "Web Fetch MCP"}, {"command", "npx -y @modelcontextprotocol/server-fetch"}, {"enabled", false}}
            });
        }
        if (!config_.contains("customCommands") || !config_["customCommands"].is_array()) {
            config_["customCommands"] = json::array({
                {{"slash", "/optimize"}, {"label", "Performance Audit"}, {"prompt", "Analyze hot paths in this project and suggest high-impact speed & memory optimizations."}},
                {{"slash", "/win32"}, {"label", "Windows Portability Check"}, {"prompt", "Check paths, scripts, and configs in this repo for Windows 10/11 compatibility issues."}}
            });
        }
    }

    void save_config_locked() {
        write_text_file(config_path(), config_.dump(2));
    }

    void load_sessions() {
        std::lock_guard<std::mutex> lk(state_mu_);
        bool ok = false;
        std::string raw = read_text_file(sessions_path(), &ok);
        if (ok && !raw.empty()) {
            try {
                sessions_ = json::parse(raw);
            } catch (...) {
                sessions_ = json::array();
            }
        }
        if (!sessions_.is_array()) sessions_ = json::array();
        if (sessions_.empty()) {
            sessions_.push_back({
                {"id", random_id("ses")},
                {"title", "Welcome Session"},
                {"mode", "build"},
                {"createdAt", now_iso8601()},
                {"updatedAt", now_iso8601()},
                {"promptTokens", 0},
                {"completionTokens", 0},
                {"costUsd", 0.0},
                {"messages", json::array()}
            });
            save_sessions_locked();
        }
    }

    void save_sessions_locked() {
        write_text_file(sessions_path(), sessions_.dump(2));
    }

    std::string active_api_key_locked(const std::string& provider) const {
        if (config_.contains("apiKeys") && config_["apiKeys"].contains(provider) && config_["apiKeys"][provider].is_string()) {
            std::string k = config_["apiKeys"][provider].get<std::string>();
            if (!k.empty()) return k;
        }
        // Fallback to Windows environment variables if user already set OPENAI_API_KEY / ANTHROPIC_API_KEY / etc.
        const char* env_name = nullptr;
        if (provider == "openai") env_name = "OPENAI_API_KEY";
        else if (provider == "anthropic") env_name = "ANTHROPIC_API_KEY";
        else if (provider == "gemini") env_name = "GEMINI_API_KEY";
        else if (provider == "openrouter") env_name = "OPENROUTER_API_KEY";
        else if (provider == "groq") env_name = "GROQ_API_KEY";
        else if (provider == "deepseek") env_name = "DEEPSEEK_API_KEY";
        else if (provider == "github") env_name = "GITHUB_TOKEN";
        if (env_name) {
            char buf[512] = {0};
            DWORD n = GetEnvironmentVariableA(env_name, buf, sizeof(buf));
            if (n > 0 && n < sizeof(buf)) return std::string(buf, n);
        }
        return "";
    }

    std::string build_project_rules_context() const {
        std::ostringstream ctx;
        const char* rule_files[] = {"AGENTS.md", "CLAUDE.md", ".aircoderules", ".cursorrules", ".aircode/MEMORY.md"};
        for (const char* rf : rule_files) {
            fs::path p = workspace_.resolve_safe(rf);
            bool ok = false;
            std::string txt = read_text_file(p, &ok);
            if (ok && !txt.empty()) {
                ctx << "\n--- Project File: " << rf << " ---\n"
                    << txt.substr(0, 6000) << "\n";
            }
        }
        return ctx.str();
    }

    std::string generate_agents_md_content() const {
        auto scan = workspace_.scan_workspace(400);
        std::set<std::string> langs;
        std::vector<std::string> key_files;
        for (const auto& f : scan["files"]) {
            std::string l = f.value("language", "plaintext");
            if (l != "plaintext") langs.insert(l);
            std::string p = f.value("path", "");
            if (p == "package.json" || p == "Cargo.toml" || p == "pyproject.toml" ||
                p == "CMakeLists.txt" || p == "README.md" || p.find("main.") != std::string::npos ||
                p.find("index.") != std::string::npos || p.find("app.") != std::string::npos) {
                if (key_files.size() < 15) key_files.push_back(p);
            }
        }
        std::ostringstream md;
        md << "# AGENTS.md — Aircode Project Intelligence\n\n"
           << "## Workspace Overview\n"
           << "- **Root**: `" << workspace_.root_utf8() << "`\n"
           << "- **Indexed Files**: " << scan.value("count", 0) << "\n"
           << "- **Detected Languages**: ";
        if (langs.empty()) md << "General";
        else {
            bool first = true;
            for (const auto& l : langs) {
                if (!first) md << ", ";
                md << "`" << l << "`";
                first = false;
            }
        }
        md << "\n\n## Key Entrypoints & Config Files\n";
        for (const auto& kf : key_files) {
            md << "- `" << kf << "`\n";
        }
        if (!scan["repoMap"].empty()) {
            md << "\n## Structural Repo Map\n";
            size_t shown = 0;
            for (const auto& entry : scan["repoMap"]) {
                md << "- **`" << entry.value("path", "") << "`**\n";
                for (const auto& s : entry["symbols"]) {
                    md << "  - `" << s.get<std::string>() << "`\n";
                }
                if (++shown >= 12) break;
            }
        }
        md << "\n## Coding & Agent Conventions\n"
           << "1. Prefer surgical edits (`edit_file`) over full file rewrites when modifying existing code.\n"
           << "2. Preserve cross-platform & Windows 10/11 path safety and UTF-8 encoding.\n"
           << "3. Run verification builds or tests after making multi-file changes.\n";
        return md.str();
    }

    // Built-in Zero-Cost Offline Local Assistant when no external API key is configured yet
    void run_local_instant_assistant(
        const std::string& prompt,
        const std::string& mode,
        const std::string& checkpoint_id,
        std::function<void(const std::string&, const json&)> emit_sse,
        std::string& final_reply,
        json& tool_trace,
        std::vector<std::string>& touched_files
    ) {
        std::string lower = to_lower_ascii(prompt);
        emit_sse("reasoning_delta", {{"text", "Running Aircode Local Instant Engine (0 cloud tokens, native C++ workspace inspection)...\n"}});

        // Always inspect workspace structure first
        std::string call_id = random_id("call");
        emit_sse("tool_start", {{"id", call_id}, {"name", "list_files"}, {"args", json::object()}});
        auto scan = workspace_.scan_workspace(350);
        emit_sse("tool_result", {
            {"id", call_id},
            {"name", "list_files"},
            {"ok", true},
            {"summary", "Indexed " + std::to_string(scan.value("count", 0)) + " files & " + std::to_string(scan["repoMap"].size()) + " symbol maps"},
            {"output", "Workspace: " + scan.value("root", "") + " (" + std::to_string(scan.value("count", 0)) + " files)"},
            {"durationMs", 4}
        });
        tool_trace.push_back({{"name", "list_files"}, {"summary", "Indexed " + std::to_string(scan.value("count", 0)) + " files"}});

        std::ostringstream reply;

        if (lower.find("/init") != std::string::npos || lower.find("agents.md") != std::string::npos) {
            std::string md = generate_agents_md_content();
            if (mode != "plan") {
                std::string wid = random_id("call");
                emit_sse("tool_start", {{"id", wid}, {"name", "write_file"}, {"args", {{"path", "AGENTS.md"}}}});
                auto wres = workspace_.tool_write_file("AGENTS.md", md, checkpoint_id);
                touched_files.push_back("AGENTS.md");
                emit_sse("tool_result", {
                    {"id", wid},
                    {"name", "write_file"},
                    {"ok", wres.value("ok", false)},
                    {"summary", "Generated AGENTS.md (" + std::to_string(md.size()) + " bytes)"},
                    {"diff", wres.value("diff", "")},
                    {"fileChanged", "AGENTS.md"},
                    {"durationMs", 3}
                });
                tool_trace.push_back({{"name", "write_file"}, {"summary", "Wrote AGENTS.md"}});
                reply << "I scanned your workspace and generated **`AGENTS.md`** with your detected tech stack, key entrypoints, and structural Repo Map.\n\n"
                      << "You can view or customize it anytime in the **Rules & Memory** tab or the **Diff Center**!";
            } else {
                reply << "### Proposed `AGENTS.md` Plan\n\n```markdown\n" << md << "\n```";
            }
        } else if (lower.find("/review") != std::string::npos || mode == "review") {
            std::string gid = random_id("call");
            emit_sse("tool_start", {{"id", gid}, {"name", "git_status"}, {"args", json::object()}});
            auto gs = workspace_.git_status();
            auto diag = workspace_.scan_diagnostics();
            emit_sse("tool_result", {
                {"id", gid},
                {"name", "git_status"},
                {"ok", true},
                {"summary", gs.value("isRepo", false) ? ("Branch " + gs.value("branch", "") + " (" + std::to_string(gs.value("dirtyCount", 0)) + " modified)") : "Non-git folder"},
                {"output", gs.dump(2)},
                {"durationMs", 12}
            });
            reply << "## Codebase & Diagnostics Audit\n\n"
                  << "- **Workspace**: `" << workspace_.root_utf8() << "`\n"
                  << "- **Files Scanned**: **" << scan.value("count", 0) << "** across **" << scan["repoMap"].size() << "** symbol-indexed modules\n"
                  << "- **Git State**: " << (gs.value("isRepo", false) ? ("`" + gs.value("branch", "main") + "` with **" + std::to_string(gs.value("dirtyCount", 0)) + "** uncommitted change(s)") : "No `.git` repository initialized") << "\n"
                  << "- **Static Diagnostics Found**: **" << diag.value("count", 0) << "**\n\n";
            if (!diag["diagnostics"].empty()) {
                reply << "### Detected Issues\n";
                for (const auto& d : diag["diagnostics"]) {
                    reply << "- **[" << d.value("severity", "warning") << "]** `" << d.value("file", "") << ":" << d.value("line", 1) << "` — " << d.value("message", "") << "\n";
                }
            } else {
                reply << "No syntax or conflict-marker errors were detected in static checks.\n";
            }
            reply << "\n> **Tip**: Connect **Ollama (100% Free Local)**, **OpenRouter Free (`qwen3-coder:free`)**, **Gemini Free Tier**, **Groq**, **Claude**, or **OpenAI** in the **Model Hub** button at the top bar for deep multi-file AI reasoning!";
        } else {
            // Perform smart keyword search if user mentioned an identifier
            std::istringstream iss(prompt);
            std::string word, best_kw;
            while (iss >> word) {
                while (!word.empty() && ispunct(static_cast<unsigned char>(word.front())) && word.front() != '@') word.erase(0, 1);
                while (!word.empty() && ispunct(static_cast<unsigned char>(word.back()))) word.pop_back();
                if (!word.empty() && word.front() == '@') {
                    std::string fpath = word.substr(1);
                    std::string rid = random_id("call");
                    emit_sse("tool_start", {{"id", rid}, {"name", "read_file"}, {"args", {{"path", fpath}}}});
                    auto rres = workspace_.tool_read_file(fpath, 1, 120);
                    emit_sse("tool_result", {
                        {"id", rid},
                        {"name", "read_file"},
                        {"ok", rres.value("ok", false)},
                        {"summary", rres.value("ok", false) ? ("Read " + fpath + " (" + std::to_string(rres.value("totalLines", 0)) + " lines)") : rres.value("error", "")},
                        {"output", rres.value("content", "").substr(0, 1500)},
                        {"durationMs", 2}
                    });
                } else if (word.size() >= 4 && word.size() > best_kw.size() &&
                           to_lower_ascii(word) != "explain" && to_lower_ascii(word) != "project" &&
                           to_lower_ascii(word) != "codebase" && to_lower_ascii(word) != "files" &&
                           to_lower_ascii(word) != "please" && to_lower_ascii(word) != "what") {
                    best_kw = word;
                }
            }

            json search_hits = json::object();
            if (!best_kw.empty()) {
                std::string sid = random_id("call");
                emit_sse("tool_start", {{"id", sid}, {"name", "grep_search"}, {"args", {{"query", best_kw}}}});
                search_hits = workspace_.tool_grep_search(best_kw, false, "", 15);
                emit_sse("tool_result", {
                    {"id", sid},
                    {"name", "grep_search"},
                    {"ok", true},
                    {"summary", "Found " + std::to_string(search_hits.value("count", 0)) + " match(es) for '" + best_kw + "'"},
                    {"output", search_hits.dump(2).substr(0, 1500)},
                    {"durationMs", 5}
                });
            }

            reply << "### Workspace Intelligence Summary (`Aircode Local Instant`)\n\n"
                  << "I inspected **`" << workspace_.root_utf8() << "`** natively in `<10ms`:\n"
                  << "- **Total Files**: `" << scan.value("count", 0) << "`\n"
                  << "- **Symbol-Mapped Code Modules**: `" << scan["repoMap"].size() << "`\n";

            if (!scan["repoMap"].empty()) {
                reply << "\n#### Key Architecture & Symbols\n";
                size_t c = 0;
                for (const auto& entry : scan["repoMap"]) {
                    reply << "- **`" << entry.value("path", "") << "`**";
                    if (!entry["symbols"].empty()) {
                        reply << " — `" << entry["symbols"][0].get<std::string>() << "`";
                    }
                    reply << "\n";
                    if (++c >= 8) break;
                }
            }

            if (search_hits.contains("matches") && !search_hits["matches"].empty()) {
                reply << "\n#### Codebase Matches for `" << best_kw << "`\n";
                size_t c = 0;
                for (const auto& m : search_hits["matches"]) {
                    reply << "- `" << m.value("file", "") << ":" << m.value("line", 1) << "` -> `" << m.value("preview", "") << "`\n";
                    if (++c >= 6) break;
                }
            }

            reply << "\n---\n"
                  << "**Ready for Full Autonomous Coding (Zero Cloud Compute Limits!):**\n"
                  << "1. **100% Free Local AI**: Run **Ollama** (`ollama run qwen2.5-coder:7b`) or **LM Studio** — Aircode auto-detects `localhost:11434` & `1234` with 1 click!\n"
                  << "2. **Free Cloud Coding APIs**: Click the **Provider / Model Selector** above to select **OpenRouter `:free`** (`qwen3-coder:free`, `deepseek-chat-v3:free`), **Google Gemini Free**, or **Groq Free**.\n"
                  << "3. **Instant Built-In Commands**: Try `/init` (generates `AGENTS.md`), `/review` (static audit), `/compact`, or `/undo` right now!";
        }

        final_reply = reply.str();
        // Stream out chunks smoothly
        const size_t chunk_sz = 48;
        for (size_t i = 0; i < final_reply.size(); i += chunk_sz) {
            emit_sse("text_delta", {{"text", final_reply.substr(i, chunk_sz)}});
        }
    }

    void setup_routes() {
        // Serve Embedded Desktop Studio UI
        server_.Get("/", [](const httplib::Request&, httplib::Response& res) {
            res.set_header("Cache-Control", "no-store");
            res.set_content(reinterpret_cast<const char*>(EMBEDDED_INDEX_HTML), EMBEDDED_INDEX_HTML_LEN, "text/html; charset=utf-8");
        });
        server_.Get("/styles.css", [](const httplib::Request&, httplib::Response& res) {
            res.set_header("Cache-Control", "no-store");
            res.set_content(reinterpret_cast<const char*>(EMBEDDED_STYLES_CSS), EMBEDDED_STYLES_CSS_LEN, "text/css; charset=utf-8");
        });
        server_.Get("/app.js", [](const httplib::Request&, httplib::Response& res) {
            res.set_header("Cache-Control", "no-store");
            res.set_content(reinterpret_cast<const char*>(EMBEDDED_APP_JS), EMBEDDED_APP_JS_LEN, "application/javascript; charset=utf-8");
        });

        // Boot state
        server_.Get("/api/state", [this](const httplib::Request&, httplib::Response& res) {
            json cfg_copy, ses_list = json::array();
            {
                std::lock_guard<std::mutex> lk(state_mu_);
                cfg_copy = config_;
                // Mask raw keys slightly in state overview while preserving configured flags
                json configured = json::object();
                if (cfg_copy.contains("apiKeys") && cfg_copy["apiKeys"].is_object()) {
                    for (auto& [k, v] : cfg_copy["apiKeys"].items()) {
                        configured[k] = v.is_string() && !v.get<std::string>().empty();
                    }
                }
                cfg_copy["configuredProviders"] = configured;
                for (const auto& s : sessions_) {
                    ses_list.push_back({
                        {"id", s.value("id", "")},
                        {"title", s.value("title", "Untitled")},
                        {"mode", s.value("mode", "build")},
                        {"updatedAt", s.value("updatedAt", "")},
                        {"messageCount", s.contains("messages") ? s["messages"].size() : 0},
                        {"promptTokens", s.value("promptTokens", 0)},
                        {"completionTokens", s.value("completionTokens", 0)},
                        {"costUsd", s.value("costUsd", 0.0)}
                    });
                }
            }
            json out = {
                {"ok", true},
                {"platform", "win32"},
                {"runtime", "Native Win32 C++17 + Edge WebView2 + WinHTTP"},
                {"version", "1.2.0"},
                {"exePath", get_exe_path_utf8()},
                {"appDataDir", path_to_native_utf8(appdata_)},
                {"workspace", workspace_.root_utf8()},
                {"config", cfg_copy},
                {"sessions", ses_list},
                {"git", workspace_.git_status()},
                {"checkpoints", workspace_.list_checkpoints_json()}
            };
            res.set_content(out.dump(), "application/json");
        });

        // Update config
        server_.Post("/api/config", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto patch = json::parse(req.body);
                std::lock_guard<std::mutex> lk(state_mu_);
                for (auto& [k, v] : patch.items()) {
                    if (k == "apiKeys" && v.is_object()) {
                        for (auto& [pk, pv] : v.items()) {
                            config_["apiKeys"][pk] = pv;
                        }
                    } else {
                        config_[k] = v;
                    }
                }
                save_config_locked();
                res.set_content(json({{"ok", true}, {"config", config_}}).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Probe local Ollama / LM Studio servers
        server_.Get("/api/providers/detect", [](const httplib::Request&, httplib::Response& res) {
            auto detected = probe_local_ai_servers();
            res.set_content(json({{"ok", true}, {"detected", detected}}).dump(), "application/json");
        });

        // Native Windows Folder Picker + Direct Workspace Switcher
        server_.Post("/api/native/pick-folder", [this](const httplib::Request& req, httplib::Response& res) {
            std::string chosen;
            if (!req.body.empty()) {
                try {
                    auto j = json::parse(req.body);
                    if (j.contains("path") && j["path"].is_string() && !j["path"].get<std::string>().empty()) {
                        chosen = j["path"].get<std::string>();
                    }
                } catch (...) {}
            }
            if (chosen.empty()) {
                chosen = pick_folder_dialog_win32(workspace_.root_utf8());
            }
            if (chosen.empty()) {
                res.set_content(json({{"ok", false}, {"cancelled", true}}).dump(), "application/json");
                return;
            }
            workspace_.set_workspace(chosen);
            res.set_content(json({
                {"ok", true},
                {"workspace", workspace_.root_utf8()},
                {"git", workspace_.git_status()}
            }).dump(), "application/json");
        });

        // Windows Explorer Right-Click Context Menu Registration & Open in Explorer
        server_.Post("/api/native/windows-shell-menu", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body.empty() ? "{}" : req.body);
                std::string action = j.value("action", "register");
                if (action == "open_explorer") {
                    std::wstring wroot = utf8_to_wide(workspace_.root_utf8());
                    ShellExecuteW(nullptr, L"explore", wroot.c_str(), nullptr, nullptr, SW_SHOWNORMAL);
                    res.set_content(json({{"ok", true}, {"message", "Opened folder in Windows Explorer"}}).dump(), "application/json");
                    return;
                }
                bool enable = (action == "register");
                std::string err;
                bool ok = set_windows_context_menu(enable, &err);
                res.set_content(json({
                    {"ok", ok},
                    {"error", err},
                    {"message", enable ? "Added 'Open in Aircode' to Windows Explorer right-click menu!" : "Removed Windows Explorer context menu entry."}
                }).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Workspace Files + Repo Map
        server_.Get("/api/files", [this](const httplib::Request&, httplib::Response& res) {
            res.set_content(workspace_.scan_workspace().dump(), "application/json");
        });

        // Read single file
        server_.Get("/api/file", [this](const httplib::Request& req, httplib::Response& res) {
            std::string p = req.get_param_value("path");
            std::string rel;
            fs::path full = workspace_.resolve_safe(p, &rel);
            bool ok = false;
            std::string content = read_text_file(full, &ok);
            if (!ok) {
                res.status = 404;
                res.set_content(json({{"ok", false}, {"error", "File not found: " + p}}).dump(), "application/json");
                return;
            }
            res.set_content(json({
                {"ok", true},
                {"path", rel},
                {"language", detect_language(rel)},
                {"content", content},
                {"lineCount", split_lines(content).size()}
            }).dump(), "application/json");
        });

        // Save file
        server_.Post("/api/file", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                std::string p = j.value("path", "");
                std::string content = j.value("content", "");
                std::string cp = workspace_.create_checkpoint("Manual Save: " + p);
                auto wres = workspace_.tool_write_file(p, content, cp);
                res.set_content(wres.dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // File Explorer CRUD operations
        server_.Post("/api/file/ops", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                std::string op = j.value("op", "");
                std::string p = j.value("path", "");
                fs::path full = workspace_.resolve_safe(p);
                std::error_code ec;
                if (op == "create_file") {
                    write_text_file(full, j.value("content", ""));
                } else if (op == "create_dir") {
                    fs::create_directories(full, ec);
                } else if (op == "delete") {
                    fs::remove_all(full, ec);
                } else if (op == "rename") {
                    fs::path dst = workspace_.resolve_safe(j.value("newPath", ""));
                    fs::rename(full, dst, ec);
                }
                res.set_content(json({{"ok", !ec}, {"error", ec ? ec.message() : ""}}).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Codebase Search
        server_.Post("/api/search", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                auto out = workspace_.tool_grep_search(
                    j.value("query", ""),
                    j.value("caseSensitive", false),
                    j.value("filePattern", ""),
                    120
                );
                res.set_content(out.dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Git Endpoints
        server_.Get("/api/git/status", [this](const httplib::Request&, httplib::Response& res) {
            res.set_content(workspace_.git_status().dump(), "application/json");
        });

        server_.Get("/api/git/diff", [this](const httplib::Request& req, httplib::Response& res) {
            res.set_content(workspace_.git_diff(req.get_param_value("path")).dump(), "application/json");
        });

        server_.Post("/api/git/action", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                std::string action = j.value("action", "");
                std::string file = j.value("path", "");
                std::string cwd = workspace_.root_utf8();
                ProcessResult pr;

                if (action == "stage") {
                    pr = run_process_silent("git add -- \"" + file + "\"", cwd, "cmd", 8000);
                } else if (action == "stage_all") {
                    pr = run_process_silent("git add -A", cwd, "cmd", 8000);
                } else if (action == "unstage") {
                    pr = run_process_silent("git reset HEAD -- \"" + file + "\"", cwd, "cmd", 8000);
                } else if (action == "discard") {
                    pr = run_process_silent("git checkout -- \"" + file + "\"", cwd, "cmd", 8000);
                } else if (action == "commit") {
                    std::string msg = j.value("message", "chore: update via Aircode");
                    std::replace(msg.begin(), msg.end(), '"', '\'');
                    pr = run_process_silent("git commit -m \"" + msg + "\"", cwd, "cmd", 12000);
                } else if (action == "generate_commit_msg") {
                    auto gs = workspace_.git_status();
                    std::ostringstream msg;
                    if (!gs["files"].empty()) {
                        msg << "feat: update ";
                        size_t n = std::min<size_t>(3, gs["files"].size());
                        for (size_t i = 0; i < n; ++i) {
                            if (i > 0) msg << ", ";
                            msg << gs["files"][i].value("path", "");
                        }
                        if (gs["files"].size() > 3) msg << " and " << (gs["files"].size() - 3) << " more files";
                    } else {
                        msg << "chore: workspace updates";
                    }
                    res.set_content(json({{"ok", true}, {"message", msg.str()}}).dump(), "application/json");
                    return;
                }
                res.set_content(json({
                    {"ok", pr.exit_code == 0},
                    {"stdout", pr.stdout_text},
                    {"stderr", pr.stderr_text},
                    {"git", workspace_.git_status()}
                }).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Checkpoints / Undo
        server_.Get("/api/checkpoints", [this](const httplib::Request&, httplib::Response& res) {
            res.set_content(json({{"ok", true}, {"checkpoints", workspace_.list_checkpoints_json()}}).dump(), "application/json");
        });

        server_.Post("/api/checkpoints/restore", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body.empty() ? "{}" : req.body);
                auto out = workspace_.restore_checkpoint(j.value("id", "latest"));
                res.set_content(out.dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Rules & Memory Bank (AGENTS.md, CLAUDE.md, .aircode/MEMORY.md)
        server_.Get("/api/rules", [this](const httplib::Request&, httplib::Response& res) {
            const char* targets[] = {"AGENTS.md", "CLAUDE.md", ".aircoderules", ".aircode/MEMORY.md"};
            json items = json::array();
            for (const char* t : targets) {
                bool ok = false;
                std::string txt = read_text_file(workspace_.resolve_safe(t), &ok);
                items.push_back({{"path", t}, {"exists", ok}, {"content", txt}});
            }
            res.set_content(json({{"ok", true}, {"files", items}}).dump(), "application/json");
        });

        server_.Post("/api/rules", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                std::string target = j.value("path", "AGENTS.md");
                std::string content = j.value("content", "");
                if (j.value("generateDefault", false)) {
                    content = generate_agents_md_content();
                }
                bool ok = write_text_file(workspace_.resolve_safe(target), content);
                res.set_content(json({{"ok", ok}, {"path", target}, {"content", content}}).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Integrated Multi-Shell Terminal
        server_.Post("/api/terminal/exec", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                std::string cmd = j.value("command", "");
                std::string shell = j.value("shell", "powershell");
                auto pr = run_process_silent(cmd, workspace_.root_utf8(), shell, 45000);
                res.set_content(json({
                    {"ok", true},
                    {"exitCode", pr.exit_code},
                    {"stdout", pr.stdout_text},
                    {"stderr", pr.stderr_text},
                    {"durationMs", pr.duration_ms},
                    {"timedOut", pr.timed_out},
                    {"shell", shell}
                }).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Diagnostics Scanner
        server_.Get("/api/diagnostics", [this](const httplib::Request&, httplib::Response& res) {
            res.set_content(workspace_.scan_diagnostics().dump(), "application/json");
        });

        // MCP Server Probe
        server_.Post("/api/mcp/test", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                std::string cmd = j.value("command", "");
                auto pr = run_process_silent(cmd + " --version", workspace_.root_utf8(), "cmd", 6000);
                res.set_content(json({
                    {"ok", true},
                    {"reachable", pr.exit_code == 0 || !pr.stdout_text.empty()},
                    {"output", pr.stdout_text + pr.stderr_text}
                }).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Sessions Management
        server_.Get("/api/session", [this](const httplib::Request& req, httplib::Response& res) {
            std::string id = req.get_param_value("id");
            std::lock_guard<std::mutex> lk(state_mu_);
            for (const auto& s : sessions_) {
                if (id.empty() || s.value("id", "") == id) {
                    res.set_content(json({{"ok", true}, {"session", s}}).dump(), "application/json");
                    return;
                }
            }
            res.status = 404;
            res.set_content(json({{"ok", false}, {"error", "Session not found"}}).dump(), "application/json");
        });

        server_.Post("/api/sessions", [this](const httplib::Request& req, httplib::Response& res) {
            auto j = req.body.empty() ? json::object() : json::parse(req.body, nullptr, false);
            std::lock_guard<std::mutex> lk(state_mu_);
            json ses = {
                {"id", random_id("ses")},
                {"title", j.is_object() ? j.value("title", "New Coding Session") : "New Coding Session"},
                {"mode", j.is_object() ? j.value("mode", "build") : "build"},
                {"createdAt", now_iso8601()},
                {"updatedAt", now_iso8601()},
                {"promptTokens", 0},
                {"completionTokens", 0},
                {"costUsd", 0.0},
                {"messages", json::array()}
            };
            sessions_.insert(sessions_.begin(), ses);
            save_sessions_locked();
            res.set_content(json({{"ok", true}, {"session", ses}}).dump(), "application/json");
        });

        server_.Post("/api/sessions/delete", [this](const httplib::Request& req, httplib::Response& res) {
            auto j = json::parse(req.body, nullptr, false);
            std::string id = j.is_object() ? j.value("id", "") : "";
            std::lock_guard<std::mutex> lk(state_mu_);
            for (auto it = sessions_.begin(); it != sessions_.end(); ++it) {
                if (it->value("id", "") == id) {
                    sessions_.erase(it);
                    break;
                }
            }
            if (sessions_.empty()) {
                sessions_.push_back({
                    {"id", random_id("ses")},
                    {"title", "New Coding Session"},
                    {"mode", "build"},
                    {"createdAt", now_iso8601()},
                    {"updatedAt", now_iso8601()},
                    {"promptTokens", 0},
                    {"completionTokens", 0},
                    {"costUsd", 0.0},
                    {"messages", json::array()}
                });
            }
            save_sessions_locked();
            res.set_content(json({{"ok", true}, {"activeId", sessions_[0]["id"]}}).dump(), "application/json");
        });

        server_.Post("/api/sessions/fork", [this](const httplib::Request& req, httplib::Response& res) {
            auto j = json::parse(req.body, nullptr, false);
            std::string id = j.is_object() ? j.value("id", "") : "";
            std::lock_guard<std::mutex> lk(state_mu_);
            for (const auto& s : sessions_) {
                if (s.value("id", "") == id) {
                    json forked = s;
                    forked["id"] = random_id("ses");
                    forked["title"] = s.value("title", "Session") + " (Fork)";
                    forked["updatedAt"] = now_iso8601();
                    sessions_.insert(sessions_.begin(), forked);
                    save_sessions_locked();
                    res.set_content(json({{"ok", true}, {"session", forked}}).dump(), "application/json");
                    return;
                }
            }
            res.status = 404;
            res.set_content(json({{"ok", false}}).dump(), "application/json");
        });

        // /compact Context Compression
        server_.Post("/api/sessions/compact", [this](const httplib::Request& req, httplib::Response& res) {
            auto j = json::parse(req.body, nullptr, false);
            std::string id = j.is_object() ? j.value("id", "") : "";
            std::lock_guard<std::mutex> lk(state_mu_);
            for (auto& s : sessions_) {
                if (s.value("id", "") == id) {
                    auto& msgs = s["messages"];
                    size_t old_count = msgs.size();
                    if (old_count > 2) {
                        std::ostringstream summary;
                        summary << "**[Compacted Session Context — " << old_count << " messages summarized]**\n";
                        for (size_t i = 0; i < msgs.size(); ++i) {
                            std::string r = msgs[i].value("role", "user");
                            std::string c = msgs[i].value("content", "");
                            if (c.size() > 160) c = c.substr(0, 160) + "...";
                            summary << "- **" << r << "**: " << c << "\n";
                        }
                        json last_msg = msgs.back();
                        msgs = json::array({
                            {{"id", random_id("msg")}, {"role", "assistant"}, {"content", summary.str()}, {"timestamp", now_iso8601()}},
                            last_msg
                        });
                        s["promptTokens"] = static_cast<int>(summary.str().size() / 4);
                        save_sessions_locked();
                    }
                    res.set_content(json({{"ok", true}, {"session", s}, {"compactedFrom", old_count}}).dump(), "application/json");
                    return;
                }
            }
            res.status = 404;
            res.set_content(json({{"ok", false}}).dump(), "application/json");
        });

        // Editor Ctrl+K Inline AI Transform
        server_.Post("/api/inline-edit", [this](const httplib::Request& req, httplib::Response& res) {
            try {
                auto j = json::parse(req.body);
                std::string path = j.value("path", "untitled");
                std::string selection = j.value("selection", "");
                std::string instruction = j.value("instruction", "");

                std::string provider, model, base_url, api_key;
                {
                    std::lock_guard<std::mutex> lk(state_mu_);
                    provider = config_.value("provider", "local-assist");
                    model = config_.value("model", "aircode-local-instant");
                    base_url = config_.value("baseUrl", "");
                    api_key = active_api_key_locked(provider);
                }

                std::string transformed = selection;
                bool used_llm = (provider != "local-assist") &&
                                (provider == "ollama" || provider == "lmstudio" || !api_key.empty());

                if (used_llm) {
                    std::string sys = "You are Aircode Inline Code Transformer. Output ONLY the transformed code replacement without markdown fences or explanations.";
                    json msgs = json::array({
                        {{"role", "user"}, {"content", "File: " + path + "\nInstruction: " + instruction + "\n\nSelected code:\n" + selection}}
                    });
                    auto llm_res = stream_llm_completion(provider, base_url, api_key, model, sys, msgs, json::array(), nullptr, nullptr, nullptr);
                    if (llm_res.ok && !llm_res.content.empty()) {
                        transformed = llm_res.content;
                        // Strip accidental markdown code fences
                        if (transformed.rfind("```", 0) == 0) {
                            auto nl = transformed.find('\n');
                            if (nl != std::string::npos) transformed = transformed.substr(nl + 1);
                            auto end_f = transformed.rfind("```");
                            if (end_f != std::string::npos) transformed = transformed.substr(0, end_f);
                        }
                    }
                } else {
                    // Instant local inline code helper (comments/docstrings/try-catch/formatting)
                    std::string low = to_lower_ascii(instruction);
                    if (low.find("comment") != std::string::npos || low.find("doc") != std::string::npos) {
                        transformed = "// [Aircode] " + instruction + "\n" + selection;
                    } else if (low.find("try") != std::string::npos || low.find("error") != std::string::npos) {
                        transformed = "try {\n" + selection + "\n} catch (err) {\n  console.error('Operation failed:', err);\n  throw err;\n}";
                    } else {
                        transformed = "// Transformed (" + instruction + "):\n" + selection;
                    }
                }

                res.set_content(json({
                    {"ok", true},
                    {"transformed", transformed},
                    {"diff", compute_unified_diff(path, selection, transformed)}
                }).dump(), "application/json");
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
            }
        });

        // Abort active stream
        server_.Post("/api/agent/abort", [this](const httplib::Request&, httplib::Response& res) {
            cancel_flag_.store(true);
            res.set_content(json({{"ok", true}}).dump(), "application/json");
        });

        // ==================== REAL-TIME SSE AUTONOMOUS AGENT STREAM ====================
        server_.Post("/api/agent/stream", [this](const httplib::Request& req, httplib::Response& res) {
            json body;
            try {
                body = json::parse(req.body);
            } catch (const std::exception& e) {
                res.status = 400;
                res.set_content(json({{"ok", false}, {"error", e.what()}}).dump(), "application/json");
                return;
            }

            cancel_flag_.store(false);
            res.set_header("Cache-Control", "no-cache");
            res.set_header("X-Accel-Buffering", "no");

            res.set_chunked_content_provider(
                "text/event-stream",
                [this, body](size_t /*offset*/, httplib::DataSink& sink) {
                    auto emit_sse = [&](const std::string& event_name, const json& data) {
                        std::string packet = "event: " + event_name + "\ndata: " + data.dump() + "\n\n";
                        sink.write(packet.data(), packet.size());
                    };

                    std::string session_id = body.value("sessionId", "");
                    std::string prompt = body.value("prompt", "");
                    std::string mode = body.value("mode", "build");
                    json attached_ctx = body.value("attachments", json::array());

                    std::string provider, model, base_url, api_key, perm_mode, shell_pref;
                    int max_steps = 12;
                    json history_for_llm = json::array();

                    {
                        std::lock_guard<std::mutex> lk(state_mu_);
                        provider = body.value("provider", config_.value("provider", "local-assist"));
                        model = body.value("model", config_.value("model", "aircode-local-instant"));
                        base_url = config_.value("baseUrl", "");
                        api_key = active_api_key_locked(provider);
                        perm_mode = config_.value("permissionMode", "auto-edit");
                        shell_pref = config_.value("shell", "powershell");
                        max_steps = config_.value("maxSteps", 12);

                        // Locate or create session
                        json* target_ses = nullptr;
                        for (auto& s : sessions_) {
                            if (s.value("id", "") == session_id) {
                                target_ses = &s;
                                break;
                            }
                        }
                        if (!target_ses && !sessions_.empty()) {
                            target_ses = &sessions_[0];
                            session_id = (*target_ses).value("id", "");
                        }
                        if (target_ses) {
                            (*target_ses)["mode"] = mode;
                            (*target_ses)["updatedAt"] = now_iso8601();
                            if ((*target_ses)["messages"].empty() && prompt.size() > 2) {
                                (*target_ses)["title"] = prompt.substr(0, std::min<size_t>(46, prompt.size()));
                            }
                            (*target_ses)["messages"].push_back({
                                {"id", random_id("msg")},
                                {"role", "user"},
                                {"content", prompt},
                                {"mode", mode},
                                {"timestamp", now_iso8601()}
                            });
                            for (const auto& m : (*target_ses)["messages"]) {
                                std::string r = m.value("role", "user");
                                if (r == "user" || r == "assistant") {
                                    history_for_llm.push_back({{"role", r}, {"content", m.value("content", "")}});
                                }
                            }
                            save_sessions_locked();
                        }
                    }

                    // Expand @mentions / attachments into last user message for LLM
                    if (!attached_ctx.empty() && !history_for_llm.empty()) {
                        std::ostringstream aug;
                        aug << history_for_llm.back().value("content", "") << "\n\n--- Attached Context ---\n";
                        for (const auto& att : attached_ctx) {
                            aug << "[" << att.value("label", "context") << "]:\n" << att.value("content", "") << "\n\n";
                        }
                        history_for_llm.back()["content"] = aug.str();
                    }

                    bool read_only = (mode == "plan" || mode == "review" || perm_mode == "read-only");
                    std::string checkpoint_id = read_only ? "" : workspace_.create_checkpoint(prompt.substr(0, 48));

                    emit_sse("status", {{"phase", "thinking"}, {"mode", mode}, {"provider", provider}, {"model", model}});

                    std::string final_text;
                    std::string full_reasoning;
                    json tool_trace = json::array();
                    std::vector<std::string> touched_files;
                    int total_prompt_tok = 0;
                    int total_comp_tok = 0;

                    bool can_use_cloud_or_local_llm =
                        (provider != "local-assist") &&
                        (provider == "ollama" || provider == "lmstudio" || !api_key.empty());

                    if (!can_use_cloud_or_local_llm) {
                        run_local_instant_assistant(prompt, mode, checkpoint_id, emit_sse, final_text, tool_trace, touched_files);
                        total_prompt_tok = static_cast<int>(prompt.size() / 4);
                        total_comp_tok = static_cast<int>(final_text.size() / 4);
                    } else {
                        // Build mode-specialized system prompt + Repo Map + AGENTS.md rules
                        auto scan = workspace_.scan_workspace(250);
                        std::ostringstream sys;
                        sys << "You are Aircode Studio, an ultra-fast autonomous AI software engineering agent running natively on Windows.\n"
                            << "Active Workspace: " << workspace_.root_utf8() << "\n"
                            << "Active Mode: " << mode << " | Permission Level: " << perm_mode << "\n";

                        if (mode == "plan") {
                            sys << "MODE=PLAN (Read-Only Architect): Inspect relevant files first, then produce a clear, numbered, file-by-file implementation plan. Do NOT modify files.\n";
                        } else if (mode == "turbo") {
                            sys << "MODE=TURBO COMPOSER: Execute fast, surgical multi-file edits (`edit_file` / `write_file`) with minimal prose.\n";
                        } else if (mode == "debug") {
                            sys << "MODE=DEBUG & FIX: Diagnose the root cause by reading code and running verification commands, apply surgical fixes, and verify.\n";
                        } else if (mode == "review") {
                            sys << "MODE=CODE & SECURITY REVIEW: Inspect git diff and source files, then report bugs, security risks, and performance issues by severity.\n";
                        } else {
                            sys << "MODE=BUILD (Autonomous Engineer): Read files before editing, make clean surgical changes, and verify your work.\n";
                        }

                        if (!scan["repoMap"].empty()) {
                            sys << "\n--- Workspace Repo Map ---\n";
                            size_t shown = 0;
                            for (const auto& rm : scan["repoMap"]) {
                                sys << rm.value("path", "") << ": ";
                                for (size_t i = 0; i < rm["symbols"].size() && i < 5; ++i) {
                                    if (i > 0) sys << " | ";
                                    sys << rm["symbols"][i].get<std::string>();
                                }
                                sys << "\n";
                                if (++shown >= 35) break;
                            }
                        }
                        sys << build_project_rules_context();

                        json tools_schema = build_agent_tools_schema(read_only);
                        bool checkpoint_emitted = false;

                        for (int step = 0; step < max_steps && !cancel_flag_.load(); ++step) {
                            auto step_res = stream_llm_completion(
                                provider,
                                base_url,
                                api_key,
                                model,
                                sys.str(),
                                history_for_llm,
                                tools_schema,
                                [&](const std::string& delta) {
                                    emit_sse("text_delta", {{"text", delta}});
                                },
                                [&](const std::string& rdelta) {
                                    emit_sse("reasoning_delta", {{"text", rdelta}});
                                },
                                &cancel_flag_
                            );

                            total_prompt_tok += step_res.prompt_tokens;
                            total_comp_tok += step_res.completion_tokens;

                            if (!step_res.ok) {
                                emit_sse("error", {
                                    {"message", step_res.error},
                                    {"hint", "Check your API key or switch to Ollama / OpenRouter Free / Aircode Local Instant in the Model Hub."}
                                });
                                break;
                            }

                            final_text += step_res.content;
                            full_reasoning += step_res.reasoning;

                            if (step_res.tool_calls.empty()) {
                                break; // Agent completed turn!
                            }

                            // Append assistant tool_calls turn to LLM conversation
                            json tc_json_arr = json::array();
                            for (const auto& tc : step_res.tool_calls) {
                                tc_json_arr.push_back({
                                    {"id", tc.id},
                                    {"type", "function"},
                                    {"function", {{"name", tc.name}, {"arguments", tc.arguments.dump()}}}
                                });
                            }
                            history_for_llm.push_back({
                                {"role", "assistant"},
                                {"content", step_res.content},
                                {"tool_calls", tc_json_arr}
                            });

                            // Execute each tool call natively
                            for (const auto& tc : step_res.tool_calls) {
                                if (cancel_flag_.load()) break;
                                auto t0 = std::chrono::steady_clock::now();
                                emit_sse("tool_start", {{"id", tc.id}, {"name", tc.name}, {"args", tc.arguments}});

                                json tool_out;
                                std::string summary;
                                std::string diff_str;
                                std::string changed_file;

                                if (tc.name == "list_files") {
                                    auto ws = workspace_.scan_workspace(300);
                                    summary = "Listed " + std::to_string(ws.value("count", 0)) + " workspace files";
                                    tool_out = ws;
                                } else if (tc.name == "read_file") {
                                    std::string p = tc.arguments.value("path", "");
                                    tool_out = workspace_.tool_read_file(p, tc.arguments.value("offset", 1), tc.arguments.value("limit", 250));
                                    summary = tool_out.value("ok", false) ? ("Read " + p + " (" + tool_out.value("shownRange", "") + ")") : tool_out.value("error", "Read failed");
                                } else if (tc.name == "grep_search") {
                                    std::string q = tc.arguments.value("query", "");
                                    tool_out = workspace_.tool_grep_search(q, false, tc.arguments.value("file_pattern", ""), 50);
                                    summary = "Found " + std::to_string(tool_out.value("count", 0)) + " matches for '" + q + "'";
                                } else if (tc.name == "git_status") {
                                    tool_out = workspace_.git_status();
                                    summary = "Checked git status";
                                } else if (tc.name == "git_diff") {
                                    tool_out = workspace_.git_diff(tc.arguments.value("path", ""));
                                    summary = "Inspected git diff";
                                } else if (tc.name == "spawn_subagent") {
                                    std::string task = tc.arguments.value("task", "");
                                    std::string tq = tc.arguments.value("target_query", "");
                                    emit_sse("subagent_update", {{"id", tc.id}, {"task", task}, {"status", "running"}});
                                    json sub_data = tq.empty() ? workspace_.scan_workspace(150) : workspace_.tool_grep_search(tq, false, "", 25);
                                    summary = "Sub-agent completed: " + task;
                                    tool_out = {{"ok", true}, {"task", task}, {"findings", sub_data}};
                                    emit_sse("subagent_update", {{"id", tc.id}, {"task", task}, {"status", "completed"}, {"summary", summary}});
                                } else if (tc.name == "write_file" && !read_only) {
                                    std::string p = tc.arguments.value("path", "");
                                    tool_out = workspace_.tool_write_file(p, tc.arguments.value("content", ""), checkpoint_id);
                                    diff_str = tool_out.value("diff", "");
                                    if (tool_out.value("ok", false)) {
                                        changed_file = tool_out.value("path", p);
                                        touched_files.push_back(changed_file);
                                        if (!checkpoint_emitted) {
                                            checkpoint_emitted = true;
                                            emit_sse("checkpoint", {{"id", checkpoint_id}, {"label", prompt.substr(0, 48)}});
                                        }
                                    }
                                    summary = tool_out.value("ok", false) ? ("Wrote " + p) : tool_out.value("error", "Write failed");
                                } else if (tc.name == "edit_file" && !read_only) {
                                    std::string p = tc.arguments.value("path", "");
                                    tool_out = workspace_.tool_edit_file(p, tc.arguments.value("old_text", ""), tc.arguments.value("new_text", ""), checkpoint_id);
                                    diff_str = tool_out.value("diff", "");
                                    if (tool_out.value("ok", false)) {
                                        changed_file = tool_out.value("path", p);
                                        touched_files.push_back(changed_file);
                                        if (!checkpoint_emitted) {
                                            checkpoint_emitted = true;
                                            emit_sse("checkpoint", {{"id", checkpoint_id}, {"label", prompt.substr(0, 48)}});
                                        }
                                    }
                                    summary = tool_out.value("ok", false) ? ("Edited " + p) : tool_out.value("error", "Edit failed");
                                } else if (tc.name == "run_command" && !read_only) {
                                    std::string cmd = tc.arguments.value("command", "");
                                    std::string low_cmd = to_lower_ascii(cmd);
                                    bool dangerous = (low_cmd.find("rm -rf /") != std::string::npos ||
                                                      low_cmd.find("format ") == 0 ||
                                                      low_cmd.find("del /f /s /q c:\\") != std::string::npos);
                                    if (dangerous && perm_mode != "autonomous") {
                                        tool_out = {{"ok", false}, {"error", "Blocked potentially destructive command under current permission policy."}};
                                        summary = "Blocked unsafe command";
                                    } else {
                                        auto pr = run_process_silent(cmd, workspace_.root_utf8(), shell_pref, 30000, &cancel_flag_);
                                        tool_out = {
                                            {"ok", pr.exit_code == 0},
                                            {"exitCode", pr.exit_code},
                                            {"stdout", pr.stdout_text.substr(0, 8000)},
                                            {"stderr", pr.stderr_text.substr(0, 4000)}
                                        };
                                        summary = "Ran `" + cmd.substr(0, 44) + "` (exit " + std::to_string(pr.exit_code) + ")";
                                    }
                                } else {
                                    tool_out = {{"ok", false}, {"error", "Tool unavailable in read-only mode: " + tc.name}};
                                    summary = "Skipped mutating tool in read-only mode";
                                }

                                auto dur = std::chrono::duration_cast<std::chrono::milliseconds>(
                                               std::chrono::steady_clock::now() - t0)
                                               .count();

                                std::string serialized = tool_out.dump();
                                if (serialized.size() > 14000) {
                                    serialized = serialized.substr(0, 14000) + "...[truncated]";
                                }

                                emit_sse("tool_result", {
                                    {"id", tc.id},
                                    {"name", tc.name},
                                    {"ok", tool_out.value("ok", true)},
                                    {"summary", summary},
                                    {"output", serialized.substr(0, 2000)},
                                    {"diff", diff_str},
                                    {"fileChanged", changed_file},
                                    {"durationMs", dur}
                                });

                                tool_trace.push_back({
                                    {"id", tc.id},
                                    {"name", tc.name},
                                    {"summary", summary},
                                    {"diff", diff_str},
                                    {"fileChanged", changed_file},
                                    {"durationMs", dur}
                                });

                                history_for_llm.push_back({
                                    {"role", "tool"},
                                    {"tool_call_id", tc.id},
                                    {"name", tc.name},
                                    {"content", serialized}
                                });
                            }
                        }
                    }

                    // Estimate cost ($0.00 for local/free models!)
                    double cost_usd = 0.0;
                    if (provider != "local-assist" && provider != "ollama" && provider != "lmstudio" &&
                        model.find(":free") == std::string::npos) {
                        cost_usd = (total_prompt_tok * 0.0000015) + (total_comp_tok * 0.000006);
                    }

                    json updated_session;
                    {
                        std::lock_guard<std::mutex> lk(state_mu_);
                        for (auto& s : sessions_) {
                            if (s.value("id", "") == session_id) {
                                s["messages"].push_back({
                                    {"id", random_id("msg")},
                                    {"role", "assistant"},
                                    {"content", final_text},
                                    {"reasoning", full_reasoning},
                                    {"tools", tool_trace},
                                    {"mode", mode},
                                    {"model", model},
                                    {"timestamp", now_iso8601()}
                                });
                                s["promptTokens"] = s.value("promptTokens", 0) + total_prompt_tok;
                                s["completionTokens"] = s.value("completionTokens", 0) + total_comp_tok;
                                s["costUsd"] = s.value("costUsd", 0.0) + cost_usd;
                                s["updatedAt"] = now_iso8601();
                                updated_session = s;
                                break;
                            }
                        }
                        save_sessions_locked();
                    }

                    int ctx_pct = std::min(100, static_cast<int>(((total_prompt_tok + total_comp_tok) * 100) / 128000));
                    emit_sse("usage", {
                        {"promptTokens", total_prompt_tok},
                        {"completionTokens", total_comp_tok},
                        {"estimatedCostUsd", cost_usd},
                        {"contextPercent", ctx_pct}
                    });
                    emit_sse("done", {
                        {"sessionId", session_id},
                        {"touchedFiles", touched_files},
                        {"session", updated_session}
                    });

                    sink.done();
                    return false;
                }
            );
        });
    }
};

} // namespace aircode
