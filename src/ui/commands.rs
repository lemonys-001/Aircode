//! Fuzzy Command Palette (`Ctrl+K` / `Ctrl+P`) & Slash Command Registry for AirCode.

#[derive(Clone, Debug)]
pub struct PaletteItem {
    pub id: String,
    pub slash: String,
    pub title: String,
    pub subtitle: String,
    pub shortcut: String,
    pub is_file: bool,
}

pub fn builtin_palette_commands(show_thinking: bool, mode: &str, effort: &str, theme: &str) -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "cmd:zen_free".into(),
            slash: "/zen".into(),
            title: "OpenCode Zen Free Lane (no API key)".into(),
            subtitle: "Authenticate like the official OpenCode client and use the anonymous public free models".into(),
            shortcut: "/zen".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:zen_models".into(),
            slash: "/models".into(),
            title: "Refresh OpenCode Zen Free Model Catalog".into(),
            subtitle: "Fetch the live anonymous catalog from opencode.ai/zen/v1/models".into(),
            shortcut: "/models".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:connect".into(),
            slash: "/connect".into(),
            title: "Connect AI Provider & OpenCode Auth".into(),
            subtitle: "Guided key setup, OpenCode Zen free workaround, auth.json import & local bridge".into(),
            shortcut: "Ctrl+Shift+P".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:shell_workspace".into(),
            slash: "/workspace".into(),
            title: "Switch to the Workspace IDE".into(),
            subtitle: "Files, editor, terminal, git and the agentic browser".into(),
            shortcut: "ctrl+1".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:shell_sidebar".into(),
            slash: "/sidebar".into(),
            title: "Toggle the Context Sidebar".into(),
            subtitle: "Tokens, cost, MCP, LSP, todos and modified files".into(),
            shortcut: "ctrl+b".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:shell_tools".into(),
            slash: "/tools".into(),
            title: "Toggle Tool Blocks in the Transcript".into(),
            subtitle: "Show or hide the + Thought / + Tool disclosures".into(),
            shortcut: "ctrl+h".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:cycle_model".into(),
            slash: "(model box)".into(),
            title: "Cycle the Active Model".into(),
            subtitle: "Step through the models available for this provider".into(),
            shortcut: "".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:toggle_thinking".into(),
            slash: "/thinking".into(),
            title: if show_thinking {
                "Hide AI Thinking Blocks".into()
            } else {
                "Show AI Thinking Blocks".into()
            },
            subtitle: format!(
                "Currently {} — toggle visibility of <think> & extended reasoning blocks",
                if show_thinking { "SHOWN" } else { "HIDDEN" }
            ),
            shortcut: "/thinking".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:compact".into(),
            slash: "/compact".into(),
            title: "Compact Session Context".into(),
            subtitle: "Summarize older turns into a clean checkpoint to save context tokens".into(),
            shortcut: "/compact".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:fork".into(),
            slash: "/fork".into(),
            title: "Fork Current Session".into(),
            subtitle: "Branch an independent copy of this session (or fork any prompt in Timeline)".into(),
            shortcut: "/fork".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:new_session".into(),
            slash: "/new".into(),
            title: "New Coding Session".into(),
            subtitle: "Start a fresh AI session in the current workspace".into(),
            shortcut: "Ctrl+N".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:timeline".into(),
            slash: "/timeline".into(),
            title: "Sessions & Prompt Timeline".into(),
            subtitle: "Inspect prompt timestamps, jump to turns, or fork from any historical prompt".into(),
            shortcut: "/timeline".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:mode_plan".into(),
            slash: "/plan".into(),
            title: "Switch to Plan Mode (Read-Only)".into(),
            subtitle: format!("Active mode: {} — Plan researches & designs without mutating files", mode.to_uppercase()),
            shortcut: "/plan".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:mode_build".into(),
            slash: "/build".into(),
            title: "Switch to Build Mode (Autonomous)".into(),
            subtitle: "Full multi-file editing, terminal, tests, sub-agents & browser execution".into(),
            shortcut: "/build".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:effort_low".into(),
            slash: "/effort low".into(),
            title: "Agent Effort: Low (Fast & Direct)".into(),
            subtitle: format!("Active effort: {} — quick 6-step budget", effort.to_uppercase()),
            shortcut: "".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:effort_medium".into(),
            slash: "/effort medium".into(),
            title: "Agent Effort: Medium (Balanced)".into(),
            subtitle: "Balanced 14-step reasoning & verification loop".into(),
            shortcut: "".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:effort_max".into(),
            slash: "/effort max".into(),
            title: "Agent Effort: Max (Deep Reasoning)".into(),
            subtitle: "Extended thinking budget + 26-step thorough verification".into(),
            shortcut: "".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:effort_ultra".into(),
            slash: "/effort ultra".into(),
            title: "Agent Effort: Ultra (Maximum Depth)".into(),
            subtitle: "32k thinking budget + 44-step exhaustive engineering & sub-agents".into(),
            shortcut: "".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:theme_picker".into(),
            slash: "/theme".into(),
            title: "Switch Color Theme".into(),
            subtitle: format!(
                "Active: {} — Tokyo Night, Catppuccin, One Dark, Rosé Pine, Gruvbox, Alabaster, Custom",
                theme
            ),
            shortcut: "/theme".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:browser".into(),
            slash: "/browser".into(),
            title: "Open Inbuilt Agentic Browser".into(),
            subtitle: "Co-browse localhost & docs with Playwright [eN] handles + instant user takeover".into(),
            shortcut: "Ctrl+B".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:browser_takeover".into(),
            slash: "/takeover".into(),
            title: "Browser: Interrupt AI & Take Control".into(),
            subtitle: "Pause AI browser actions so you can manually log in, click, or inspect".into(),
            shortcut: "".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:skills".into(),
            slash: "/skills".into(),
            title: "Skills, Sub-Agents & MCP Hub".into(),
            subtitle: "Toggle skills (including AirCode Self-Customizer), sub-agents & MCP servers".into(),
            shortcut: "/skills".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:customize_aircode".into(),
            slash: "/customize".into(),
            title: "Customize AirCode with AI (Skill Guide)".into(),
            subtitle: "Open the built-in `aircode-customizer` skill or ask AI to style AirCode".into(),
            shortcut: "".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:toggle_terminal".into(),
            slash: "/terminal".into(),
            title: "Toggle Integrated Terminal Drawer".into(),
            subtitle: "Show or collapse the native PTY terminal pane for extra editor space".into(),
            shortcut: "Ctrl+J".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:goto_line".into(),
            slash: "/goto".into(),
            title: "Editor: Go to Line...".into(),
            subtitle: "Jump cursor directly to a line number in the active code tab".into(),
            shortcut: "Ctrl+G".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:find".into(),
            slash: "/find".into(),
            title: "Editor: Find in File".into(),
            subtitle: "Search occurrences in the active editor buffer".into(),
            shortcut: "Ctrl+F".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:open_folder".into(),
            slash: "/open".into(),
            title: "Open Workspace Folder...".into(),
            subtitle: "Pick a project directory using the native OS dialog".into(),
            shortcut: "Ctrl+O".into(),
            is_file: false,
        },
        PaletteItem {
            id: "cmd:safety".into(),
            slash: "/settings".into(),
            title: "Safety Gates & Editor Font Size".into(),
            subtitle: "Configure shell approval, delete confirmation, sandbox & font size".into(),
            shortcut: "".into(),
            is_file: false,
        },
    ]
}

pub fn filter_palette_items(
    query: &str,
    commands: &[PaletteItem],
    workspace_files: &[(String, String)], // (rel_path, name)
) -> Vec<PaletteItem> {
    let q = query.trim().to_ascii_lowercase();
    let wants_commands_only = q.starts_with('>') || q.starts_with('/');
    let clean_q = q.trim_start_matches(['>', '/']).trim();

    let mut scored: Vec<(i32, PaletteItem)> = Vec::new();

    for cmd in commands {
        if clean_q.is_empty() {
            scored.push((100, cmd.clone()));
            continue;
        }
        let hay_title = cmd.title.to_ascii_lowercase();
        let hay_slash = cmd.slash.to_ascii_lowercase();
        let hay_sub = cmd.subtitle.to_ascii_lowercase();
        if let Some(sc) = fuzzy_score(clean_q, &hay_slash)
            .map(|s| s + 50)
            .or_else(|| fuzzy_score(clean_q, &hay_title).map(|s| s + 25))
            .or_else(|| fuzzy_score(clean_q, &hay_sub))
        {
            scored.push((sc, cmd.clone()));
        }
    }

    if !wants_commands_only && !clean_q.is_empty() {
        for (rel, name) in workspace_files {
            let nl = name.to_ascii_lowercase();
            let rl = rel.to_ascii_lowercase();
            if let Some(sc) = fuzzy_score(clean_q, &nl)
                .map(|s| s + 30)
                .or_else(|| fuzzy_score(clean_q, &rl))
            {
                scored.push((
                    sc,
                    PaletteItem {
                        id: format!("file:{}", rel),
                        slash: "FILE".into(),
                        title: name.clone(),
                        subtitle: rel.clone(),
                        shortcut: "Enter".into(),
                        is_file: true,
                    },
                ));
            }
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(12).map(|(_, item)| item).collect()
}

fn fuzzy_score(needle: &str, hay: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(1);
    }
    if hay == needle {
        return Some(200);
    }
    if hay.starts_with(needle) {
        return Some(150);
    }
    if let Some(pos) = hay.find(needle) {
        return Some(110 - (pos as i32).min(40));
    }
    // Subsequence match
    let mut n_chars = needle.chars().peekable();
    let mut streak = 0i32;
    let mut score = 20i32;
    for hc in hay.chars() {
        if Some(&hc) == n_chars.peek() {
            n_chars.next();
            streak += 1;
            score += 4 + streak * 2;
            if n_chars.peek().is_none() {
                return Some(score);
            }
        } else {
            streak = 0;
        }
    }
    None
}
