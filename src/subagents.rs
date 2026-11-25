//! Sub-Agent Builder & Registry — allows the AI (and user) to create custom specialized
//! sub-agents (`.aircode/subagents/<slug>.json`) and delegate subtasks to them.

use crate::json::JsonValue;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SubAgentSpec {
    pub slug: String,
    pub title: String,
    pub effort: String, // "low" | "medium" | "max" | "ultra"
    pub readonly: bool,
    pub system_prompt: String,
    pub builtin: bool,
}

impl SubAgentSpec {
    pub fn to_json(&self) -> JsonValue {
        JsonValue::obj(vec![
            ("slug", self.slug.clone().into()),
            ("title", self.title.clone().into()),
            ("effort", self.effort.clone().into()),
            ("readonly", self.readonly.into()),
            ("systemPrompt", self.system_prompt.clone().into()),
        ])
    }

    pub fn from_json(v: &JsonValue) -> Self {
        Self {
            slug: v.string_or("slug", "custom-subagent"),
            title: v.string_or("title", "Custom Sub-Agent"),
            effort: v.string_or("effort", "medium"),
            readonly: v.bool_or("readonly", false),
            system_prompt: v.string_or("systemPrompt", "Complete the delegated engineering task."),
            builtin: false,
        }
    }
}

pub fn builtin_subagents() -> Vec<SubAgentSpec> {
    vec![
        SubAgentSpec {
            slug: "explorer".into(),
            title: "Codebase Explorer".into(),
            effort: "low".into(),
            readonly: true,
            system_prompt: "Fast read-only exploration sub-agent. Search files, trace call graphs, and return concise file:line findings.".into(),
            builtin: true,
        },
        SubAgentSpec {
            slug: "architect".into(),
            title: "Systems Architect".into(),
            effort: "max".into(),
            readonly: true,
            system_prompt: "Deep architectural review sub-agent. Analyze invariants, edge cases, interfaces, and design clean multi-file plans.".into(),
            builtin: true,
        },
        SubAgentSpec {
            slug: "test-healer".into(),
            title: "Compiler & Test Healer".into(),
            effort: "max".into(),
            readonly: false,
            system_prompt: "Run diagnose_errors and run_tests, isolate compiler or test failures, apply surgical fixes, and verify green status.".into(),
            builtin: true,
        },
        SubAgentSpec {
            slug: "browser-verifier".into(),
            title: "Agentic Browser Tester".into(),
            effort: "medium".into(),
            readonly: true,
            system_prompt: "Navigate target URLs in AirCode's inbuilt Agentic Browser, snapshot interactive elements, test flows, and report UI/HTTP issues.".into(),
            builtin: true,
        },
    ]
}

pub fn load_all_subagents(workspace: &Path) -> Vec<SubAgentSpec> {
    let mut list = builtin_subagents();
    let dir = workspace.join(".aircode").join("subagents");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(v) = JsonValue::parse(&raw) else {
                continue;
            };
            let spec = SubAgentSpec::from_json(&v);
            if let Some(existing) = list.iter_mut().find(|s| s.slug == spec.slug) {
                *existing = spec;
            } else {
                list.push(spec);
            }
        }
    }
    list
}

pub fn save_subagent(workspace: &Path, spec: &SubAgentSpec) -> Result<PathBuf, String> {
    let clean_slug: String = spec
        .slug
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
        return Err("Sub-agent slug cannot be empty".into());
    }
    let dir = workspace.join(".aircode").join("subagents");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut normalized = spec.clone();
    normalized.slug = clean_slug.clone();
    let file = dir.join(format!("{}.json", clean_slug));
    std::fs::write(&file, normalized.to_json().to_json_string().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(file)
}
