//! Inbuilt Agentic Browser for Builders (`src/browser.rs`) — Windows 10/11 & Linux DE.
//!
//! Provides a Playwright-style interactive DOM & visual page engine directly inside AirCode's
//! 100% Pure-Rust Native GUI:
//! - AI Agent tools (`browser_navigate`, `browser_snapshot`, `browser_click`, `browser_type`,
//!   `browser_scroll`) inspect `[e1]`, `[e2]` element handles and drive web apps/docs.
//! - Human builders can click **Interrupt & Take Control** (or click directly on the page while AI
//!   is driving) to pause AI automation, type into inputs, follow links, test localhost servers,
//!   and click **Hand Back to AI** to resume seamlessly.
//! - Supports both `NativeFast` (WinHTTP / POSIX HTTP + HTML5 layout) and optional `HeadlessJs`
//!   DOM hydration (Edge / Chrome / Chromium `--headless --dump-dom` for React/Vite SPAs).

use crate::platform::http_request;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserOwner {
    Idle,
    AiDriving(String),
    UserTakeover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserJsEngine {
    NativeFast,
    HeadlessDomJs,
}

#[derive(Clone, Debug)]
pub enum PageBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    ListItem {
        text: String,
    },
    CodeBlock {
        text: String,
    },
    Banner {
        text: String,
    },
    Link {
        ref_id: usize,
        label: String,
        href: String,
    },
    Button {
        ref_id: usize,
        label: String,
        action_url: Option<String>,
    },
    Input {
        ref_id: usize,
        name: String,
        placeholder: String,
        value: String,
        secret: bool,
        action_url: Option<String>,
    },
    Divider,
}

#[derive(Clone, Debug)]
pub struct InteractiveElement {
    pub ref_id: usize,
    pub kind: String, // "link" | "button" | "input"
    pub label: String,
    pub target: String,
}

pub struct AgenticBrowser {
    pub current_url: String,
    pub title: String,
    pub http_status: u16,
    pub owner: BrowserOwner,
    pub engine: BrowserJsEngine,
    pub show_dom_inspector: bool,
    pub history: Vec<String>,
    pub history_pos: usize,
    pub blocks: Vec<PageBlock>,
    pub elements: Vec<InteractiveElement>,
    pub focused_input_ref: Option<usize>,
    pub scroll_y: i32,
    pub last_action_log: String,
}

#[derive(Clone)]
pub struct SharedBrowser {
    pub inner: Arc<Mutex<AgenticBrowser>>,
    pub user_interrupted: Arc<AtomicBool>,
}

impl SharedBrowser {
    pub fn new() -> Self {
        let mut b = AgenticBrowser {
            current_url: "aircode://welcome".into(),
            title: "AirCode Agentic Browser".into(),
            http_status: 200,
            owner: BrowserOwner::Idle,
            engine: BrowserJsEngine::NativeFast,
            show_dom_inspector: false,
            history: vec!["aircode://welcome".into()],
            history_pos: 0,
            blocks: Vec::new(),
            elements: Vec::new(),
            focused_input_ref: None,
            scroll_y: 0,
            last_action_log: "Ready — Builder + AI Co-Browsing".into(),
        };
        b.load_html("aircode://welcome", 200, WELCOME_HTML, false);
        Self {
            inner: Arc::new(Mutex::new(b)),
            user_interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn interrupt_for_user(&self) {
        self.user_interrupted.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.inner.lock() {
            g.owner = BrowserOwner::UserTakeover;
            g.last_action_log =
                "User took control — AI browser actions paused until 'Resume AI'".into();
        }
    }

    pub fn hand_back_to_ai(&self) {
        self.user_interrupted.store(false, Ordering::SeqCst);
        if let Ok(mut g) = self.inner.lock() {
            g.owner = BrowserOwner::Idle;
            g.last_action_log = "Control handed back to AI Agent".into();
        }
    }

    pub fn is_user_takeover(&self) -> bool {
        self.user_interrupted.load(Ordering::SeqCst)
    }

    pub fn navigate(&self, raw_url: &str, by_ai: bool) -> Result<String, String> {
        if by_ai && self.is_user_takeover() {
            return Err(
                "USER_TAKEOVER_ACTIVE: The user interrupted the browser to interact with it manually. Pause browser actions and ask or wait until they hand control back."
                    .into(),
            );
        }

        let url = normalize_url(raw_url);
        let engine = {
            let mut g = self.inner.lock().unwrap();
            if by_ai {
                g.owner = BrowserOwner::AiDriving(format!("Navigating to {}", url));
            }
            g.last_action_log = format!(
                "{} navigating -> {}",
                if by_ai { "AI" } else { "User" },
                url
            );
            g.engine
        };

        let (status, html) = fetch_url_html(&url, engine)?;

        let mut g = self.inner.lock().unwrap();
        g.load_html(&url, status, &html, true);
        if by_ai && !self.is_user_takeover() {
            g.owner = BrowserOwner::AiDriving(format!("Viewing {}", g.title));
        }
        Ok(g.playwright_snapshot(45))
    }

    pub fn click_ref(&self, ref_id: usize, by_ai: bool) -> Result<String, String> {
        if by_ai && self.is_user_takeover() {
            return Err(
                "USER_TAKEOVER_ACTIVE: User took manual control of the browser. Wait for user before continuing browser automation."
                    .into(),
            );
        }

        enum ClickTarget {
            Navigate(String),
            FocusInput(usize),
            SubmitForm(String),
            NoOp(String),
        }

        let target = {
            let mut g = self.inner.lock().unwrap();
            if by_ai {
                g.owner = BrowserOwner::AiDriving(format!("Clicking [e{}]", ref_id));
            }
            let mut found = None;
            for blk in &g.blocks {
                match blk {
                    PageBlock::Link {
                        ref_id: id,
                        href,
                        label,
                    } if *id == ref_id => {
                        let resolved = resolve_relative_url(&g.current_url, href);
                        g.last_action_log = format!("Clicked [e{}] '{}' -> {}", id, label, resolved);
                        found = Some(ClickTarget::Navigate(resolved));
                        break;
                    }
                    PageBlock::Button {
                        ref_id: id,
                        label,
                        action_url,
                    } if *id == ref_id => {
                        let act_owned = action_url.clone();
                        let label_owned = label.clone();
                        if let Some(act) = act_owned {
                            let resolved = g.build_form_submit_url(&act);
                            g.last_action_log = format!("Submitted [e{}] '{}'", id, label);
                            found = Some(ClickTarget::SubmitForm(resolved));
                        } else {
                            g.last_action_log = format!("Activated button [e{}] '{}'", id, label_owned);
                            found = Some(ClickTarget::NoOp(label_owned));
                        }
                        break;
                    }
                    PageBlock::Input { ref_id: id, .. } if *id == ref_id => {
                        found = Some(ClickTarget::FocusInput(*id));
                        break;
                    }
                    _ => {}
                }
            }
            found.ok_or_else(|| format!("Element [e{}] not found on current page", ref_id))?
        };

        match target {
            ClickTarget::Navigate(u) | ClickTarget::SubmitForm(u) => self.navigate(&u, by_ai),
            ClickTarget::FocusInput(id) => {
                let mut g = self.inner.lock().unwrap();
                g.focused_input_ref = Some(id);
                Ok(g.playwright_snapshot(35))
            }
            ClickTarget::NoOp(lbl) => {
                let g = self.inner.lock().unwrap();
                Ok(format!(
                    "Clicked button [e{}] '{}'.\n{}",
                    ref_id,
                    lbl,
                    g.playwright_snapshot(30)
                ))
            }
        }
    }

    pub fn type_into_ref(
        &self,
        ref_id: usize,
        text: &str,
        submit: bool,
        by_ai: bool,
    ) -> Result<String, String> {
        if by_ai && self.is_user_takeover() {
            return Err("USER_TAKEOVER_ACTIVE: User currently controls the browser.".into());
        }

        let mut submit_target = None;
        {
            let mut g = self.inner.lock().unwrap();
            if by_ai {
                g.owner = BrowserOwner::AiDriving(format!("Typing into [e{}]", ref_id));
            }
            let mut matched = false;
            let mut form_act = None;
            for blk in &mut g.blocks {
                if let PageBlock::Input {
                    ref_id: id,
                    value,
                    action_url,
                    ..
                } = blk
                {
                    if *id == ref_id {
                        *value = text.to_string();
                        form_act = action_url.clone();
                        matched = true;
                        break;
                    }
                }
            }
            if !matched {
                return Err(format!("Input element [e{}] not found", ref_id));
            }
            g.focused_input_ref = Some(ref_id);
            g.last_action_log = format!("Typed '{}' into [e{}]", text, ref_id);
            if submit {
                if let Some(act) = form_act {
                    submit_target = Some(g.build_form_submit_url(&act));
                } else if g.current_url == "aircode://welcome" {
                    // Search box on welcome page navigates directly or searches DuckDuckGo HTML
                    let q = url_encode(text);
                    submit_target = Some(if text.starts_with("http://")
                        || text.starts_with("https://")
                        || text.starts_with("localhost")
                        || text.starts_with("127.0.0.1")
                    {
                        normalize_url(text)
                    } else {
                        format!("https://html.duckduckgo.com/html/?q={}", q)
                    });
                }
            }
        }

        if let Some(target_url) = submit_target {
            return self.navigate(&target_url, by_ai);
        }
        let g = self.inner.lock().unwrap();
        Ok(g.playwright_snapshot(35))
    }

    pub fn snapshot(&self, max_blocks: usize) -> String {
        let g = self.inner.lock().unwrap();
        g.playwright_snapshot(max_blocks)
    }

    pub fn go_back(&self) {
        let target = {
            let mut g = self.inner.lock().unwrap();
            if g.history_pos > 0 {
                g.history_pos -= 1;
                Some(g.history[g.history_pos].clone())
            } else {
                None
            }
        };
        if let Some(u) = target {
            if let Ok((st, html)) = fetch_url_html(&u, BrowserJsEngine::NativeFast) {
                let mut g = self.inner.lock().unwrap();
                g.load_html(&u, st, &html, false);
            }
        }
    }

    pub fn go_forward(&self) {
        let target = {
            let mut g = self.inner.lock().unwrap();
            if g.history_pos + 1 < g.history.len() {
                g.history_pos += 1;
                Some(g.history[g.history_pos].clone())
            } else {
                None
            }
        };
        if let Some(u) = target {
            if let Ok((st, html)) = fetch_url_html(&u, BrowserJsEngine::NativeFast) {
                let mut g = self.inner.lock().unwrap();
                g.load_html(&u, st, &html, false);
            }
        }
    }
}

impl AgenticBrowser {
    pub fn load_html(&mut self, url: &str, status: u16, html: &str, push_history: bool) {
        self.current_url = url.to_string();
        self.http_status = status;
        self.scroll_y = 0;
        self.focused_input_ref = None;
        if push_history {
            if self.history.get(self.history_pos).map(|s| s.as_str()) != Some(url) {
                let trunc = self.history_pos + 1;
                self.history.truncate(trunc);
                self.history.push(url.to_string());
                self.history_pos = self.history.len() - 1;
            }
        }
        let (title, blocks, elements) = parse_html_document(html, url);
        self.title = title;
        self.blocks = blocks;
        self.elements = elements;
    }

    pub fn build_form_submit_url(&self, action: &str) -> String {
        let base = resolve_relative_url(&self.current_url, action);
        let mut params = Vec::new();
        for blk in &self.blocks {
            if let PageBlock::Input { name, value, .. } = blk {
                if !name.is_empty() {
                    params.push(format!("{}={}", url_encode(name), url_encode(value)));
                }
            }
        }
        if params.is_empty() {
            base
        } else if base.contains('?') {
            format!("{}&{}", base, params.join("&"))
        } else {
            format!("{}?{}", base, params.join("&"))
        }
    }

    pub fn playwright_snapshot(&self, max_blocks: usize) -> String {
        let mut out = format!(
            "Page URL: {}\nPage Title: {}\nHTTP Status: {}\nControl State: {:?}\n\n=== Interactive Elements (Playwright Refs) ===\n",
            self.current_url, self.title, self.http_status, self.owner
        );
        if self.elements.is_empty() {
            out.push_str("(no interactive elements)\n");
        } else {
            for el in self.elements.iter().take(60) {
                let mut extra = String::new();
                for b in &self.blocks {
                    if let PageBlock::Input {
                        ref_id, value, ..
                    } = b
                    {
                        if *ref_id == el.ref_id && !value.is_empty() {
                            extra = format!(" value=\"{}\"", value);
                        }
                    }
                }
                out.push_str(&format!(
                    "- [e{}] {} \"{}\"{} -> {}\n",
                    el.ref_id, el.kind, el.label, extra, el.target
                ));
            }
        }

        out.push_str("\n=== Visible Page Content ===\n");
        for blk in self.blocks.iter().take(max_blocks) {
            match blk {
                PageBlock::Heading { level, text } => {
                    out.push_str(&format!("{} {}\n", "#".repeat(*level as usize), text));
                }
                PageBlock::Paragraph { text } | PageBlock::Banner { text } => {
                    out.push_str(&format!("{}\n", text));
                }
                PageBlock::ListItem { text } => {
                    out.push_str(&format!("* {}\n", text));
                }
                PageBlock::CodeBlock { text } => {
                    out.push_str(&format!("```\n{}\n```\n", text));
                }
                PageBlock::Link {
                    ref_id,
                    label,
                    href,
                } => {
                    out.push_str(&format!("[e{} link: {}]({})\n", ref_id, label, href));
                }
                PageBlock::Button { ref_id, label, .. } => {
                    out.push_str(&format!("[e{} button: {}]\n", ref_id, label));
                }
                PageBlock::Input {
                    ref_id,
                    placeholder,
                    value,
                    ..
                } => {
                    out.push_str(&format!(
                        "[e{} input ({}): \"{}\"]\n",
                        ref_id, placeholder, value
                    ));
                }
                PageBlock::Divider => out.push_str("---\n"),
            }
        }
        out
    }
}

pub fn open_in_external_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/C", "start", "", url]);
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}

fn normalize_url(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return "aircode://welcome".into();
    }
    if t.starts_with("aircode://")
        || t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("file://")
    {
        return t.to_string();
    }
    if t.starts_with("localhost") || t.starts_with("127.0.0.1") || t.starts_with("0.0.0.0") {
        return format!("http://{}", t);
    }
    if t.contains('.') && !t.contains(' ') {
        return format!("https://{}", t);
    }
    format!("https://html.duckduckgo.com/html/?q={}", url_encode(t))
}

fn resolve_relative_url(base: &str, href: &str) -> String {
    let h = href.trim();
    if h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with("aircode://")
        || h.starts_with("file://")
    {
        return h.to_string();
    }
    if h.starts_with("//") {
        return format!("https:{}", h);
    }
    if let Some(scheme_pos) = base.find("://") {
        let after_scheme = &base[scheme_pos + 3..];
        let origin_end = after_scheme
            .find('/')
            .map(|i| scheme_pos + 3 + i)
            .unwrap_or(base.len());
        let origin = &base[..origin_end];
        if h.starts_with('/') {
            return format!("{}{}", origin, h);
        }
        let parent = base.rsplit_once('/').map(|(p, _)| p).unwrap_or(origin);
        return format!("{}/{}", parent, h);
    }
    h.to_string()
}

fn fetch_url_html(url: &str, engine: BrowserJsEngine) -> Result<(u16, String), String> {
    if url == "aircode://welcome" {
        return Ok((200, WELCOME_HTML.to_string()));
    }
    if url == "aircode://opencode-auth" {
        return Ok((200, OPENCODE_AUTH_HELP_HTML.to_string()));
    }
    if let Some(file_p) = url.strip_prefix("file://") {
        let clean = file_p.trim_start_matches('/');
        let candidate = if cfg!(windows) {
            PathBuf::from(clean)
        } else {
            PathBuf::from(format!("/{}", clean))
        };
        let content = std::fs::read_to_string(&candidate)
            .map_err(|e| format!("Failed reading {}: {}", candidate.display(), e))?;
        return Ok((200, content));
    }

    if engine == BrowserJsEngine::HeadlessDomJs {
        if let Some(dom) = try_headless_browser_dump_dom(url) {
            return Ok((200, dom));
        }
    }

    let resp = http_request(
        "GET",
        url,
        &[
            (
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AirCodeAgenticBrowser/2.1",
            ),
            ("Accept", "text/html,application/xhtml+xml,application/json,*/*"),
        ],
        "",
        20_000,
    );
    if let Some(err) = resp.error {
        return Err(err);
    }
    Ok((resp.status, resp.body))
}

fn try_headless_browser_dump_dom(url: &str) -> Option<String> {
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        ]
    } else {
        &["chromium", "chromium-browser", "google-chrome", "brave-browser"]
    };

    for bin in candidates {
        let mut cmd = Command::new(bin);
        cmd.args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--virtual-time-budget=2200",
            "--dump-dom",
            url,
        ]);
        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Ok(out) = cmd.output() {
            if out.status.success() && !out.stdout.is_empty() {
                return Some(String::from_utf8_lossy(&out.stdout).into_owned());
            }
        }
    }
    None
}

fn parse_html_document(
    raw: &str,
    url: &str,
) -> (String, Vec<PageBlock>, Vec<InteractiveElement>) {
    let trimmed = raw.trim();
    // Handle JSON API responses cleanly in the browser too!
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let preview: String = trimmed.chars().take(6000).collect();
        return (
            format!("JSON Response ({})", url),
            vec![
                PageBlock::Heading {
                    level: 2,
                    text: format!("JSON Payload — {}", url),
                },
                PageBlock::CodeBlock { text: preview },
            ],
            Vec::new(),
        );
    }

    let mut title = extract_between_ci(raw, "<title", "</title>")
        .map(|s| strip_tags_and_entities(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| url.to_string());

    // Strip <script>, <style>, <svg>, <noscript>, <head>
    let cleaned = remove_tag_blocks(
        &remove_tag_blocks(
            &remove_tag_blocks(
                &remove_tag_blocks(raw, "script"),
                "style",
            ),
            "svg",
        ),
        "head",
    );

    let mut blocks = Vec::new();
    let mut elements = Vec::new();
    let mut next_ref = 1usize;
    let mut current_form_action: Option<String> = None;

    let mut pos = 0usize;
    let bytes = cleaned.as_bytes();
    let mut text_accum = String::new();

    let flush_paragraph = |acc: &mut String, out: &mut Vec<PageBlock>| {
        let collapsed = collapse_ws(&decode_entities(acc));
        acc.clear();
        if !collapsed.is_empty() {
            out.push(PageBlock::Paragraph { text: collapsed });
        }
    };

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            let Some(rel_end) = cleaned[pos..].find('>') else {
                break;
            };
            let tag_raw = &cleaned[pos + 1..pos + rel_end];
            let tag_trim = tag_raw.trim();
            let is_closing = tag_trim.starts_with('/');
            let tag_name = tag_trim
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            match tag_name.as_str() {
                "h1" | "h2" | "h3" | "h4" if !is_closing => {
                    flush_paragraph(&mut text_accum, &mut blocks);
                    let lvl = tag_name[1..].parse::<u8>().unwrap_or(2);
                    let close_pat = format!("</{}", tag_name);
                    if let Some((inner, after)) =
                        slice_until_ci(&cleaned[pos + rel_end + 1..], &close_pat)
                    {
                        let txt = collapse_ws(&strip_tags_and_entities(inner));
                        if !txt.is_empty() {
                            if title == url && lvl == 1 {
                                title = txt.clone();
                            }
                            blocks.push(PageBlock::Heading {
                                level: lvl,
                                text: txt,
                            });
                        }
                        pos = pos + rel_end + 1 + after;
                        continue;
                    }
                }
                "pre" | "code" if !is_closing && tag_name == "pre" => {
                    flush_paragraph(&mut text_accum, &mut blocks);
                    if let Some((inner, after)) =
                        slice_until_ci(&cleaned[pos + rel_end + 1..], "</pre")
                    {
                        let code_txt = decode_entities(&strip_all_tags(inner))
                            .trim()
                            .to_string();
                        if !code_txt.is_empty() {
                            blocks.push(PageBlock::CodeBlock { text: code_txt });
                        }
                        pos = pos + rel_end + 1 + after;
                        continue;
                    }
                }
                "li" if !is_closing => {
                    flush_paragraph(&mut text_accum, &mut blocks);
                    if let Some((inner, after)) =
                        slice_until_ci(&cleaned[pos + rel_end + 1..], "</li")
                    {
                        let txt = collapse_ws(&strip_tags_and_entities(inner));
                        if !txt.is_empty() {
                            blocks.push(PageBlock::ListItem { text: txt });
                        }
                        pos = pos + rel_end + 1 + after;
                        continue;
                    }
                }
                "a" if !is_closing => {
                    let href = extract_attr(tag_raw, "href").unwrap_or_default();
                    if let Some((inner, after)) =
                        slice_until_ci(&cleaned[pos + rel_end + 1..], "</a")
                    {
                        let lbl = collapse_ws(&strip_tags_and_entities(inner));
                        if !lbl.is_empty() && !href.is_empty() && !href.starts_with("javascript:") {
                            flush_paragraph(&mut text_accum, &mut blocks);
                            let id = next_ref;
                            next_ref += 1;
                            elements.push(InteractiveElement {
                                ref_id: id,
                                kind: "link".into(),
                                label: lbl.clone(),
                                target: href.clone(),
                            });
                            blocks.push(PageBlock::Link {
                                ref_id: id,
                                label: lbl,
                                href,
                            });
                        }
                        pos = pos + rel_end + 1 + after;
                        continue;
                    }
                }
                "button" if !is_closing => {
                    flush_paragraph(&mut text_accum, &mut blocks);
                    if let Some((inner, after)) =
                        slice_until_ci(&cleaned[pos + rel_end + 1..], "</button")
                    {
                        let lbl = collapse_ws(&strip_tags_and_entities(inner));
                        let label = if lbl.is_empty() {
                            "Submit".into()
                        } else {
                            lbl
                        };
                        let id = next_ref;
                        next_ref += 1;
                        elements.push(InteractiveElement {
                            ref_id: id,
                            kind: "button".into(),
                            label: label.clone(),
                            target: current_form_action
                                .clone()
                                .unwrap_or_else(|| "button".into()),
                        });
                        blocks.push(PageBlock::Button {
                            ref_id: id,
                            label,
                            action_url: current_form_action.clone(),
                        });
                        pos = pos + rel_end + 1 + after;
                        continue;
                    }
                }
                "form" => {
                    flush_paragraph(&mut text_accum, &mut blocks);
                    if is_closing {
                        current_form_action = None;
                    } else {
                        current_form_action =
                            Some(extract_attr(tag_raw, "action").unwrap_or_else(|| url.to_string()));
                    }
                }
                "input" | "textarea" if !is_closing => {
                    let typ = extract_attr(tag_raw, "type")
                        .unwrap_or_else(|| "text".into())
                        .to_ascii_lowercase();
                    if typ != "hidden" {
                        flush_paragraph(&mut text_accum, &mut blocks);
                        let id = next_ref;
                        next_ref += 1;
                        if typ == "submit" || typ == "button" {
                            let lbl = extract_attr(tag_raw, "value")
                                .unwrap_or_else(|| "Submit".into());
                            elements.push(InteractiveElement {
                                ref_id: id,
                                kind: "button".into(),
                                label: lbl.clone(),
                                target: current_form_action
                                    .clone()
                                    .unwrap_or_else(|| "submit".into()),
                            });
                            blocks.push(PageBlock::Button {
                                ref_id: id,
                                label: lbl,
                                action_url: current_form_action.clone(),
                            });
                        } else {
                            let name = extract_attr(tag_raw, "name").unwrap_or_default();
                            let placeholder = extract_attr(tag_raw, "placeholder")
                                .or_else(|| if !name.is_empty() { Some(name.clone()) } else { None })
                                .unwrap_or_else(|| "Type text...".into());
                            let val = extract_attr(tag_raw, "value").unwrap_or_default();
                            elements.push(InteractiveElement {
                                ref_id: id,
                                kind: "input".into(),
                                label: placeholder.clone(),
                                target: name.clone(),
                            });
                            blocks.push(PageBlock::Input {
                                ref_id: id,
                                name,
                                placeholder,
                                value: val,
                                secret: typ == "password",
                                action_url: current_form_action.clone(),
                            });
                        }
                    }
                }
                "hr" => {
                    flush_paragraph(&mut text_accum, &mut blocks);
                    blocks.push(PageBlock::Divider);
                }
                "p" | "div" | "section" | "article" | "header" | "footer" | "tr" | "br" => {
                    flush_paragraph(&mut text_accum, &mut blocks);
                }
                _ => {}
            }

            pos += rel_end + 1;
        } else {
            let next_lt = cleaned[pos..].find('<').unwrap_or(cleaned.len() - pos);
            text_accum.push_str(&cleaned[pos..pos + next_lt]);
            text_accum.push(' ');
            pos += next_lt;
        }
    }
    flush_paragraph(&mut text_accum, &mut blocks);

    if blocks.is_empty() {
        blocks.push(PageBlock::Banner {
            text: "Page loaded with no visible text blocks. Try switching Engine -> Headless JS for client-rendered SPAs.".into(),
        });
    }

    (title, blocks, elements)
}

fn remove_tag_blocks(html: &str, tag: &str) -> String {
    let open_pat = format!("<{}", tag);
    let close_pat = format!("</{}>", tag);
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(idx) = find_ci(rest, &open_pat) {
        out.push_str(&rest[..idx]);
        let after_open = &rest[idx..];
        if let Some(end_idx) = find_ci(after_open, &close_pat) {
            rest = &after_open[end_idx + close_pat.len()..];
        } else {
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn find_ci(haystack: &str, needle_lower: &str) -> Option<usize> {
    let n_bytes = needle_lower.as_bytes();
    let h_bytes = haystack.as_bytes();
    if n_bytes.is_empty() || h_bytes.len() < n_bytes.len() {
        return None;
    }
    for i in 0..=(h_bytes.len() - n_bytes.len()) {
        if h_bytes[i..i + n_bytes.len()]
            .iter()
            .zip(n_bytes.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            return haystack.is_char_boundary(i).then_some(i);
        }
    }
    None
}

fn extract_between_ci(html: &str, open_prefix: &str, close_tag: &str) -> Option<String> {
    let start = find_ci(html, &open_prefix.to_ascii_lowercase())?;
    let after_tag = html[start..].find('>')? + start + 1;
    let end_rel = find_ci(&html[after_tag..], &close_tag.to_ascii_lowercase())?;
    Some(html[after_tag..after_tag + end_rel].to_string())
}

fn slice_until_ci<'a>(s: &'a str, close_prefix_lower: &str) -> Option<(&'a str, usize)> {
    let idx = find_ci(s, close_prefix_lower)?;
    let inner = &s[..idx];
    let after_gt = s[idx..].find('>').map(|g| idx + g + 1).unwrap_or(s.len());
    Some((inner, after_gt))
}

fn extract_attr(tag_body: &str, attr_name: &str) -> Option<String> {
    let pat = format!("{}=", attr_name.to_ascii_lowercase());
    let idx = find_ci(tag_body, &pat)?;
    let rest = tag_body[idx + pat.len()..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        Some(decode_entities(&stripped[..end]))
    } else if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        Some(decode_entities(&stripped[..end]))
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(decode_entities(&rest[..end]))
    }
}

fn strip_all_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    out
}

fn strip_tags_and_entities(s: &str) -> String {
    decode_entities(&strip_all_tags(s))
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

const WELCOME_HTML: &str = r#"
<html>
<head><title>AirCode Agentic Browser</title></head>
<body>
  <h1>Agentic Browser for Builders</h1>
  <p>Co-browse localhost dev servers, API endpoints, and docs alongside AirCode. Every link, button, and input has a Playwright-style [eN] handle so the AI can navigate, click, and fill forms — and you can click "Take Over" anytime to interrupt and drive manually.</p>
  <form action="https://html.duckduckgo.com/html/">
    <input type="text" name="q" placeholder="Search docs or enter http://localhost:3000..." value="" />
    <button type="submit">Search / Go</button>
  </form>
  <h2>Quick Builder Targets</h2>
  <a href="http://localhost:3000">Open http://localhost:3000 (Local Dev Server)</a>
  <a href="http://localhost:5173">Open http://localhost:5173 (Vite Dev Server)</a>
  <a href="aircode://opencode-auth">OpenCode Zen & AI Auth Setup Guide</a>
  <a href="https://doc.rust-lang.org/book/">The Rust Programming Language Book</a>
</body>
</html>
"#;

const OPENCODE_AUTH_HELP_HTML: &str = r#"
<html>
<head><title>OpenCode Auth & Provider Guide</title></head>
<body>
  <h1>OpenCode AI Server & Zen Auth Setup</h1>
  <p>AirCode supports three seamless OpenCode connection modes in /connect:</p>
  <ul>
    <li>1. Zero-Key OpenCode Zen Free Tier Workaround — Uses https://opencode.ai/zen/v1 with public client headers for instant free coding models (big-pickle, glm-4.7-free, minimax-m2.1-free).</li>
    <li>2. Auto-Import OpenCode CLI auth.json — Automatically detects tokens saved by `opencode auth login` in ~/.local/share/opencode/auth.json.</li>
    <li>3. Local `opencode serve` Bridge — Proxies through `opencode serve --port 4096` to use OAuth / Claude Max / GitHub Copilot subscriptions!</li>
  </ul>
  <a href="https://opencode.ai/auth">Get an OpenCode Zen Token (opencode.ai/auth)</a>
  <a href="aircode://welcome">Back to Agentic Browser Home</a>
</body>
</html>
"#;
