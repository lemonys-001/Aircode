//! Pure-Rust multi-language lexical syntax highlighter for the native code editor.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    TypeName,
    StringLit,
    NumberLit,
    Comment,
    MacroAttr,
    Function,
    Operator,
}

impl TokenKind {
    pub fn rgb(self) -> u32 {
        match self {
            TokenKind::Plain => 0xE2E8F0,
            TokenKind::Keyword => 0xC084FC,   // Vivid Purple
            TokenKind::TypeName => 0x38BDF8,  // Sky Cyan
            TokenKind::StringLit => 0x34D399, // Emerald Green
            TokenKind::NumberLit => 0xFBBF24, // Amber Gold
            TokenKind::Comment => 0x64748B,   // Muted Slate
            TokenKind::MacroAttr => 0x22D3EE, // Bright Cyan
            TokenKind::Function => 0x60A5FA,  // Soft Blue
            TokenKind::Operator => 0x94A3B8,  // Cool Gray
        }
    }

    pub fn themed_rgb(self, pal: &crate::themes::ThemePalette) -> u32 {
        match self {
            TokenKind::Plain => pal.text_primary,
            TokenKind::Keyword => pal.syn_keyword,
            TokenKind::TypeName => pal.syn_type,
            TokenKind::StringLit => pal.syn_string,
            TokenKind::NumberLit => pal.syn_number,
            TokenKind::Comment => pal.syn_comment,
            TokenKind::MacroAttr => pal.accent_secondary,
            TokenKind::Function => pal.syn_func,
            TokenKind::Operator => pal.text_secondary,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SyntaxSpan {
    pub text: String,
    pub kind: TokenKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LangFamily {
    Rust,
    TypeScript,
    Python,
    CFamily,
    Go,
    Json,
    TomlYaml,
    ShellPs1,
    Sql,
    Markdown,
    Plain,
}

impl LangFamily {
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => LangFamily::Rust,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => LangFamily::TypeScript,
            "py" | "pyw" => LangFamily::Python,
            "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "cs" | "java" => LangFamily::CFamily,
            "go" => LangFamily::Go,
            "json" => LangFamily::Json,
            "toml" | "yaml" | "yml" | "ini" | "cfg" => LangFamily::TomlYaml,
            "ps1" | "psm1" | "sh" | "bash" | "bat" | "cmd" => LangFamily::ShellPs1,
            "sql" => LangFamily::Sql,
            "md" | "markdown" => LangFamily::Markdown,
            _ => LangFamily::Plain,
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            LangFamily::Rust => "RS",
            LangFamily::TypeScript => "TS",
            LangFamily::Python => "PY",
            LangFamily::CFamily => "C++",
            LangFamily::Go => "GO",
            LangFamily::Json => "JSON",
            LangFamily::TomlYaml => "CFG",
            LangFamily::ShellPs1 => "PS1",
            LangFamily::Sql => "SQL",
            LangFamily::Markdown => "MD",
            LangFamily::Plain => "TXT",
        }
    }
}

fn is_keyword(lang: LangFamily, word: &str) -> bool {
    match lang {
        LangFamily::Rust => matches!(
            word,
            "fn" | "let" | "mut" | "pub" | "struct" | "enum" | "impl" | "trait" | "use" | "mod"
                | "crate" | "super" | "self" | "Self" | "match" | "if" | "else" | "for" | "while"
                | "loop" | "return" | "break" | "continue" | "async" | "await" | "move" | "where"
                | "const" | "static" | "type" | "unsafe" | "extern" | "ref" | "in" | "as" | "true"
                | "false" | "dyn"
        ),
        LangFamily::TypeScript => matches!(
            word,
            "function" | "const" | "let" | "var" | "return" | "if" | "else" | "for" | "while"
                | "switch" | "case" | "break" | "continue" | "import" | "export" | "from"
                | "default" | "interface" | "type" | "class" | "extends" | "implements" | "new"
                | "async" | "await" | "try" | "catch" | "finally" | "throw" | "true" | "false"
                | "null" | "undefined"
        ),
        LangFamily::Python => matches!(
            word,
            "def" | "class" | "import" | "from" | "as" | "return" | "if" | "elif" | "else" | "for"
                | "while" | "in" | "not" | "and" | "or" | "try" | "except" | "finally" | "with"
                | "lambda" | "yield" | "async" | "await" | "pass" | "break" | "continue" | "True"
                | "False" | "None" | "raise"
        ),
        LangFamily::CFamily => matches!(
            word,
            "int" | "void" | "char" | "bool" | "float" | "double" | "long" | "unsigned" | "struct"
                | "class" | "namespace" | "template" | "typename" | "public" | "private"
                | "protected" | "virtual" | "override" | "const" | "constexpr" | "static"
                | "inline" | "return" | "if" | "else" | "for" | "while" | "switch" | "case"
                | "break" | "continue" | "new" | "delete" | "nullptr" | "true" | "false" | "using"
        ),
        LangFamily::Go => matches!(
            word,
            "func" | "package" | "import" | "var" | "const" | "type" | "struct" | "interface"
                | "return" | "if" | "else" | "for" | "range" | "switch" | "case" | "default"
                | "go" | "defer" | "chan" | "select" | "map" | "true" | "false" | "nil"
        ),
        LangFamily::ShellPs1 => matches!(
            word,
            "if" | "else" | "elseif" | "fi" | "then" | "for" | "foreach" | "in" | "do" | "done"
                | "function" | "param" | "return" | "exit" | "set" | "export" | "local" | "echo"
                | "Write-Host" | "Get-ChildItem" | "Test-Path" | "Copy-Item" | "New-Item"
        ),
        LangFamily::Sql => matches!(
            word.to_ascii_uppercase().as_str(),
            "SELECT" | "FROM" | "WHERE" | "INSERT" | "INTO" | "VALUES" | "UPDATE" | "SET"
                | "DELETE" | "CREATE" | "TABLE" | "IF" | "NOT" | "EXISTS" | "PRIMARY" | "KEY"
                | "ORDER" | "BY" | "LIMIT" | "JOIN" | "LEFT" | "INNER" | "ON" | "AND" | "OR"
                | "NULL" | "PRAGMA"
        ),
        _ => false,
    }
}

fn is_builtin_type(lang: LangFamily, word: &str) -> bool {
    if word
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
        && word.chars().any(|c| c.is_ascii_lowercase())
    {
        return true;
    }
    match lang {
        LangFamily::Rust => matches!(
            word,
            "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize"
                | "f32" | "f64" | "bool" | "str" | "char"
        ),
        LangFamily::TypeScript => matches!(
            word,
            "string" | "number" | "boolean" | "any" | "unknown" | "never" | "void"
        ),
        _ => false,
    }
}

pub fn tokenize_line(lang: LangFamily, line: &str) -> Vec<SyntaxSpan> {
    if line.is_empty() {
        return Vec::new();
    }
    if lang == LangFamily::Markdown {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            return vec![SyntaxSpan {
                text: line.to_string(),
                kind: TokenKind::TypeName,
            }];
        }
        if trimmed.starts_with("```") {
            return vec![SyntaxSpan {
                text: line.to_string(),
                kind: TokenKind::MacroAttr,
            }];
        }
    }

    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut spans: Vec<SyntaxSpan> = Vec::new();

    let push_span = |spans: &mut Vec<SyntaxSpan>, kind: TokenKind, s: String| {
        if s.is_empty() {
            return;
        }
        if let Some(last) = spans.last_mut() {
            if last.kind == kind {
                last.text.push_str(&s);
                return;
            }
        }
        spans.push(SyntaxSpan { text: s, kind });
    };

    while i < n {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        // Line comments
        let is_slash_comment = matches!(
            lang,
            LangFamily::Rust | LangFamily::TypeScript | LangFamily::CFamily | LangFamily::Go
        ) && ch == '/'
            && (next == Some('/') || next == Some('*'));
        let is_hash_comment = matches!(
            lang,
            LangFamily::Python | LangFamily::TomlYaml | LangFamily::ShellPs1
        ) && ch == '#';
        let is_sql_comment = lang == LangFamily::Sql && ch == '-' && next == Some('-');

        if is_slash_comment || is_hash_comment || is_sql_comment {
            let rest: String = chars[i..].iter().collect();
            push_span(&mut spans, TokenKind::Comment, rest);
            break;
        }

        // Rust/C preprocessor or attributes
        if (lang == LangFamily::Rust && ch == '#' && next == Some('['))
            || (lang == LangFamily::CFamily && ch == '#')
        {
            let rest: String = chars[i..].iter().collect();
            push_span(&mut spans, TokenKind::MacroAttr, rest);
            break;
        }

        // Strings
        if ch == '"' || (ch == '\'' && lang != LangFamily::Rust) {
            let quote = ch;
            let start = i;
            i += 1;
            while i < n {
                if chars[i] == '\\' {
                    i = (i + 2).min(n);
                    continue;
                }
                if chars[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            push_span(&mut spans, TokenKind::StringLit, s);
            continue;
        }

        // Numbers
        if ch.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < n
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            push_span(&mut spans, TokenKind::NumberLit, s);
            continue;
        }

        // Identifiers / Keywords / Types / Functions / Macros
        if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
            let start = i;
            i += 1;
            while i < n
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '_'
                    || (lang == LangFamily::ShellPs1 && chars[i] == '-'))
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if lang == LangFamily::Rust && chars.get(i) == Some(&'!') {
                i += 1;
                let mut m = word;
                m.push('!');
                push_span(&mut spans, TokenKind::MacroAttr, m);
            } else if is_keyword(lang, &word) {
                push_span(&mut spans, TokenKind::Keyword, word);
            } else if is_builtin_type(lang, &word) {
                push_span(&mut spans, TokenKind::TypeName, word);
            } else if chars.get(i) == Some(&'(') {
                push_span(&mut spans, TokenKind::Function, word);
            } else {
                push_span(&mut spans, TokenKind::Plain, word);
            }
            continue;
        }

        // Operators & punctuation
        let kind = if ch.is_ascii_punctuation() {
            TokenKind::Operator
        } else {
            TokenKind::Plain
        };
        push_span(&mut spans, kind, ch.to_string());
        i += 1;
    }

    spans
}
