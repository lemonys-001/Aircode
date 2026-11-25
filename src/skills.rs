//! Skills Engine for AirCode + Built-in `aircode-customizer` Skill ("How to Customize AirCode").

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SkillEntry {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub builtin: bool,
    pub enabled: bool,
    pub source_path: Option<PathBuf>,
}

pub const AIRCODE_CUSTOMIZER_MD: &str = r##"# AirCode Customizer Skill (`aircode-customizer`)

You are running inside **AirCode**, a 100% Pure-Rust Native Desktop IDE & Agentic Studio.
Whenever the user asks to customize AirCode's appearance, theme colors, font size, skills, sub-agents, MCP servers, or behavior, follow these exact file schemas inside the workspace's `.aircode/` folder. AirCode automatically hot-reloads these files after your tool call finishes!

## 1. Customizing AirCode Themes (`.aircode/theme.json`)
Write `.aircode/theme.json` using `write_file` to create or tweak a live custom theme:
```json
{
  "name": "Cyber Emerald",
  "base": "tokyo-night",
  "bgApp": "#0B1015",
  "bgRail": "#080C10",
  "bgHeader": "#080C10",
  "bgSidebar": "#0E151C",
  "bgEditor": "#101922",
  "bgTerminal": "#090E13",
  "bgAgent": "#0E151C",
  "bgCard": "#16222E",
  "bgInput": "#090E13",
  "border": "#1F3142",
  "borderFocus": "#10B981",
  "textPrimary": "#ECFDF5",
  "textSecondary": "#94A3B8",
  "textMuted": "#64748B",
  "accent": "#10B981",
  "accentSecondary": "#38BDF8",
  "success": "#34D399",
  "warning": "#FBBF24",
  "danger": "#F43F5E"
}
```
Supported `"base"` presets: `"tokyo-night"`, `"catppuccin-mocha"`, `"one-dark"`, `"rose-pine"`, `"gruvbox-dark"`, `"alabaster-light"`.

## 2. Customizing AirCode Studio Config (`.aircode/config.json`)
Write `.aircode/config.json` to adjust studio defaults for this project:
```json
{
  "theme": "custom",
  "editorFontSize": 15,
  "mode": "build",
  "effort": "max",
  "showThinking": true,
  "customInstructions": "Prefer idiomatic, zero-warning code and run tests after edits."
}
```

## 3. Creating Reusable Skills (`.aircode/skills/<slug>/SKILL.md`)
Use `create_skill` (or `write_file` to `.aircode/skills/<slug>/SKILL.md`) with frontmatter:
```markdown
---
name: my-skill-slug
title: My Domain Skill
description: Concise one-line description of when to apply this skill
---
Detailed domain instructions, conventions, and checklists...
```

## 4. Creating Custom Sub-Agents (`.aircode/subagents/<slug>.json`)
Use `create_subagent` (or write `.aircode/subagents/<slug>.json`) so you or the user can delegate specialized subtasks via `run_subagent`:
```json
{
  "slug": "security-reviewer",
  "title": "Security & Sandbox Auditor",
  "effort": "max",
  "readonly": true,
  "systemPrompt": "Inspect code changes for injection, auth, memory, and path-traversal flaws."
}
```

## 5. Adding MCP Tool Servers (`.aircode/mcp.json`)
Register stdio or HTTP Model Context Protocol (`2024-11-05`) servers in `.aircode/mcp.json`:
```json
{
  "servers": [
    {
      "name": "filesystem",
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
      "enabled": true
    }
  ]
}
```
"##;

pub fn builtin_skills() -> Vec<SkillEntry> {
    vec![
        SkillEntry {
            slug: "aircode-customizer".into(),
            title: "AirCode Self-Customizer".into(),
            description: "Teaches the AI how to customize AirCode themes, config, skills, sub-agents & MCPs live.".into(),
            instructions: AIRCODE_CUSTOMIZER_MD.into(),
            builtin: true,
            enabled: true,
            source_path: None,
        },
        SkillEntry {
            slug: "agentic-browser-qa".into(),
            title: "Agentic Browser Builder QA".into(),
            description: "Best practices for inspecting, clicking, typing & verifying web apps in AirCode's inbuilt browser.".into(),
            instructions: "When building or debugging web servers/UIs:\n\
                1. Start or check the dev server URL.\n\
                2. Call `browser_navigate` to open the URL in AirCode's inbuilt Agentic Browser.\n\
                3. Call `browser_snapshot` to inspect interactive `[e1]`, `[e2]` element refs.\n\
                4. Use `browser_click` and `browser_type` to verify flows; if authentication or CAPTCHA appears, tell the user they can click 'Take Over' in the Browser bar and resume when ready.".into(),
            builtin: true,
            enabled: true,
            source_path: None,
        },
        SkillEntry {
            slug: "surgical-engineering".into(),
            title: "Surgical Code & Test Discipline".into(),
            description: "Enforces minimal diffs, root-cause diagnosis, and compile/test verification.".into(),
            instructions: "Always inspect target files before editing, prefer surgical `edit_file` changes over blind rewrites, and run `diagnose_errors` or `run_tests` before concluding Build-mode tasks.".into(),
            builtin: true,
            enabled: true,
            source_path: None,
        },
    ]
}

pub fn load_all_skills(
    workspace: &Path,
    app_data_dir: &Path,
    disabled_slugs: &HashSet<String>,
) -> Vec<SkillEntry> {
    let mut list = builtin_skills();
    let search_roots = [
        workspace.join(".aircode").join("skills"),
        workspace.join(".claude").join("skills"),
        app_data_dir.join("skills"),
    ];

    for root in &search_roots {
        let Ok(rd) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            let md_path = if p.is_dir() {
                p.join("SKILL.md")
            } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
                p.clone()
            } else {
                continue;
            };
            let Ok(raw) = std::fs::read_to_string(&md_path) else {
                continue;
            };
            let fallback_slug = entry
                .file_name()
                .to_string_lossy()
                .trim_end_matches(".md")
                .to_string();
            let parsed = parse_skill_markdown(&fallback_slug, &raw, Some(md_path));
            if let Some(existing) = list.iter_mut().find(|s| s.slug == parsed.slug) {
                *existing = parsed;
            } else {
                list.push(parsed);
            }
        }
    }

    for skill in &mut list {
        if disabled_slugs.contains(&skill.slug) {
            skill.enabled = false;
        }
    }
    list
}

fn parse_skill_markdown(
    fallback_slug: &str,
    raw: &str,
    source_path: Option<PathBuf>,
) -> SkillEntry {
    let mut slug = fallback_slug.to_string();
    let mut title = fallback_slug.replace('-', " ");
    let mut description = "Custom workspace skill".to_string();
    let mut body = raw.to_string();

    if let Some(rest) = raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n")) {
        if let Some((fm, content)) = rest.split_once("\n---") {
            body = content.trim_start_matches(['\r', '\n']).to_string();
            for line in fm.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim();
                    let val = v.trim().trim_matches('"').trim_matches('\'');
                    match key {
                        "name" | "slug" if !val.is_empty() => slug = val.to_string(),
                        "title" if !val.is_empty() => title = val.to_string(),
                        "description" if !val.is_empty() => description = val.to_string(),
                        _ => {}
                    }
                }
            }
        }
    }

    SkillEntry {
        slug,
        title,
        description,
        instructions: body,
        builtin: false,
        enabled: true,
        source_path,
    }
}

pub fn save_workspace_skill(
    workspace: &Path,
    slug: &str,
    title: &str,
    description: &str,
    instructions: &str,
) -> Result<PathBuf, String> {
    let clean_slug: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if clean_slug.trim_matches('-').is_empty() {
        return Err("Skill slug cannot be empty".into());
    }
    let dir = workspace.join(".aircode").join("skills").join(&clean_slug);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join("SKILL.md");
    let doc = format!(
        "---\nname: {}\ntitle: {}\ndescription: {}\n---\n\n{}\n",
        clean_slug,
        title.trim(),
        description.trim(),
        instructions.trim()
    );
    std::fs::write(&file, doc.as_bytes()).map_err(|e| e.to_string())?;
    Ok(file)
}

pub fn format_enabled_skills_for_prompt(skills: &[SkillEntry]) -> String {
    let active: Vec<&SkillEntry> = skills.iter().filter(|s| s.enabled).collect();
    if active.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\n=== ACTIVE SKILLS ===\n");
    for s in active {
        out.push_str(&format!(
            "\n### Skill `{}` — {}\n{}\n",
            s.slug, s.title, s.instructions
        ));
    }
    out
}
