#pragma once

#include "win_utils.hpp"
#include "json.hpp"

#include <map>
#include <set>
#include <unordered_set>

namespace aircode {

using json = nlohmann::json;

inline bool is_ignored_dir(const std::string& name) {
    static const std::unordered_set<std::string> ignored = {
        ".git", "node_modules", ".next", ".nuxt", "dist", "build", "target",
        "out", ".venv", "venv", "__pycache__", ".pytest_cache", ".mypy_cache",
        ".cache", ".turbo", ".idea", ".vs", "bin", "obj", ".aircode"
    };
    return ignored.count(name) > 0;
}

inline bool is_likely_binary_ext(const std::string& ext_lower) {
    static const std::unordered_set<std::string> bins = {
        ".exe", ".dll", ".pdb", ".obj", ".o", ".a", ".lib", ".so", ".dylib",
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".bmp", ".pdf",
        ".zip", ".tar", ".gz", ".7z", ".rar", ".woff", ".woff2", ".ttf",
        ".mp3", ".mp4", ".wav", ".ogg", ".wasm", ".pyc", ".class"
    };
    return bins.count(ext_lower) > 0;
}

inline std::string to_lower_ascii(std::string s) {
    for (char& c : s) {
        if (c >= 'A' && c <= 'Z') c = static_cast<char>(c - 'A' + 'a');
    }
    return s;
}

inline std::string detect_language(const std::string& rel_path) {
    auto dot = rel_path.find_last_of('.');
    if (dot == std::string::npos) return "plaintext";
    std::string ext = to_lower_ascii(rel_path.substr(dot));
    if (ext == ".ts" || ext == ".tsx") return "typescript";
    if (ext == ".js" || ext == ".jsx" || ext == ".mjs" || ext == ".cjs") return "javascript";
    if (ext == ".py") return "python";
    if (ext == ".cpp" || ext == ".cc" || ext == ".cxx" || ext == ".hpp" || ext == ".h" || ext == ".c") return "cpp";
    if (ext == ".rs") return "rust";
    if (ext == ".go") return "go";
    if (ext == ".cs") return "csharp";
    if (ext == ".java") return "java";
    if (ext == ".json") return "json";
    if (ext == ".html" || ext == ".htm") return "html";
    if (ext == ".css" || ext == ".scss") return "css";
    if (ext == ".md" || ext == ".markdown") return "markdown";
    if (ext == ".ps1" || ext == ".bat" || ext == ".cmd") return "powershell";
    if (ext == ".sh" || ext == ".bash") return "shell";
    if (ext == ".yml" || ext == ".yaml") return "yaml";
    if (ext == ".toml") return "toml";
    if (ext == ".sql") return "sql";
    return "plaintext";
}

struct FileBackupEntry {
    bool existed = false;
    std::string original_content;
    std::string new_content;
};

struct Checkpoint {
    std::string id;
    std::string label;
    std::string created_at;
    std::map<std::string, FileBackupEntry> files; // rel_path -> backup
};

class WorkspaceManager {
public:
    explicit WorkspaceManager(const std::string& initial_root) {
        set_workspace(initial_root);
    }

    void set_workspace(const std::string& path_utf8) {
        std::lock_guard<std::mutex> lk(mu_);
        std::error_code ec;
        fs::path p = u8path_safe(path_utf8);
        if (p.empty() || !fs::exists(p, ec)) {
            p = fs::current_path(ec);
        }
        p = fs::weakly_canonical(p, ec);
        root_ = p;
    }

    std::string root_utf8() const {
        std::lock_guard<std::mutex> lk(mu_);
        return path_to_native_utf8(root_);
    }

    fs::path resolve_safe(const std::string& rel_or_abs, std::string* clean_rel_out = nullptr) const {
        std::lock_guard<std::mutex> lk(mu_);
        std::error_code ec;
        fs::path input = u8path_safe(rel_or_abs);
        fs::path full = input.is_absolute() ? fs::weakly_canonical(input, ec) : fs::weakly_canonical(root_ / input, ec);
        if (full.empty()) {
            full = input.is_absolute() ? input : (root_ / input);
        }
        fs::path rel = fs::relative(full, root_, ec);
        std::string rel_str = path_to_utf8(rel);
        if (clean_rel_out) *clean_rel_out = rel_str;
        return full;
    }

    // Begin or return active checkpoint for a turn
    std::string create_checkpoint(const std::string& label) {
        std::lock_guard<std::mutex> lk(mu_);
        Checkpoint cp;
        cp.id = random_id("chk");
        cp.label = label.empty() ? "Agent Edit Checkpoint" : label;
        cp.created_at = now_iso8601();
        checkpoints_.push_back(std::move(cp));
        if (checkpoints_.size() > 40) {
            checkpoints_.erase(checkpoints_.begin());
        }
        return checkpoints_.back().id;
    }

    void record_file_before_write(const std::string& checkpoint_id, const std::string& rel_path, const fs::path& full_path) {
        std::lock_guard<std::mutex> lk(mu_);
        if (checkpoint_id.empty()) return;
        for (auto& cp : checkpoints_) {
            if (cp.id == checkpoint_id) {
                if (cp.files.find(rel_path) == cp.files.end()) {
                    FileBackupEntry entry;
                    std::error_code ec;
                    entry.existed = fs::exists(full_path, ec) && fs::is_regular_file(full_path, ec);
                    if (entry.existed) {
                        entry.original_content = read_text_file(full_path);
                    }
                    cp.files[rel_path] = std::move(entry);
                }
                break;
            }
        }
    }

    void record_file_after_write(const std::string& checkpoint_id, const std::string& rel_path, const std::string& new_content) {
        std::lock_guard<std::mutex> lk(mu_);
        if (checkpoint_id.empty()) return;
        for (auto& cp : checkpoints_) {
            if (cp.id == checkpoint_id) {
                auto it = cp.files.find(rel_path);
                if (it != cp.files.end()) {
                    it->second.new_content = new_content;
                }
                break;
            }
        }
    }

    json list_checkpoints_json() const {
        std::lock_guard<std::mutex> lk(mu_);
        json arr = json::array();
        for (auto it = checkpoints_.rbegin(); it != checkpoints_.rend(); ++it) {
            if (it->files.empty()) continue;
            json files_arr = json::array();
            for (const auto& [rel, entry] : it->files) {
                files_arr.push_back({
                    {"path", rel},
                    {"existed", entry.existed},
                    {"diff", compute_unified_diff(rel, entry.original_content, entry.new_content)}
                });
            }
            arr.push_back({
                {"id", it->id},
                {"label", it->label},
                {"createdAt", it->created_at},
                {"fileCount", it->files.size()},
                {"files", files_arr}
            });
        }
        return arr;
    }

    json restore_checkpoint(const std::string& checkpoint_id) {
        std::map<std::string, FileBackupEntry> to_restore;
        std::string label;
        fs::path root_copy;
        {
            std::lock_guard<std::mutex> lk(mu_);
            root_copy = root_;
            std::string target_id = checkpoint_id;
            if (target_id.empty() || target_id == "latest") {
                for (auto it = checkpoints_.rbegin(); it != checkpoints_.rend(); ++it) {
                    if (!it->files.empty()) {
                        target_id = it->id;
                        break;
                    }
                }
            }
            for (const auto& cp : checkpoints_) {
                if (cp.id == target_id) {
                    to_restore = cp.files;
                    label = cp.label;
                    break;
                }
            }
        }
        if (to_restore.empty()) {
            return {{"ok", false}, {"error", "No matching checkpoint with modified files found."}};
        }
        json restored_files = json::array();
        std::error_code ec;
        for (const auto& [rel, entry] : to_restore) {
            fs::path full = root_copy / u8path_safe(rel);
            if (entry.existed) {
                write_text_file(full, entry.original_content);
            } else {
                fs::remove(full, ec);
            }
            restored_files.push_back(rel);
        }
        return {
            {"ok", true},
            {"label", label},
            {"restoredFiles", restored_files}
        };
    }

    // Scans workspace files + builds compact Repo Map symbol index
    json scan_workspace(int max_files = 1500) const {
        fs::path root_copy;
        {
            std::lock_guard<std::mutex> lk(mu_);
            root_copy = root_;
        }

        json files = json::array();
        json repo_map = json::array();
        std::error_code ec;
        int count = 0;

        auto iter = fs::recursive_directory_iterator(
            root_copy,
            fs::directory_options::skip_permission_denied,
            ec
        );

        for (auto it = fs::begin(iter); it != fs::end(iter); it.increment(ec)) {
            if (ec) {
                ec.clear();
                continue;
            }
            const fs::path& p = it->path();
            std::string fname = path_to_utf8(p.filename());
            if (it->is_directory(ec)) {
                if (is_ignored_dir(fname)) {
                    it.disable_recursion_pending();
                }
                continue;
            }
            if (!it->is_regular_file(ec)) continue;

            std::string rel = path_to_utf8(fs::relative(p, root_copy, ec));
            auto sz = it->file_size(ec);
            std::string ext = p.has_extension() ? to_lower_ascii(path_to_utf8(p.extension())) : "";
            bool binary = is_likely_binary_ext(ext);

            files.push_back({
                {"path", rel},
                {"name", fname},
                {"size", ec ? 0 : static_cast<uint64_t>(sz)},
                {"language", detect_language(rel)},
                {"binary", binary}
            });

            // Extract structural Repo Map symbols for top code files (< 128 KB)
            if (!binary && sz > 0 && sz < 131072 && repo_map.size() < 80) {
                std::string lang = detect_language(rel);
                if (lang == "typescript" || lang == "javascript" || lang == "python" ||
                    lang == "cpp" || lang == "rust" || lang == "go" || lang == "csharp" || lang == "java") {
                    std::string text = read_text_file(p);
                    auto syms = extract_symbols(text, lang);
                    if (!syms.empty()) {
                        repo_map.push_back({
                            {"path", rel},
                            {"symbols", syms}
                        });
                    }
                }
            }

            if (++count >= max_files) break;
        }

        return {
            {"root", path_to_native_utf8(root_copy)},
            {"count", count},
            {"files", files},
            {"repoMap", repo_map}
        };
    }

    static std::vector<std::string> extract_symbols(const std::string& content, const std::string& lang) {
        std::vector<std::string> out;
        auto lines = split_lines(content);
        for (size_t i = 0; i < lines.size() && out.size() < 12; ++i) {
            const std::string& ln = lines[i];
            size_t first = ln.find_first_not_of(" \t");
            if (first == std::string::npos || first > 4) continue;
            std::string trimmed = ln.substr(first);
            if (trimmed.rfind("//", 0) == 0 || trimmed.rfind("#", 0) == 0 || trimmed.rfind("/*", 0) == 0) continue;

            bool hit = false;
            if (lang == "python") {
                hit = trimmed.rfind("def ", 0) == 0 || trimmed.rfind("async def ", 0) == 0 || trimmed.rfind("class ", 0) == 0;
            } else if (lang == "typescript" || lang == "javascript") {
                hit = trimmed.rfind("export function ", 0) == 0 || trimmed.rfind("export async function ", 0) == 0 ||
                      trimmed.rfind("export class ", 0) == 0 || trimmed.rfind("export interface ", 0) == 0 ||
                      trimmed.rfind("export type ", 0) == 0 || trimmed.rfind("function ", 0) == 0 ||
                      trimmed.rfind("class ", 0) == 0;
            } else if (lang == "cpp" || lang == "csharp" || lang == "java") {
                hit = trimmed.rfind("class ", 0) == 0 || trimmed.rfind("struct ", 0) == 0 ||
                      trimmed.rfind("namespace ", 0) == 0 || trimmed.rfind("inline ", 0) == 0;
            } else if (lang == "rust") {
                hit = trimmed.rfind("pub fn ", 0) == 0 || trimmed.rfind("fn ", 0) == 0 ||
                      trimmed.rfind("pub struct ", 0) == 0 || trimmed.rfind("struct ", 0) == 0 ||
                      trimmed.rfind("pub enum ", 0) == 0 || trimmed.rfind("impl ", 0) == 0;
            } else if (lang == "go") {
                hit = trimmed.rfind("func ", 0) == 0 || trimmed.rfind("type ", 0) == 0;
            }
            if (hit) {
                std::string sig = trimmed.substr(0, std::min<size_t>(trimmed.size(), 88));
                if (!sig.empty() && (sig.back() == '{' || sig.back() == ':')) sig.pop_back();
                out.push_back("L" + std::to_string(i + 1) + ": " + sig);
            }
        }
        return out;
    }

    json tool_read_file(const std::string& path_arg, int offset = 1, int limit = 300) const {
        std::string rel;
        fs::path full = resolve_safe(path_arg, &rel);
        bool ok = false;
        std::string content = read_text_file(full, &ok);
        if (!ok) {
            return {{"ok", false}, {"error", "File not found or unreadable: " + path_arg}};
        }
        auto lines = split_lines(content);
        int start_idx = std::max(0, offset - 1);
        int end_idx = std::min(static_cast<int>(lines.size()), start_idx + std::max(1, limit));
        std::ostringstream numbered;
        for (int i = start_idx; i < end_idx; ++i) {
            numbered << std::setw(4) << (i + 1) << " | " << lines[i] << "\n";
        }
        return {
            {"ok", true},
            {"path", rel},
            {"totalLines", lines.size()},
            {"shownRange", std::to_string(start_idx + 1) + "-" + std::to_string(end_idx)},
            {"content", numbered.str()},
            {"rawContent", content}
        };
    }

    json tool_write_file(const std::string& path_arg, const std::string& content, const std::string& checkpoint_id = "") {
        std::string rel;
        fs::path full = resolve_safe(path_arg, &rel);
        bool existed = false;
        std::string old_content = read_text_file(full, &existed);

        record_file_before_write(checkpoint_id, rel, full);
        if (!write_text_file(full, content)) {
            return {{"ok", false}, {"error", "Failed to write file: " + rel}};
        }
        record_file_after_write(checkpoint_id, rel, content);

        std::string diff = compute_unified_diff(rel, old_content, content);
        return {
            {"ok", true},
            {"path", rel},
            {"created", !existed},
            {"bytes", content.size()},
            {"diff", diff}
        };
    }

    // CRLF-resilient surgical search & replace for Windows and POSIX files
    json tool_edit_file(const std::string& path_arg, const std::string& old_text, const std::string& new_text, const std::string& checkpoint_id = "") {
        std::string rel;
        fs::path full = resolve_safe(path_arg, &rel);
        bool ok = false;
        std::string raw = read_text_file(full, &ok);
        if (!ok) {
            return {{"ok", false}, {"error", "File does not exist: " + rel}};
        }
        if (old_text.empty()) {
            return {{"ok", false}, {"error", "old_text must not be empty"}};
        }

        bool uses_crlf = raw.find("\r\n") != std::string::npos;
        auto normalize_lf = [](const std::string& s) {
            std::string out;
            out.reserve(s.size());
            for (size_t i = 0; i < s.size(); ++i) {
                if (s[i] == '\r' && i + 1 < s.size() && s[i + 1] == '\n') continue;
                out.push_back(s[i]);
            }
            return out;
        };
        auto to_crlf = [](const std::string& s) {
            std::string out;
            out.reserve(s.size() + 64);
            for (size_t i = 0; i < s.size(); ++i) {
                if (s[i] == '\n' && (i == 0 || s[i - 1] != '\r')) {
                    out.push_back('\r');
                }
                out.push_back(s[i]);
            }
            return out;
        };

        std::string norm_file = normalize_lf(raw);
        std::string norm_old = normalize_lf(old_text);
        std::string norm_new = normalize_lf(new_text);

        size_t pos = norm_file.find(norm_old);
        if (pos == std::string::npos) {
            return {{"ok", false}, {"error", "old_text was not found in " + rel + ". Read the file first to verify exact lines."}};
        }

        std::string updated_lf = norm_file.substr(0, pos) + norm_new + norm_file.substr(pos + norm_old.size());
        std::string final_bytes = uses_crlf ? to_crlf(updated_lf) : updated_lf;

        record_file_before_write(checkpoint_id, rel, full);
        if (!write_text_file(full, final_bytes)) {
            return {{"ok", false}, {"error", "Failed writing updated file: " + rel}};
        }
        record_file_after_write(checkpoint_id, rel, final_bytes);

        std::string diff = compute_unified_diff(rel, norm_file, updated_lf);
        return {
            {"ok", true},
            {"path", rel},
            {"diff", diff}
        };
    }

    json tool_grep_search(const std::string& query, bool case_sensitive = false, const std::string& file_pattern = "", int max_results = 80) const {
        if (query.empty()) return {{"ok", true}, {"matches", json::array()}};

        fs::path root_copy;
        {
            std::lock_guard<std::mutex> lk(mu_);
            root_copy = root_;
        }

        std::string q = case_sensitive ? query : to_lower_ascii(query);
        std::string pat = to_lower_ascii(file_pattern);
        json matches = json::array();
        std::error_code ec;

        auto iter = fs::recursive_directory_iterator(root_copy, fs::directory_options::skip_permission_denied, ec);
        for (auto it = fs::begin(iter); it != fs::end(iter); it.increment(ec)) {
            if (ec) { ec.clear(); continue; }
            const fs::path& p = it->path();
            std::string fname = path_to_utf8(p.filename());
            if (it->is_directory(ec)) {
                if (is_ignored_dir(fname)) it.disable_recursion_pending();
                continue;
            }
            if (!it->is_regular_file(ec)) continue;
            std::string ext = p.has_extension() ? to_lower_ascii(path_to_utf8(p.extension())) : "";
            if (is_likely_binary_ext(ext)) continue;
            auto sz = it->file_size(ec);
            if (ec || sz > 1024 * 1024) continue;

            std::string rel = path_to_utf8(fs::relative(p, root_copy, ec));
            if (!pat.empty() && to_lower_ascii(rel).find(pat) == std::string::npos) continue;

            std::string content = read_text_file(p);
            auto lines = split_lines(content);
            for (size_t i = 0; i < lines.size(); ++i) {
                std::string hay = case_sensitive ? lines[i] : to_lower_ascii(lines[i]);
                size_t col = hay.find(q);
                if (col != std::string::npos) {
                    std::string preview = lines[i];
                    if (preview.size() > 220) preview = preview.substr(0, 220) + "...";
                    matches.push_back({
                        {"file", rel},
                        {"line", i + 1},
                        {"column", col + 1},
                        {"preview", preview}
                    });
                    if (static_cast<int>(matches.size()) >= max_results) break;
                }
            }
            if (static_cast<int>(matches.size()) >= max_results) break;
        }

        return {
            {"ok", true},
            {"query", query},
            {"count", matches.size()},
            {"matches", matches}
        };
    }

    json git_status() const {
        std::string cwd = root_utf8();
        auto res = run_process_silent("git status --porcelain -b", cwd, "cmd", 8000);
        if (res.exit_code != 0) {
            return {{"isRepo", false}, {"branch", ""}, {"files", json::array()}, {"commits", json::array()}};
        }
        auto lines = split_lines(res.stdout_text);
        std::string branch = "main";
        json changed = json::array();
        for (size_t i = 0; i < lines.size(); ++i) {
            const std::string& ln = lines[i];
            if (i == 0 && ln.rfind("## ", 0) == 0) {
                std::string b = ln.substr(3);
                auto dots = b.find("...");
                branch = dots != std::string::npos ? b.substr(0, dots) : b;
                continue;
            }
            if (ln.size() < 4) continue;
            std::string xy = ln.substr(0, 2);
            std::string file = ln.substr(3);
            changed.push_back({
                {"status", xy},
                {"staged", xy[0] != ' ' && xy[0] != '?'},
                {"path", file}
            });
        }
        auto log_res = run_process_silent("git log -n 10 --pretty=format:\"%h|%s|%cr\"", cwd, "cmd", 6000);
        json commits = json::array();
        if (log_res.exit_code == 0) {
            for (const auto& cl : split_lines(log_res.stdout_text)) {
                auto p1 = cl.find('|');
                auto p2 = p1 != std::string::npos ? cl.find('|', p1 + 1) : std::string::npos;
                if (p1 != std::string::npos && p2 != std::string::npos) {
                    commits.push_back({
                        {"hash", cl.substr(0, p1)},
                        {"subject", cl.substr(p1 + 1, p2 - p1 - 1)},
                        {"ago", cl.substr(p2 + 1)}
                    });
                }
            }
        }
        return {
            {"isRepo", true},
            {"branch", branch},
            {"dirtyCount", changed.size()},
            {"files", changed},
            {"commits", commits}
        };
    }

    json git_diff(const std::string& path_filter = "") const {
        std::string cwd = root_utf8();
        std::string cmd = "git diff HEAD";
        if (!path_filter.empty()) {
            cmd += " -- \"" + path_filter + "\"";
        }
        auto res = run_process_silent(cmd, cwd, "cmd", 10000);
        if (res.exit_code != 0 || res.stdout_text.empty()) {
            std::string fallback_cmd = "git diff";
            if (!path_filter.empty()) fallback_cmd += " -- \"" + path_filter + "\"";
            res = run_process_silent(fallback_cmd, cwd, "cmd", 10000);
        }
        return {
            {"ok", true},
            {"diff", res.stdout_text}
        };
    }

    json scan_diagnostics() const {
        fs::path root_copy;
        {
            std::lock_guard<std::mutex> lk(mu_);
            root_copy = root_;
        }
        json items = json::array();
        std::error_code ec;

        // 1. Built-in ultra-fast static checks (JSON syntax, Git merge conflict markers)
        auto iter = fs::recursive_directory_iterator(root_copy, fs::directory_options::skip_permission_denied, ec);
        for (auto it = fs::begin(iter); it != fs::end(iter); it.increment(ec)) {
            if (ec) { ec.clear(); continue; }
            const fs::path& p = it->path();
            std::string fname = path_to_utf8(p.filename());
            if (it->is_directory(ec)) {
                if (is_ignored_dir(fname)) it.disable_recursion_pending();
                continue;
            }
            if (!it->is_regular_file(ec)) continue;
            auto sz = it->file_size(ec);
            if (ec || sz > 512 * 1024) continue;
            std::string ext = p.has_extension() ? to_lower_ascii(path_to_utf8(p.extension())) : "";
            if (is_likely_binary_ext(ext)) continue;

            std::string rel = path_to_utf8(fs::relative(p, root_copy, ec));
            std::string content = read_text_file(p);

            if (ext == ".json" && fname != "tsconfig.json") {
                try {
                    auto parsed = json::parse(content);
                    (void)parsed;
                } catch (const std::exception& e) {
                    items.push_back({
                        {"file", rel},
                        {"line", 1},
                        {"severity", "error"},
                        {"source", "json-validator"},
                        {"message", std::string("Invalid JSON: ") + e.what()}
                    });
                }
            }

            auto lines = split_lines(content);
            for (size_t i = 0; i < lines.size(); ++i) {
                if (lines[i].rfind("<<<<<<< ", 0) == 0 || lines[i].rfind(">>>>>>> ", 0) == 0) {
                    items.push_back({
                        {"file", rel},
                        {"line", i + 1},
                        {"severity", "error"},
                        {"source", "merge-conflict"},
                        {"message", "Unresolved Git merge conflict marker"}
                    });
                }
            }
            if (items.size() >= 60) break;
        }

        // 2. Run TypeScript check if tsconfig.json exists
        if (fs::exists(root_copy / "tsconfig.json", ec)) {
            auto tsc = run_process_silent("npx --no-install tsc --noEmit --pretty false", path_to_native_utf8(root_copy), "cmd", 15000);
            std::regex re(R"(^(.+?)\((\d+),(\d+)\):\s*(error|warning)\s+TS\d+:\s*(.+)$)");
            for (const auto& ln : split_lines(tsc.stdout_text + "\n" + tsc.stderr_text)) {
                std::smatch m;
                if (std::regex_match(ln, m, re)) {
                    items.push_back({
                        {"file", m[1].str()},
                        {"line", std::stoi(m[2].str())},
                        {"severity", m[4].str()},
                        {"source", "typescript"},
                        {"message", m[5].str()}
                    });
                }
            }
        }

        return {
            {"ok", true},
            {"count", items.size()},
            {"diagnostics", items}
        };
    }

private:
    mutable std::mutex mu_;
    fs::path root_;
    std::vector<Checkpoint> checkpoints_;
};

} // namespace aircode
