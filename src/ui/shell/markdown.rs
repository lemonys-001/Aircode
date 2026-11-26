//! Minimal, dependency-free markdown renderer for the conversation column.
//!
//! It turns assistant text into a flat list of `MdSpan`s (one per visual line),
//! which the chat panel draws on the monospace grid. Supported: ATX headings,
//! fenced code blocks, bullets, ordered lists, block quotes, `inline code`,
//! **bold**, *italic* and horizontal rules. That is the subset OpenCode actually
//! renders in its transcript, and it keeps the code small enough to audit.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdKind {
    Text,
    Heading,
    Code,
    Bullet,
    Quote,
    Rule,
}

#[derive(Clone, Debug)]
pub struct MdSpan {
    pub text: String,
    pub kind: MdKind,
    pub bold: bool,
    pub code: bool,
    /// 0 = normal, 1 = inside a code fence (keeps leading whitespace).
    pub fence_depth: usize,
}

#[derive(Clone, Debug)]
pub struct MdLine {
    pub spans: Vec<MdSpan>,
    pub kind: MdKind,
}

/// Split `text` into wrapped markdown lines of at most `cols` characters.
pub fn render_markdown(text: &str, cols: usize) -> Vec<MdLine> {
    let cols = cols.max(8);
    let mut out: Vec<MdLine> = Vec::new();
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut list_no: usize = 0;

    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');

        if line.trim_start().starts_with("```") {
            if in_fence {
                in_fence = false;
                fence_lang.clear();
                out.push(MdLine {
                    spans: vec![MdSpan {
                        text: format!("``` {}", fence_lang),
                        kind: MdKind::Code,
                        bold: false,
                        code: true,
                        fence_depth: 0,
                    }],
                    kind: MdKind::Code,
                });
                continue;
            }
            in_fence = true;
            fence_lang = line.trim_start().trim_start_matches('`').trim().to_string();
            out.push(MdLine {
                spans: vec![MdSpan {
                    text: format!("```{}", fence_lang),
                    kind: MdKind::Code,
                    bold: false,
                    code: true,
                    fence_depth: 1,
                }],
                kind: MdKind::Code,
            });
            continue;
        }

        if in_fence {
            for chunk in hard_wrap(line, cols) {
                out.push(MdLine {
                    spans: vec![MdSpan {
                        text: chunk,
                        kind: MdKind::Code,
                        bold: false,
                        code: true,
                        fence_depth: 1,
                    }],
                    kind: MdKind::Code,
                });
            }
            continue;
        }

        let trimmed = line.trim_start();

        // Horizontal rule
        if trimmed.len() >= 3
            && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_')
            && trimmed.chars().any(|c| c != ' ')
        {
            out.push(MdLine {
                spans: vec![MdSpan {
                    text: String::new(),
                    kind: MdKind::Rule,
                    bold: false,
                    code: false,
                    fence_depth: 0,
                }],
                kind: MdKind::Rule,
            });
            list_no = 0;
            continue;
        }

        // Heading
        if let Some(rest) = trimmed.strip_prefix('#') {
            let level = rest.len() - rest.trim_start_matches('#').len() + 1;
            let body = rest.trim_start_matches('#').trim();
            let body = body.trim_end_matches('#').trim();
            let _ = level;
            out.push(MdLine {
                spans: inline_spans(body, cols),
                kind: MdKind::Heading,
            });
            list_no = 0;
            continue;
        }

        // Block quote
        if let Some(rest) = trimmed.strip_prefix('>') {
            let body = rest.trim_start();
            let body = format!("| {}", body);
            for chunk in wrap_words(body, cols.saturating_sub(2)) {
                out.push(MdLine {
                    spans: inline_spans(&chunk, cols),
                    kind: MdKind::Quote,
                });
            }
            list_no = 0;
            continue;
        }

        // Bullet
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let body = format!("* {}", rest.trim());
            for chunk in wrap_words(body, cols.saturating_sub(2)) {
                out.push(MdLine {
                    spans: inline_spans(&chunk, cols),
                    kind: MdKind::Bullet,
                });
            }
            list_no = 0;
            continue;
        }

        // Ordered list
        if let Some((num, rest)) = split_ordered(trimmed) {
            list_no = num;
            let body = format!("{}. {}", num, rest.trim());
            for chunk in wrap_words(body, cols.saturating_sub(2)) {
                out.push(MdLine {
                    spans: inline_spans(&chunk, cols),
                    kind: MdKind::Bullet,
                });
            }
            continue;
        }

        // Blank line
        if trimmed.is_empty() {
            out.push(MdLine {
                spans: vec![MdSpan {
                    text: String::new(),
                    kind: MdKind::Text,
                    bold: false,
                    code: false,
                    fence_depth: 0,
                }],
                kind: MdKind::Text,
            });
            list_no = 0;
            continue;
        }

        // Paragraph — preserve explicit indentation of continuation lines.
        let indent = line.len() - trimmed.len();
        let body = if indent > 0 && list_no > 0 {
            format!("{}{}", " ".repeat(indent), trimmed)
        } else {
            trimmed.to_string()
        };
        for chunk in wrap_words(body, cols) {
            out.push(MdLine {
                spans: inline_spans(&chunk, cols),
                kind: MdKind::Text,
            });
        }
    }

    if out.is_empty() {
        out.push(MdLine {
            spans: vec![MdSpan {
                text: String::new(),
                kind: MdKind::Text,
                bold: false,
                code: false,
                fence_depth: 0,
            }],
            kind: MdKind::Text,
        });
    }
    out
}

fn split_ordered(s: &str) -> Option<(usize, &str)> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &s[digits.len()..];
    let rest = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    Some((digits.parse().ok()?, rest))
}

/// Break a line that has no spaces (URLs, long identifiers) at `cols`.
fn hard_wrap(line: &str, cols: usize) -> Vec<String> {
    if line.chars().count() <= cols {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut rest = line;
    while rest.chars().count() > cols {
        let head: String = rest.chars().take(cols).collect();
        rest = &rest[head.len()..];
        out.push(head);
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

fn wrap_words<S: AsRef<str>>(text: S, cols: usize) -> Vec<String> {
    let text = text.as_ref();
    if text.chars().count() <= cols {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let wlen = word.chars().count();
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.chars().count() + 1 + wlen <= cols {
            cur.push(' ');
            cur.push_str(word);
        } else {
            out.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
        for chunk in hard_wrap(&cur, cols) {
            if chunk != cur {
                out.extend(hard_wrap(&cur, cols));
                cur.clear();
                break;
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Parse `` `code` ``, `**bold**` and `*italic*` inside a single visual line.
fn inline_spans(line: &str, cols: usize) -> Vec<MdSpan> {
    let mut spans: Vec<MdSpan> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let bold = false;
    let code = false;

    let flush = |buf: &mut String, spans: &mut Vec<MdSpan>, bold: bool, code: bool| {
        if buf.is_empty() {
            return;
        }
        spans.push(MdSpan {
            text: std::mem::take(buf),
            kind: if code {
                MdKind::Code
            } else if bold {
                MdKind::Heading
            } else {
                MdKind::Text
            },
            bold,
            code,
            fence_depth: 0,
        });
    };

    while i < chars.len() {
        let c = chars[i];
        // Inline code
        if c == '`' {
            if let Some(end) = (i + 1..chars.len()).find(|&k| chars[k] == '`') {
                flush(&mut buf, &mut spans, bold, code);
                let inner: String = chars[i + 1..end].iter().collect();
                if !inner.is_empty() {
                    spans.push(MdSpan {
                        text: inner,
                        kind: MdKind::Code,
                        bold,
                        code: true,
                        fence_depth: 0,
                    });
                }
                i = end + 1;
                continue;
            }
        }
        // Bold
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if let Some(end) = (i + 2..chars.len()).find(|&k| chars[k] == '*' && chars.get(k + 1) == Some(&'*')) {
                flush(&mut buf, &mut spans, bold, code);
                let inner: String = chars[i + 2..end].iter().collect();
                spans.push(MdSpan {
                    text: inner,
                    kind: MdKind::Heading,
                    bold: true,
                    code,
                    fence_depth: 0,
                });
                i = end + 2;
                continue;
            }
        }
        // Italic
        if c == '*' || c == '_' {
            if let Some(end) = (i + 1..chars.len()).find(|&k| chars[k] == c) {
                if end > i + 1 {
                    flush(&mut buf, &mut spans, bold, code);
                    let inner: String = chars[i + 1..end].iter().collect();
                    spans.push(MdSpan {
                        text: inner,
                        kind: MdKind::Quote,
                        bold,
                        code,
                        fence_depth: 0,
                    });
                    i = end + 1;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut spans, bold, code);

    if spans.is_empty() {
        spans.push(MdSpan {
            text: String::new(),
            kind: MdKind::Text,
            bold: false,
            code: false,
            fence_depth: 0,
        });
    }
    // Guard against pathological spans produced by the inline scanner.
    for s in spans.iter_mut() {
        if s.text.chars().count() > cols * 4 {
            s.text = crate::ui::shell::theme::truncate_right(&s.text, cols);
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(lines: &[MdLine]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.text.clone())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_headings_bullets_and_quotes() {
        let md = "# Title\n\n- one\n- two\n\n> quoted\n1. first\n2. second";
        let lines = render_markdown(md, 40);
        let text = flat(&lines);
        assert!(text.contains("Title"));
        assert!(text.contains("* one"));
        assert!(text.contains("* two"));
        assert!(text.contains("| quoted"));
        assert!(text.contains("1. first"));
        assert!(text.contains("2. second"));
        assert_eq!(lines[0].kind, MdKind::Heading);
    }

    #[test]
    fn test_fenced_code_is_verbatim() {
        let md = "before\n```rust\nfn main() {\n    let x = 1;\n}\n```\nafter";
        let lines = render_markdown(md, 80);
        let text = flat(&lines);
        assert!(text.contains("fn main() {"));
        assert!(text.contains("    let x = 1;"));
        assert!(text.contains("```rust"));
        assert!(text.contains("```"));
        let code_rows = lines
            .iter()
            .filter(|l| l.kind == MdKind::Code)
            .count();
        assert!(code_rows >= 4);
    }

    #[test]
    fn test_inline_bold_and_code() {
        let md = "Use **bold** and `code` now";
        let lines = render_markdown(md, 60);
        let spans = &lines[0].spans;
        assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
        assert!(spans.iter().any(|s| s.code && s.text == "code"));
    }

    #[test]
    fn test_wrapping_never_exceeds_columns() {
        let md = "word ".repeat(200);
        let lines = render_markdown(&md, 40);
        for l in &lines {
            let total: usize = l.spans.iter().map(|s| s.text.chars().count()).sum();
            assert!(total <= 40, "line too long: {}", total);
        }
    }

    #[test]
    fn test_rule_and_blank_lines() {
        let md = "a\n\n---\n\nb";
        let lines = render_markdown(md, 30);
        assert!(lines.iter().any(|l| l.kind == MdKind::Rule));
        assert!(lines.iter().any(|l| l.spans.is_empty() || l.spans[0].text.is_empty()));
    }
}
