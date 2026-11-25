//! Pure-Rust multi-tab code editor buffer, undo/redo stack, selection, and side-by-side Diff engine.

use crate::syntax::LangFamily;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Unchanged,
    Removed,
    Added,
    Changed,
}

#[derive(Clone, Debug)]
pub struct SideBySideDiffRow {
    pub left_no: Option<usize>,
    pub left_text: String,
    pub right_no: Option<usize>,
    pub right_text: String,
    pub kind: DiffLineKind,
}

pub fn build_side_by_side_diff(original: &str, modified: &str) -> Vec<SideBySideDiffRow> {
    let left: Vec<&str> = original.lines().collect();
    let right: Vec<&str> = modified.lines().collect();

    let mut prefix = 0usize;
    while prefix < left.len() && prefix < right.len() && left[prefix] == right[prefix] {
        prefix += 1;
    }

    let mut left_end = left.len();
    let mut right_end = right.len();
    while left_end > prefix && right_end > prefix && left[left_end - 1] == right[right_end - 1] {
        left_end -= 1;
        right_end -= 1;
    }

    let mut rows = Vec::with_capacity(left.len().max(right.len()));
    for i in 0..prefix {
        rows.push(SideBySideDiffRow {
            left_no: Some(i + 1),
            left_text: left[i].to_string(),
            right_no: Some(i + 1),
            right_text: right[i].to_string(),
            kind: DiffLineKind::Unchanged,
        });
    }

    let mid_left = &left[prefix..left_end];
    let mid_right = &right[prefix..right_end];
    let max_mid = mid_left.len().max(mid_right.len());

    for k in 0..max_mid {
        match (mid_left.get(k), mid_right.get(k)) {
            (Some(l), Some(r)) if l == r => rows.push(SideBySideDiffRow {
                left_no: Some(prefix + k + 1),
                left_text: (*l).to_string(),
                right_no: Some(prefix + k + 1),
                right_text: (*r).to_string(),
                kind: DiffLineKind::Unchanged,
            }),
            (Some(l), Some(r)) => rows.push(SideBySideDiffRow {
                left_no: Some(prefix + k + 1),
                left_text: (*l).to_string(),
                right_no: Some(prefix + k + 1),
                right_text: (*r).to_string(),
                kind: DiffLineKind::Changed,
            }),
            (Some(l), None) => rows.push(SideBySideDiffRow {
                left_no: Some(prefix + k + 1),
                left_text: (*l).to_string(),
                right_no: None,
                right_text: String::new(),
                kind: DiffLineKind::Removed,
            }),
            (None, Some(r)) => rows.push(SideBySideDiffRow {
                left_no: None,
                left_text: String::new(),
                right_no: Some(prefix + k + 1),
                right_text: (*r).to_string(),
                kind: DiffLineKind::Added,
            }),
            (None, None) => {}
        }
    }

    let suffix_len = left.len() - left_end;
    for s in 0..suffix_len {
        let li = left_end + s;
        let ri = right_end + s;
        rows.push(SideBySideDiffRow {
            left_no: Some(li + 1),
            left_text: left[li].to_string(),
            right_no: Some(ri + 1),
            right_text: right[ri].to_string(),
            kind: DiffLineKind::Unchanged,
        });
    }

    rows
}

#[derive(Clone, Debug)]
pub struct UndoSnapshot {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

#[derive(Clone, Debug)]
pub enum TabContent {
    Code {
        lines: Vec<String>,
        saved_text: String,
        cursor_line: usize,
        cursor_col: usize,
        sel_anchor: Option<(usize, usize)>,
        scroll_y: usize,
        scroll_x: usize,
        readonly: bool,
        undo_stack: Vec<UndoSnapshot>,
        redo_stack: Vec<UndoSnapshot>,
    },
    Diff {
        rows: Vec<SideBySideDiffRow>,
        scroll_y: usize,
    },
}

#[derive(Clone, Debug)]
pub struct EditorTab {
    pub id: String,
    pub title: String,
    pub rel_path: String,
    pub full_path: Option<PathBuf>,
    pub lang: LangFamily,
    pub content: TabContent,
}

impl EditorTab {
    pub fn new_code(
        title: String,
        rel_path: String,
        full_path: Option<PathBuf>,
        raw_text: &str,
        readonly: bool,
    ) -> Self {
        let normalized = raw_text.replace("\r\n", "\n");
        let mut lines: Vec<String> = normalized.split('\n').map(|s| s.to_string()).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let id = format!("code:{}", rel_path);
        let lang = LangFamily::from_path(&rel_path);
        Self {
            id,
            title,
            rel_path,
            full_path,
            lang,
            content: TabContent::Code {
                lines,
                saved_text: normalized,
                cursor_line: 0,
                cursor_col: 0,
                sel_anchor: None,
                scroll_y: 0,
                scroll_x: 0,
                readonly,
                undo_stack: Vec::new(),
                redo_stack: Vec::new(),
            },
        }
    }

    pub fn new_diff(rel_path: String, original: &str, modified: &str) -> Self {
        let short = rel_path.rsplit(['/', '\\']).next().unwrap_or(&rel_path);
        let rows = build_side_by_side_diff(original, modified);
        Self {
            id: format!("diff:{}", rel_path),
            title: format!("Diff: {}", short),
            rel_path: rel_path.clone(),
            full_path: None,
            lang: LangFamily::from_path(&rel_path),
            content: TabContent::Diff { rows, scroll_y: 0 },
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.content {
            TabContent::Code {
                lines, saved_text, ..
            } => lines.join("\n") != *saved_text,
            TabContent::Diff { .. } => false,
        }
    }

    pub fn full_text(&self) -> String {
        match &self.content {
            TabContent::Code { lines, .. } => lines.join("\n"),
            TabContent::Diff { .. } => String::new(),
        }
    }

    pub fn mark_saved(&mut self) {
        if let TabContent::Code {
            lines, saved_text, ..
        } = &mut self.content
        {
            *saved_text = lines.join("\n");
        }
    }

    fn push_undo(&mut self) {
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            undo_stack,
            redo_stack,
            ..
        } = &mut self.content
        {
            if undo_stack.len() >= 80 {
                undo_stack.remove(0);
            }
            undo_stack.push(UndoSnapshot {
                lines: lines.clone(),
                cursor_line: *cursor_line,
                cursor_col: *cursor_col,
            });
            redo_stack.clear();
        }
    }

    pub fn undo(&mut self) {
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            undo_stack,
            redo_stack,
            readonly,
            ..
        } = &mut self.content
        {
            if *readonly {
                return;
            }
            if let Some(prev) = undo_stack.pop() {
                redo_stack.push(UndoSnapshot {
                    lines: lines.clone(),
                    cursor_line: *cursor_line,
                    cursor_col: *cursor_col,
                });
                *lines = prev.lines;
                *cursor_line = prev.cursor_line.min(lines.len().saturating_sub(1));
                *cursor_col = prev.cursor_col.min(char_len(&lines[*cursor_line]));
                *sel_anchor = None;
            }
        }
    }

    pub fn redo(&mut self) {
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            undo_stack,
            redo_stack,
            readonly,
            ..
        } = &mut self.content
        {
            if *readonly {
                return;
            }
            if let Some(next) = redo_stack.pop() {
                undo_stack.push(UndoSnapshot {
                    lines: lines.clone(),
                    cursor_line: *cursor_line,
                    cursor_col: *cursor_col,
                });
                *lines = next.lines;
                *cursor_line = next.cursor_line.min(lines.len().saturating_sub(1));
                *cursor_col = next.cursor_col.min(char_len(&lines[*cursor_line]));
                *sel_anchor = None;
            }
        }
    }

    pub fn ordered_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        if let TabContent::Code {
            cursor_line,
            cursor_col,
            sel_anchor: Some(anchor),
            ..
        } = &self.content
        {
            let cur = (*cursor_line, *cursor_col);
            if *anchor == cur {
                None
            } else if *anchor < cur {
                Some((*anchor, cur))
            } else {
                Some((cur, *anchor))
            }
        } else {
            None
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((sl, sc), (el, ec)) = self.ordered_selection()?;
        if let TabContent::Code { lines, .. } = &self.content {
            if sl == el {
                return Some(slice_chars(&lines[sl], sc, ec));
            }
            let mut out = String::new();
            out.push_str(&slice_chars(&lines[sl], sc, char_len(&lines[sl])));
            out.push('\n');
            for row in (sl + 1)..el {
                out.push_str(&lines[row]);
                out.push('\n');
            }
            out.push_str(&slice_chars(&lines[el], 0, ec));
            Some(out)
        } else {
            None
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some(((sl, sc), (el, ec))) = self.ordered_selection() else {
            return false;
        };
        self.push_undo();
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            ..
        } = &mut self.content
        {
            let prefix = slice_chars(&lines[sl], 0, sc);
            let suffix = slice_chars(&lines[el], ec, char_len(&lines[el]));
            lines[sl] = format!("{}{}", prefix, suffix);
            if el > sl {
                lines.drain((sl + 1)..=el);
            }
            *cursor_line = sl;
            *cursor_col = sc;
            *sel_anchor = None;
            return true;
        }
        false
    }

    pub fn select_all(&mut self) {
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            ..
        } = &mut self.content
        {
            *sel_anchor = Some((0, 0));
            let last = lines.len().saturating_sub(1);
            *cursor_line = last;
            *cursor_col = char_len(&lines[last]);
        }
    }

    pub fn insert_str(&mut self, text: &str) {
        let is_ro = matches!(&self.content, TabContent::Code { readonly: true, .. } | TabContent::Diff { .. });
        if is_ro || text.is_empty() {
            return;
        }
        if !self.delete_selection() {
            self.push_undo();
        }
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            ..
        } = &mut self.content
        {
            let clean = text.replace("\r\n", "\n").replace('\r', "\n");
            let parts: Vec<&str> = clean.split('\n').collect();
            let cur_line = &lines[*cursor_line];
            let left = slice_chars(cur_line, 0, *cursor_col);
            let right = slice_chars(cur_line, *cursor_col, char_len(cur_line));

            if parts.len() == 1 {
                lines[*cursor_line] = format!("{}{}{}", left, parts[0], right);
                *cursor_col += parts[0].chars().count();
            } else {
                lines[*cursor_line] = format!("{}{}", left, parts[0]);
                for (idx, mid) in parts.iter().enumerate().skip(1) {
                    let insert_at = *cursor_line + idx;
                    if idx + 1 == parts.len() {
                        lines.insert(insert_at, format!("{}{}", mid, right));
                        *cursor_line = insert_at;
                        *cursor_col = mid.chars().count();
                    } else {
                        lines.insert(insert_at, (*mid).to_string());
                    }
                }
            }
            *sel_anchor = None;
        }
    }

    pub fn insert_smart_char(&mut self, ch: char) {
        let is_ro = matches!(
            &self.content,
            TabContent::Code { readonly: true, .. } | TabContent::Diff { .. }
        );
        if is_ro {
            return;
        }
        let pair_close = match ch {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            _ => None,
        };

        // Wrap selection if user typed an opening delimiter while text is selected
        if let Some(close_ch) = pair_close {
            if let Some(sel) = self.selected_text() {
                self.insert_str(&format!("{}{}{}", ch, sel, close_ch));
                return;
            }
        }

        // Skip over closing delimiter if already right under cursor
        if matches!(ch, ')' | ']' | '}' | '"' | '\'') {
            if let TabContent::Code {
                lines,
                cursor_line,
                cursor_col,
                sel_anchor,
                ..
            } = &mut self.content
            {
                if sel_anchor.is_none() {
                    if let Some(next_c) = lines[*cursor_line].chars().nth(*cursor_col) {
                        if next_c == ch {
                            *cursor_col += 1;
                            return;
                        }
                    }
                }
            }
        }

        if let Some(close_ch) = pair_close {
            let should_pair = if let TabContent::Code {
                lines,
                cursor_line,
                cursor_col,
                ..
            } = &self.content
            {
                let next_opt = lines[*cursor_line].chars().nth(*cursor_col);
                match next_opt {
                    None => true,
                    Some(nc) => nc.is_whitespace() || matches!(nc, ')' | ']' | '}' | ',' | ';'),
                }
            } else {
                false
            };

            if should_pair {
                self.insert_str(&format!("{}{}", ch, close_ch));
                if let TabContent::Code { cursor_col, .. } = &mut self.content {
                    *cursor_col = cursor_col.saturating_sub(1);
                }
                return;
            }
        }

        self.insert_str(&ch.to_string());
    }

    pub fn insert_newline_auto_indent(&mut self) {
        let (indent, prev_ch, next_ch) = if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            ..
        } = &self.content
        {
            let line = &lines[*cursor_line];
            let ind = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>();
            let prev = if *cursor_col > 0 {
                line.chars().nth(*cursor_col - 1)
            } else {
                None
            };
            let next = line.chars().nth(*cursor_col);
            (ind, prev, next)
        } else {
            return;
        };

        let opens_block = matches!(prev_ch, Some('{') | Some('(') | Some('[') | Some(':'));
        let between_braces = matches!(
            (prev_ch, next_ch),
            (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')'))
        );

        if between_braces {
            let inner_indent = format!("{}    ", indent);
            let inserted = format!("\n{}\n{}", inner_indent, indent);
            self.insert_str(&inserted);
            if let TabContent::Code {
                cursor_line,
                cursor_col,
                ..
            } = &mut self.content
            {
                *cursor_line = cursor_line.saturating_sub(1);
                *cursor_col = char_len(&inner_indent);
            }
        } else if opens_block {
            self.insert_str(&format!("\n{}    ", indent));
        } else {
            self.insert_str(&format!("\n{}", indent));
        }
    }

    pub fn indent_block(&mut self, unindent: bool) {
        if matches!(
            &self.content,
            TabContent::Code { readonly: true, .. } | TabContent::Diff { .. }
        ) {
            return;
        }
        let sel = self.ordered_selection();
        if sel.is_none() && !unindent {
            self.insert_str("    ");
            return;
        }
        self.push_undo();
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            ..
        } = &mut self.content
        {
            let (sl, el) = sel
                .map(|((a, _), (b, _))| (a, b))
                .unwrap_or((*cursor_line, *cursor_line));
            for row in sl..=el.min(lines.len().saturating_sub(1)) {
                if unindent {
                    let remove_spaces = lines[row]
                        .chars()
                        .take(4)
                        .take_while(|c| *c == ' ')
                        .count();
                    if remove_spaces > 0 {
                        lines[row] = slice_chars(&lines[row], remove_spaces, char_len(&lines[row]));
                        if row == *cursor_line {
                            *cursor_col = cursor_col.saturating_sub(remove_spaces);
                        }
                    } else if lines[row].starts_with('\t') {
                        lines[row] = slice_chars(&lines[row], 1, char_len(&lines[row]));
                        if row == *cursor_line {
                            *cursor_col = cursor_col.saturating_sub(1);
                        }
                    }
                } else {
                    lines[row].insert_str(0, "    ");
                    if row == *cursor_line {
                        *cursor_col += 4;
                    }
                }
            }
        }
    }

    pub fn toggle_comment(&mut self) {
        if matches!(
            &self.content,
            TabContent::Code { readonly: true, .. } | TabContent::Diff { .. }
        ) {
            return;
        }
        let sel = self.ordered_selection();
        let comment_prefix = match self.lang {
            LangFamily::Python | LangFamily::ShellPs1 | LangFamily::TomlYaml => "#",
            _ => "//",
        };
        self.push_undo();
        if let TabContent::Code {
            lines, cursor_line, ..
        } = &mut self.content
        {
            let (sl, el) = sel
                .map(|((a, _), (b, _))| (a, b))
                .unwrap_or((*cursor_line, *cursor_line));
            let all_commented = (sl..=el).all(|r| {
                let t = lines[r].trim_start();
                t.is_empty() || t.starts_with(comment_prefix)
            });
            for r in sl..=el {
                if lines[r].trim().is_empty() {
                    continue;
                }
                if all_commented {
                    if let Some(idx) = lines[r].find(comment_prefix) {
                        let after = idx + comment_prefix.len();
                        let strip_extra = usize::from(lines[r].as_bytes().get(after) == Some(&b' '));
                        lines[r].replace_range(idx..(after + strip_extra), "");
                    }
                } else {
                    let lead = lines[r]
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .count();
                    let pfx = slice_chars(&lines[r], 0, lead);
                    let rest = slice_chars(&lines[r], lead, char_len(&lines[r]));
                    lines[r] = format!("{}{} {}", pfx, comment_prefix, rest);
                }
            }
        }
    }

    pub fn move_line_vertical(&mut self, delta: isize, visible_rows: usize) {
        if matches!(
            &self.content,
            TabContent::Code { readonly: true, .. } | TabContent::Diff { .. }
        ) {
            return;
        }
        self.push_undo();
        if let TabContent::Code {
            lines,
            cursor_line,
            scroll_y,
            ..
        } = &mut self.content
        {
            let target = *cursor_line as isize + delta;
            if target >= 0 && (target as usize) < lines.len() {
                let tgt = target as usize;
                lines.swap(*cursor_line, tgt);
                *cursor_line = tgt;
                ensure_cursor_visible(*cursor_line, scroll_y, visible_rows);
            }
        }
    }

    pub fn goto_line(&mut self, one_based: usize, visible_rows: usize) {
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            scroll_y,
            ..
        } = &mut self.content
        {
            let target = one_based.saturating_sub(1).min(lines.len().saturating_sub(1));
            *cursor_line = target;
            *cursor_col = 0;
            *sel_anchor = None;
            ensure_cursor_visible(*cursor_line, scroll_y, visible_rows);
        }
    }

    pub fn select_word_or_next_match(&mut self, visible_rows: usize) {
        if let Some(sel) = self.selected_text() {
            if !sel.contains('\n') && !sel.is_empty() {
                let _ = self.find_next(&sel, visible_rows);
                return;
            }
        }
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            ..
        } = &mut self.content
        {
            let chars: Vec<char> = lines[*cursor_line].chars().collect();
            if chars.is_empty() {
                return;
            }
            let pos = (*cursor_col).min(chars.len() - 1);
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            if !is_word(chars[pos]) {
                return;
            }
            let mut start = pos;
            while start > 0 && is_word(chars[start - 1]) {
                start -= 1;
            }
            let mut end = pos + 1;
            while end < chars.len() && is_word(chars[end]) {
                end += 1;
            }
            *sel_anchor = Some((*cursor_line, start));
            *cursor_col = end;
        }
    }

    pub fn matching_bracket(&self) -> Option<(usize, usize)> {
        let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            ..
        } = &self.content
        else {
            return None;
        };
        let line_chars: Vec<char> = lines.get(*cursor_line)?.chars().collect();
        let (check_col, ch) = if let Some(&c) = line_chars.get(*cursor_col) {
            if matches!(c, '(' | ')' | '[' | ']' | '{' | '}') {
                (*cursor_col, c)
            } else if *cursor_col > 0
                && matches!(line_chars[*cursor_col - 1], '(' | ')' | '[' | ']' | '{' | '}')
            {
                (*cursor_col - 1, line_chars[*cursor_col - 1])
            } else {
                return None;
            }
        } else if *cursor_col > 0
            && matches!(
                line_chars.get(*cursor_col - 1),
                Some('(' | ')' | '[' | ']' | '{' | '}')
            )
        {
            (*cursor_col - 1, line_chars[*cursor_col - 1])
        } else {
            return None;
        };

        let (open_c, close_c, forward) = match ch {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => return None,
        };

        let mut depth = 0i32;
        if forward {
            for (r, l) in lines.iter().enumerate().skip(*cursor_line) {
                let cs: Vec<char> = l.chars().collect();
                let start = if r == *cursor_line { check_col } else { 0 };
                for (c, &cur) in cs.iter().enumerate().skip(start) {
                    if cur == open_c {
                        depth += 1;
                    } else if cur == close_c {
                        depth -= 1;
                        if depth == 0 {
                            return Some((r, c));
                        }
                    }
                }
            }
        } else {
            for r in (0..=*cursor_line).rev() {
                let cs: Vec<char> = lines[r].chars().collect();
                let end = if r == *cursor_line {
                    check_col + 1
                } else {
                    cs.len()
                };
                for c in (0..end).rev() {
                    let cur = cs[c];
                    if cur == close_c {
                        depth += 1;
                    } else if cur == open_c {
                        depth -= 1;
                        if depth == 0 {
                            return Some((r, c));
                        }
                    }
                }
            }
        }
        None
    }

    pub fn backspace(&mut self) {
        if matches!(&self.content, TabContent::Code { readonly: true, .. } | TabContent::Diff { .. }) {
            return;
        }
        if self.delete_selection() {
            return;
        }
        self.push_undo();
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            ..
        } = &mut self.content
        {
            if *cursor_col > 0 {
                let l = &lines[*cursor_line];
                let left = slice_chars(l, 0, *cursor_col - 1);
                let right = slice_chars(l, *cursor_col, char_len(l));
                lines[*cursor_line] = format!("{}{}", left, right);
                *cursor_col -= 1;
            } else if *cursor_line > 0 {
                let tail = lines.remove(*cursor_line);
                *cursor_line -= 1;
                let prev_len = char_len(&lines[*cursor_line]);
                lines[*cursor_line].push_str(&tail);
                *cursor_col = prev_len;
            }
        }
    }

    pub fn delete_forward(&mut self) {
        if matches!(&self.content, TabContent::Code { readonly: true, .. } | TabContent::Diff { .. }) {
            return;
        }
        if self.delete_selection() {
            return;
        }
        self.push_undo();
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            ..
        } = &mut self.content
        {
            let cur_len = char_len(&lines[*cursor_line]);
            if *cursor_col < cur_len {
                let l = &lines[*cursor_line];
                let left = slice_chars(l, 0, *cursor_col);
                let right = slice_chars(l, *cursor_col + 1, cur_len);
                lines[*cursor_line] = format!("{}{}", left, right);
            } else if *cursor_line + 1 < lines.len() {
                let next_line = lines.remove(*cursor_line + 1);
                lines[*cursor_line].push_str(&next_line);
            }
        }
    }

    pub fn move_cursor(&mut self, dline: isize, dcol: isize, extend_selection: bool, visible_rows: usize) {
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            scroll_y,
            ..
        } = &mut self.content
        {
            if extend_selection {
                if sel_anchor.is_none() {
                    *sel_anchor = Some((*cursor_line, *cursor_col));
                }
            } else {
                *sel_anchor = None;
            }

            if dline != 0 {
                let next_l = (*cursor_line as isize + dline).clamp(0, lines.len() as isize - 1) as usize;
                *cursor_line = next_l;
                *cursor_col = (*cursor_col).min(char_len(&lines[*cursor_line]));
            }
            if dcol < 0 {
                if *cursor_col > 0 {
                    *cursor_col -= 1;
                } else if *cursor_line > 0 {
                    *cursor_line -= 1;
                    *cursor_col = char_len(&lines[*cursor_line]);
                }
            } else if dcol > 0 {
                let max_c = char_len(&lines[*cursor_line]);
                if *cursor_col < max_c {
                    *cursor_col += 1;
                } else if *cursor_line + 1 < lines.len() {
                    *cursor_line += 1;
                    *cursor_col = 0;
                }
            }

            ensure_cursor_visible(*cursor_line, scroll_y, visible_rows);
        }
    }

    pub fn jump_home_end(&mut self, to_end: bool, extend_selection: bool) {
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            ..
        } = &mut self.content
        {
            if extend_selection && sel_anchor.is_none() {
                *sel_anchor = Some((*cursor_line, *cursor_col));
            } else if !extend_selection {
                *sel_anchor = None;
            }
            *cursor_col = if to_end {
                char_len(&lines[*cursor_line])
            } else {
                0
            };
        }
    }

    pub fn find_next(&mut self, query: &str, visible_rows: usize) -> bool {
        if query.is_empty() {
            return false;
        }
        let needle = query.to_lowercase();
        if let TabContent::Code {
            lines,
            cursor_line,
            cursor_col,
            sel_anchor,
            scroll_y,
            ..
        } = &mut self.content
        {
            let total = lines.len();
            for offset in 0..=total {
                let row = (*cursor_line + offset) % total;
                let hay = lines[row].to_lowercase();
                let start_char = if offset == 0 { *cursor_col + 1 } else { 0 };
                let sub = slice_chars(&hay, start_char, char_len(&hay));
                if let Some(byte_pos) = sub.find(&needle) {
                    let char_off = sub[..byte_pos].chars().count();
                    let match_start = start_char + char_off;
                    let match_end = match_start + needle.chars().count();
                    *sel_anchor = Some((row, match_start));
                    *cursor_line = row;
                    *cursor_col = match_end;
                    ensure_cursor_visible(*cursor_line, scroll_y, visible_rows);
                    return true;
                }
            }
        }
        false
    }
}

pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

pub fn slice_chars(s: &str, start: usize, end: usize) -> String {
    if start >= end {
        return String::new();
    }
    s.chars().skip(start).take(end - start).collect()
}

pub fn ensure_cursor_visible(cursor_line: usize, scroll_y: &mut usize, visible_rows: usize) {
    let vis = visible_rows.max(4);
    if cursor_line < *scroll_y {
        *scroll_y = cursor_line;
    } else if cursor_line >= *scroll_y + vis {
        *scroll_y = cursor_line + 1 - vis;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_insert_undo_and_diff() {
        let mut tab = EditorTab::new_code(
            "main.rs".to_string(),
            "src/main.rs".to_string(),
            None,
            "fn main() {\n}",
            false,
        );
        assert!(!tab.is_dirty());
        tab.move_cursor(0, 11, false, 20);
        tab.insert_newline_auto_indent();
        tab.insert_str("    println!(\"hi\");");
        assert!(tab.is_dirty());
        assert!(tab.full_text().contains("println!"));
        tab.undo();
        tab.undo();
        assert!(!tab.is_dirty());

        let diff = build_side_by_side_diff("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(diff.len(), 3);
        assert_eq!(diff[1].kind, DiffLineKind::Changed);
    }
}
