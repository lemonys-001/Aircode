#pragma once

#include "win_utils.hpp"
#include "json.hpp"
#include <winhttp.h>
#include <functional>
#include <map>

namespace aircode {

using json = nlohmann::json;

struct ParsedUrl {
    bool https = true;
    std::wstring host;
    INTERNET_PORT port = INTERNET_DEFAULT_HTTPS_PORT;
    std::wstring path_and_query = L"/";
};

inline ParsedUrl parse_url_winhttp(const std::string& url_utf8) {
    ParsedUrl out;
    std::wstring wurl = utf8_to_wide(url_utf8);
    URL_COMPONENTS uc{};
    uc.dwStructSize = sizeof(uc);
    wchar_t host_buf[512] = {0};
    wchar_t path_buf[2048] = {0};
    wchar_t extra_buf[2048] = {0};
    uc.lpszHostName = host_buf;
    uc.dwHostNameLength = 512;
    uc.lpszUrlPath = path_buf;
    uc.dwUrlPathLength = 2048;
    uc.lpszExtraInfo = extra_buf;
    uc.dwExtraInfoLength = 2048;

    if (WinHttpCrackUrl(wurl.c_str(), static_cast<DWORD>(wurl.size()), 0, &uc)) {
        out.https = (uc.nScheme == INTERNET_SCHEME_HTTPS);
        out.host.assign(uc.lpszHostName, uc.dwHostNameLength);
        out.port = uc.nPort;
        out.path_and_query.assign(uc.lpszUrlPath, uc.dwUrlPathLength);
        if (uc.dwExtraInfoLength > 0) {
            out.path_and_query.append(uc.lpszExtraInfo, uc.dwExtraInfoLength);
        }
        if (out.path_and_query.empty()) out.path_and_query = L"/";
    }
    return out;
}

struct HttpStreamResponse {
    int status_code = 0;
    std::string raw_body;
    std::string error;
};

inline HttpStreamResponse winhttp_request_stream(
    const std::string& method,
    const std::string& url,
    const std::vector<std::pair<std::string, std::string>>& headers,
    const std::string& body,
    std::function<void(const char*, size_t)> on_chunk,
    std::atomic<bool>* cancel_flag = nullptr,
    int connect_timeout_ms = 10000,
    int read_timeout_ms = 120000
) {
    HttpStreamResponse resp;
    ParsedUrl pu = parse_url_winhttp(url);
    if (pu.host.empty()) {
        resp.error = "Invalid URL: " + url;
        return resp;
    }

    HINTERNET hSession = WinHttpOpen(
        L"Aircode-Studio/1.2 (Windows NT 10.0; Win64; x64)",
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        WINHTTP_NO_PROXY_NAME,
        WINHTTP_NO_PROXY_BYPASS,
        0
    );
    if (!hSession) {
        resp.error = "WinHttpOpen failed";
        return resp;
    }

    WinHttpSetTimeouts(hSession, connect_timeout_ms, connect_timeout_ms, 30000, read_timeout_ms);

    HINTERNET hConnect = WinHttpConnect(hSession, pu.host.c_str(), pu.port, 0);
    if (!hConnect) {
        WinHttpCloseHandle(hSession);
        resp.error = "Failed to connect to " + wide_to_utf8(pu.host);
        return resp;
    }

    DWORD flags = pu.https ? WINHTTP_FLAG_SECURE : 0;
    std::wstring wmethod = utf8_to_wide(method);
    HINTERNET hRequest = WinHttpOpenRequest(
        hConnect,
        wmethod.c_str(),
        pu.path_and_query.c_str(),
        nullptr,
        WINHTTP_NO_REFERER,
        WINHTTP_DEFAULT_ACCEPT_TYPES,
        flags
    );
    if (!hRequest) {
        WinHttpCloseHandle(hConnect);
        WinHttpCloseHandle(hSession);
        resp.error = "WinHttpOpenRequest failed";
        return resp;
    }

    for (const auto& [k, v] : headers) {
        std::wstring hdr = utf8_to_wide(k + ": " + v + "\r\n");
        WinHttpAddRequestHeaders(hRequest, hdr.c_str(), static_cast<DWORD>(hdr.size()), WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE);
    }

    BOOL sent = WinHttpSendRequest(
        hRequest,
        WINHTTP_NO_ADDITIONAL_HEADERS,
        0,
        body.empty() ? WINHTTP_NO_REQUEST_DATA : const_cast<char*>(body.data()),
        static_cast<DWORD>(body.size()),
        static_cast<DWORD>(body.size()),
        0
    );

    if (!sent || !WinHttpReceiveResponse(hRequest, nullptr)) {
        DWORD err = GetLastError();
        WinHttpCloseHandle(hRequest);
        WinHttpCloseHandle(hConnect);
        WinHttpCloseHandle(hSession);
        resp.error = "Network request failed to " + url + " (WinHTTP code " + std::to_string(err) + ")";
        return resp;
    }

    DWORD status = 0;
    DWORD status_sz = sizeof(status);
    WinHttpQueryHeaders(
        hRequest,
        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_HEADER_NAME_BY_INDEX,
        &status,
        &status_sz,
        WINHTTP_NO_HEADER_INDEX
    );
    resp.status_code = static_cast<int>(status);

    char buffer[8192];
    for (;;) {
        if (cancel_flag && cancel_flag->load()) {
            resp.error = "Cancelled by user";
            break;
        }
        DWORD avail = 0;
        if (!WinHttpQueryDataAvailable(hRequest, &avail)) break;
        if (avail == 0) break;

        DWORD to_read = std::min<DWORD>(avail, sizeof(buffer));
        DWORD read_bytes = 0;
        if (!WinHttpReadData(hRequest, buffer, to_read, &read_bytes) || read_bytes == 0) break;

        if (resp.status_code >= 200 && resp.status_code < 300 && on_chunk) {
            on_chunk(buffer, read_bytes);
        } else {
            if (resp.raw_body.size() < 256 * 1024) {
                resp.raw_body.append(buffer, read_bytes);
            }
        }
    }

    WinHttpCloseHandle(hRequest);
    WinHttpCloseHandle(hConnect);
    WinHttpCloseHandle(hSession);
    return resp;
}

struct ToolCallSpec {
    std::string id;
    std::string name;
    json arguments = json::object();
    std::string raw_args;
};

struct LLMStreamResult {
    bool ok = false;
    std::string error;
    std::string content;
    std::string reasoning;
    std::vector<ToolCallSpec> tool_calls;
    int prompt_tokens = 0;
    int completion_tokens = 0;
};

// Parse fallback <tool_call>{"name":"...","arguments":{...}}</tool_call> blocks for local models
inline void extract_xml_tool_calls(std::string& content, std::vector<ToolCallSpec>& out_calls) {
    const std::string open_tag = "<tool_call>";
    const std::string close_tag = "</tool_call>";
    for (;;) {
        size_t s = content.find(open_tag);
        if (s == std::string::npos) break;
        size_t e = content.find(close_tag, s + open_tag.size());
        if (e == std::string::npos) break;
        std::string inner = content.substr(s + open_tag.size(), e - (s + open_tag.size()));
        try {
            auto j = json::parse(inner);
            if (j.contains("name") && j["name"].is_string()) {
                ToolCallSpec tc;
                tc.id = random_id("call");
                tc.name = j["name"].get<std::string>();
                tc.arguments = j.value("arguments", json::object());
                tc.raw_args = tc.arguments.dump();
                out_calls.push_back(std::move(tc));
            }
        } catch (...) {
            break;
        }
        content.erase(s, (e + close_tag.size()) - s);
    }
}

inline LLMStreamResult stream_llm_completion(
    const std::string& provider,
    const std::string& base_url,
    const std::string& api_key,
    const std::string& model,
    const std::string& system_prompt,
    const json& messages,
    const json& openai_tools,
    std::function<void(const std::string&)> on_text_delta,
    std::function<void(const std::string&)> on_reasoning_delta,
    std::atomic<bool>* cancel_flag = nullptr,
    bool force_xml_tools = false
) {
    LLMStreamResult result;
    std::string trimmed_base = base_url;
    while (!trimmed_base.empty() && trimmed_base.back() == '/') trimmed_base.pop_back();

    // ==================== ANTHROPIC NATIVE /v1/messages ====================
    if (provider == "anthropic") {
        std::string url = (trimmed_base.empty() ? "https://api.anthropic.com" : trimmed_base);
        if (url.find("/v1/messages") == std::string::npos) {
            if (url.size() >= 3 && url.substr(url.size() - 3) == "/v1") url += "/messages";
            else url += "/v1/messages";
        }

        json anth_tools = json::array();
        for (const auto& t : openai_tools) {
            if (t.contains("function")) {
                const auto& fn = t["function"];
                anth_tools.push_back({
                    {"name", fn.value("name", "")},
                    {"description", fn.value("description", "")},
                    {"input_schema", fn.value("parameters", json::object())}
                });
            }
        }

        json anth_msgs = json::array();
        for (const auto& m : messages) {
            std::string role = m.value("role", "user");
            if (role == "system") continue;
            if (role == "tool") {
                json block = {
                    {"type", "tool_result"},
                    {"tool_use_id", m.value("tool_call_id", "")},
                    {"content", m.value("content", "")}
                };
                if (!anth_msgs.empty() && anth_msgs.back()["role"] == "user" && anth_msgs.back()["content"].is_array()) {
                    anth_msgs.back()["content"].push_back(block);
                } else {
                    anth_msgs.push_back({{"role", "user"}, {"content", json::array({block})}});
                }
            } else if (role == "assistant" && m.contains("tool_calls") && m["tool_calls"].is_array() && !m["tool_calls"].empty()) {
                json blocks = json::array();
                std::string txt = m.value("content", "");
                if (!txt.empty()) blocks.push_back({{"type", "text"}, {"text", txt}});
                for (const auto& tc : m["tool_calls"]) {
                    json parsed_in = json::object();
                    try { parsed_in = json::parse(tc["function"].value("arguments", "{}")); } catch (...) {}
                    blocks.push_back({
                        {"type", "tool_use"},
                        {"id", tc.value("id", random_id("toolu"))},
                        {"name", tc["function"].value("name", "")},
                        {"input", parsed_in}
                    });
                }
                anth_msgs.push_back({{"role", "assistant"}, {"content", blocks}});
            } else {
                anth_msgs.push_back({
                    {"role", role == "assistant" ? "assistant" : "user"},
                    {"content", m.value("content", "")}
                });
            }
        }

        json payload = {
            {"model", model},
            {"max_tokens", 8192},
            {"system", system_prompt},
            {"messages", anth_msgs},
            {"stream", true}
        };
        if (!anth_tools.empty()) payload["tools"] = anth_tools;

        std::string sse_buf;
        std::map<int, ToolCallSpec> active_tools;

        auto http_res = winhttp_request_stream(
            "POST",
            url,
            {
                {"Content-Type", "application/json"},
                {"x-api-key", api_key},
                {"anthropic-version", "2023-06-01"}
            },
            payload.dump(),
            [&](const char* data, size_t len) {
                sse_buf.append(data, len);
                size_t nl = 0;
                while ((nl = sse_buf.find('\n')) != std::string::npos) {
                    std::string line = sse_buf.substr(0, nl);
                    sse_buf.erase(0, nl + 1);
                    if (!line.empty() && line.back() == '\r') line.pop_back();
                    if (line.rfind("data:", 0) != 0) continue;
                    std::string json_str = line.substr(5);
                    while (!json_str.empty() && json_str.front() == ' ') json_str.erase(0, 1);
                    if (json_str.empty() || json_str == "[DONE]") continue;
                    try {
                        auto ev = json::parse(json_str);
                        std::string type = ev.value("type", "");
                        if (type == "message_start" && ev.contains("message") && ev["message"].contains("usage")) {
                            result.prompt_tokens += ev["message"]["usage"].value("input_tokens", 0);
                        } else if (type == "content_block_start" && ev.contains("content_block")) {
                            int idx = ev.value("index", 0);
                            const auto& cb = ev["content_block"];
                            if (cb.value("type", "") == "tool_use") {
                                ToolCallSpec tc;
                                tc.id = cb.value("id", random_id("toolu"));
                                tc.name = cb.value("name", "");
                                active_tools[idx] = tc;
                            }
                        } else if (type == "content_block_delta" && ev.contains("delta")) {
                            int idx = ev.value("index", 0);
                            const auto& d = ev["delta"];
                            std::string dtype = d.value("type", "");
                            if (dtype == "text_delta") {
                                std::string t = d.value("text", "");
                                result.content += t;
                                if (on_text_delta && !t.empty()) on_text_delta(t);
                            } else if (dtype == "thinking_delta") {
                                std::string th = d.value("thinking", "");
                                result.reasoning += th;
                                if (on_reasoning_delta && !th.empty()) on_reasoning_delta(th);
                            } else if (dtype == "input_json_delta") {
                                active_tools[idx].raw_args += d.value("partial_json", "");
                            }
                        } else if (type == "message_delta" && ev.contains("usage")) {
                            result.completion_tokens += ev["usage"].value("output_tokens", 0);
                        }
                    } catch (...) {}
                }
            },
            cancel_flag
        );

        if (!http_res.error.empty()) {
            result.error = http_res.error;
            return result;
        }
        if (http_res.status_code < 200 || http_res.status_code >= 300) {
            result.error = "Anthropic HTTP " + std::to_string(http_res.status_code) + ": " + http_res.raw_body;
            return result;
        }
        for (auto& [idx, tc] : active_tools) {
            if (!tc.raw_args.empty()) {
                try { tc.arguments = json::parse(tc.raw_args); } catch (...) {}
            }
            result.tool_calls.push_back(std::move(tc));
        }
        result.ok = true;
        return result;
    }

    // ==================== OPENAI-COMPATIBLE /v1/chat/completions ====================
    std::string endpoint = trimmed_base;
    if (endpoint.empty()) {
        if (provider == "ollama") endpoint = "http://127.0.0.1:11434/v1";
        else if (provider == "lmstudio") endpoint = "http://127.0.0.1:1234/v1";
        else if (provider == "openrouter") endpoint = "https://openrouter.ai/api/v1";
        else if (provider == "gemini") endpoint = "https://generativelanguage.googleapis.com/v1beta/openai";
        else if (provider == "groq") endpoint = "https://api.groq.com/openai/v1";
        else if (provider == "deepseek") endpoint = "https://api.deepseek.com/v1";
        else if (provider == "github") endpoint = "https://models.inference.ai.azure.com";
        else endpoint = "https://api.openai.com/v1";
    }
    if (endpoint.find("/chat/completions") == std::string::npos) {
        endpoint += "/chat/completions";
    }

    std::string effective_sys = system_prompt;
    if (force_xml_tools && !openai_tools.empty()) {
        effective_sys += "\n\nIMPORTANT TOOL FORMAT: When you need to call a tool, output ONLY an XML block in this exact format:\n"
                         "<tool_call>{\"name\": \"tool_name\", \"arguments\": {\"arg1\": \"value\"}}</tool_call>\n"
                         "Available tools JSON schema:\n" + openai_tools.dump();
    }

    json full_msgs = json::array();
    full_msgs.push_back({{"role", "system"}, {"content", effective_sys}});
    for (const auto& m : messages) {
        if (force_xml_tools && m.value("role", "") == "tool") {
            full_msgs.push_back({
                {"role", "user"},
                {"content", "[Tool Result " + m.value("name", "") + "]:\n" + m.value("content", "")}
            });
        } else if (force_xml_tools && m.value("role", "") == "assistant" && m.contains("tool_calls")) {
            full_msgs.push_back({
                {"role", "assistant"},
                {"content", m.value("content", "")}
            });
        } else {
            full_msgs.push_back(m);
        }
    }

    json payload = {
        {"model", model},
        {"messages", full_msgs},
        {"stream", true}
    };
    if (!force_xml_tools && !openai_tools.empty()) {
        payload["tools"] = openai_tools;
    }

    std::vector<std::pair<std::string, std::string>> headers = {
        {"Content-Type", "application/json"}
    };
    if (!api_key.empty()) {
        headers.push_back({"Authorization", "Bearer " + api_key});
    }
    if (provider == "openrouter") {
        headers.push_back({"HTTP-Referer", "https://github.com/lemonys-001/Aircode"});
        headers.push_back({"X-Title", "Aircode Studio Windows"});
    }

    std::string sse_buf;
    std::map<int, ToolCallSpec> tool_map;
    bool in_think_tag = false;
    std::string tag_carry;

    auto http_res = winhttp_request_stream(
        "POST",
        endpoint,
        headers,
        payload.dump(),
        [&](const char* data, size_t len) {
            sse_buf.append(data, len);
            size_t nl = 0;
            while ((nl = sse_buf.find('\n')) != std::string::npos) {
                std::string line = sse_buf.substr(0, nl);
                sse_buf.erase(0, nl + 1);
                if (!line.empty() && line.back() == '\r') line.pop_back();
                if (line.rfind("data:", 0) != 0) continue;
                std::string json_str = line.substr(5);
                while (!json_str.empty() && json_str.front() == ' ') json_str.erase(0, 1);
                if (json_str.empty() || json_str == "[DONE]") continue;

                try {
                    auto chunk = json::parse(json_str);
                    if (chunk.contains("usage") && chunk["usage"].is_object()) {
                        result.prompt_tokens = chunk["usage"].value("prompt_tokens", result.prompt_tokens);
                        result.completion_tokens = chunk["usage"].value("completion_tokens", result.completion_tokens);
                    }
                    if (!chunk.contains("choices") || !chunk["choices"].is_array() || chunk["choices"].empty()) continue;
                    const auto& delta = chunk["choices"][0].value("delta", json::object());

                    // Reasoning tokens (DeepSeek R1, OpenRouter, Groq QwQ)
                    std::string r_tok;
                    if (delta.contains("reasoning_content") && delta["reasoning_content"].is_string()) {
                        r_tok = delta["reasoning_content"].get<std::string>();
                    } else if (delta.contains("reasoning") && delta["reasoning"].is_string()) {
                        r_tok = delta["reasoning"].get<std::string>();
                    }
                    if (!r_tok.empty()) {
                        result.reasoning += r_tok;
                        if (on_reasoning_delta) on_reasoning_delta(r_tok);
                    }

                    // Content tokens (with inline <think>...</think> splitter for Ollama / QwQ)
                    if (delta.contains("content") && delta["content"].is_string()) {
                        std::string c_tok = delta["content"].get<std::string>();
                        if (!c_tok.empty()) {
                            if (result.content.empty() && result.reasoning.empty() && c_tok.find("<think>") != std::string::npos) {
                                in_think_tag = true;
                                auto pos = c_tok.find("<think>");
                                c_tok = c_tok.substr(pos + 7);
                            }
                            if (in_think_tag) {
                                auto end_p = c_tok.find("</think>");
                                if (end_p != std::string::npos) {
                                    std::string r_part = c_tok.substr(0, end_p);
                                    std::string c_part = c_tok.substr(end_p + 8);
                                    in_think_tag = false;
                                    if (!r_part.empty()) {
                                        result.reasoning += r_part;
                                        if (on_reasoning_delta) on_reasoning_delta(r_part);
                                    }
                                    if (!c_part.empty()) {
                                        result.content += c_part;
                                        if (on_text_delta) on_text_delta(c_part);
                                    }
                                } else {
                                    result.reasoning += c_tok;
                                    if (on_reasoning_delta) on_reasoning_delta(c_tok);
                                }
                            } else {
                                result.content += c_tok;
                                if (on_text_delta) on_text_delta(c_tok);
                            }
                        }
                    }

                    // Tool calls delta accumulation
                    if (delta.contains("tool_calls") && delta["tool_calls"].is_array()) {
                        for (const auto& tc_delta : delta["tool_calls"]) {
                            int idx = tc_delta.value("index", 0);
                            auto& spec = tool_map[idx];
                            if (tc_delta.contains("id") && tc_delta["id"].is_string()) {
                                spec.id = tc_delta["id"].get<std::string>();
                            }
                            if (tc_delta.contains("function") && tc_delta["function"].is_object()) {
                                const auto& fn = tc_delta["function"];
                                if (fn.contains("name") && fn["name"].is_string()) {
                                    spec.name += fn["name"].get<std::string>();
                                }
                                if (fn.contains("arguments") && fn["arguments"].is_string()) {
                                    spec.raw_args += fn["arguments"].get<std::string>();
                                }
                            }
                        }
                    }
                } catch (...) {}
            }
        },
        cancel_flag
    );

    // Automatic retry with XML tools if model rejected native `tools` schema
    if ((http_res.status_code == 400 || http_res.status_code == 422) && !force_xml_tools && !openai_tools.empty()) {
        std::string lower_err = to_lower_ascii(http_res.raw_body);
        if (lower_err.find("tool") != std::string::npos || lower_err.find("function") != std::string::npos) {
            return stream_llm_completion(
                provider, base_url, api_key, model, system_prompt,
                messages, openai_tools, on_text_delta, on_reasoning_delta, cancel_flag, true
            );
        }
    }

    if (!http_res.error.empty()) {
        result.error = http_res.error;
        return result;
    }
    if (http_res.status_code < 200 || http_res.status_code >= 300) {
        result.error = "HTTP " + std::to_string(http_res.status_code) + " from " + endpoint + ": " + http_res.raw_body;
        return result;
    }

    for (auto& [idx, spec] : tool_map) {
        if (spec.id.empty()) spec.id = random_id("call");
        if (!spec.raw_args.empty()) {
            try { spec.arguments = json::parse(spec.raw_args); } catch (...) {}
        }
        if (!spec.name.empty()) {
            result.tool_calls.push_back(std::move(spec));
        }
    }

    if (result.tool_calls.empty() && result.content.find("<tool_call>") != std::string::npos) {
        extract_xml_tool_calls(result.content, result.tool_calls);
    }

    if (result.prompt_tokens == 0) {
        result.prompt_tokens = static_cast<int>((system_prompt.size() + payload.dump().size()) / 4);
    }
    if (result.completion_tokens == 0) {
        result.completion_tokens = static_cast<int>((result.content.size() + result.reasoning.size()) / 4);
    }

    result.ok = true;
    return result;
}

inline json probe_local_ai_servers() {
    json detected = json::object();

    // Probe Ollama (127.0.0.1:11434)
    {
        std::string body;
        auto r = winhttp_request_stream(
            "GET", "http://127.0.0.1:11434/api/tags", {}, "",
            [&](const char* d, size_t n) { body.append(d, n); },
            nullptr, 350, 800
        );
        if (r.status_code == 200) {
            json models = json::array();
            try {
                auto j = json::parse(body);
                if (j.contains("models") && j["models"].is_array()) {
                    for (const auto& m : j["models"]) {
                        models.push_back(m.value("name", ""));
                    }
                }
            } catch (...) {}
            detected["ollama"] = {{"online", true}, {"models", models}, {"url", "http://127.0.0.1:11434/v1"}};
        } else {
            detected["ollama"] = {{"online", false}, {"models", json::array()}, {"url", "http://127.0.0.1:11434/v1"}};
        }
    }

    // Probe LM Studio (127.0.0.1:1234)
    {
        std::string body;
        auto r = winhttp_request_stream(
            "GET", "http://127.0.0.1:1234/v1/models", {}, "",
            [&](const char* d, size_t n) { body.append(d, n); },
            nullptr, 300, 700
        );
        if (r.status_code == 200) {
            json models = json::array();
            try {
                auto j = json::parse(body);
                if (j.contains("data") && j["data"].is_array()) {
                    for (const auto& m : j["data"]) {
                        models.push_back(m.value("id", ""));
                    }
                }
            } catch (...) {}
            detected["lmstudio"] = {{"online", true}, {"models", models}, {"url", "http://127.0.0.1:1234/v1"}};
        } else {
            detected["lmstudio"] = {{"online", false}, {"models", json::array()}, {"url", "http://127.0.0.1:1234/v1"}};
        }
    }

    return detected;
}

} // namespace aircode
