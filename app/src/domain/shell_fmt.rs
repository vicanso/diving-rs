//! Quote- and subshell-aware Shell & Dockerfile instruction formatter.
//!
//! Cleans BuildKit `RUN |N KEY=VAL ... /bin/sh -c` wrappers, extracts build
//! arguments cleanly, splits shell statements only at top-level operators
//! (`&&`, `||`, `;`, `;;`) without breaking inside `'...'`, `"..."`, `` `...` ``,
//! `$(...)`, or `( ... )`, indents POSIX control flow, and wraps long flag/package
//! runs onto continuation lines.

use std::mem::take;

use crate::ui::LayerOpcodeKind;

const SHELL_PREFIXES: [&str; 3] = ["/bin/sh -c ", "/bin/bash -c ", "/bin/ash -c "];
const SHELL_MARKERS: [&str; 3] = [" /bin/sh -c ", " /bin/bash -c ", " /bin/ash -c "];
const INDENT: &str = "    ";
const LINE_WRAP_LIMIT: usize = 88;
const TOKEN_CONT_LIMIT: usize = 80;

/// Structured parse and auto-formatted view of a single layer's `created_by` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLayerCommand {
    pub opcode: LayerOpcodeKind,
    pub summary: String,
    pub build_args: Vec<String>,
    pub formatted_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellOp {
    None,
    Semi,
    DoubleSemi,
    And,
    Or,
}

impl ShellOp {
    fn as_suffix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Semi => ";",
            Self::DoubleSemi => " ;;",
            Self::And => " &&",
            Self::Or => " ||",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellSegment {
    code: String,
    op: ShellOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexFrame {
    Normal,
    Single,
    Double,
    Backtick,
    CmdSubst { paren_depth: usize },
}

/// Parses, normalizes, and auto-formats a single layer command string.
pub fn parse_layer_command(raw_cmd: &str) -> ParsedLayerCommand {
    let trimmed = strip_buildkit_comment_suffix(raw_cmd.trim());
    if trimmed.is_empty() {
        return ParsedLayerCommand {
            opcode: LayerOpcodeKind::Meta,
            summary: "(metadata)".to_owned(),
            build_args: Vec::new(),
            formatted_lines: vec!["(metadata)".to_owned()],
        };
    }

    let mut had_nop = false;
    let mut had_sh = false;

    // Unwrap top-level `/bin/{sh,bash,ash} -c #(nop) ...` (even if prefixed with `RUN `).
    let work_base = if let Some(after_run) = strip_ascii_prefix_ci(trimmed, "RUN ")
        .map(str::trim_start)
        .filter(|rest| has_nop_shell_prefix(rest))
    {
        after_run
    } else {
        trimmed
    };
    let mut work = if let Some(body) = strip_nop_shell_prefix(work_base) {
        had_nop = true;
        body
    } else if let Some(body) = strip_shell_c_prefix(work_base) {
        had_sh = true;
        body
    } else if let Some(rest) = strip_nop_prefix(work_base) {
        had_nop = true;
        rest
    } else {
        work_base
    };

    work = work.trim();

    let mut explicit_op: Option<&'static str> = None;
    if let Some((op, rest)) = take_instruction_keyword(work) {
        explicit_op = Some(op);
        work = rest;
    }

    let mut build_args = Vec::new();
    let mut is_exec_json = work.starts_with('[');

    if !is_exec_json {
        if let Some((args, body)) = unwrap_buildkit_pipe_shell(work) {
            had_sh = true;
            build_args = args;
            work = body;
            if explicit_op.is_none() {
                explicit_op = Some("RUN");
            }
        } else if let Some(body) = strip_shell_c_prefix(work) {
            had_sh = true;
            work = body.trim();
            if explicit_op.is_none() {
                explicit_op = Some("RUN");
            }
        }
        is_exec_json = work.starts_with('[');
    }

    let mut cleaned_instr = if let Some(op) = explicit_op {
        if op == "EXPOSE" {
            clean_expose_line(&format!("{op} {work}"))
        } else if work.is_empty() {
            op.to_owned()
        } else {
            format!("{op} {}", collapse_spaces(work))
        }
    } else {
        collapse_spaces(work)
    };

    if explicit_op == Some("RUN") && !is_exec_json {
        // Legacy helper kept `"apk add ..."` without `"RUN "` when `/bin/sh -c` ran.
        cleaned_instr = collapse_spaces(work);
    }

    if cleaned_instr.is_empty() {
        return ParsedLayerCommand {
            opcode: LayerOpcodeKind::Meta,
            summary: "(metadata)".to_owned(),
            build_args,
            formatted_lines: vec!["(metadata)".to_owned()],
        };
    }

    let opcode = match explicit_op {
        Some("FROM") => LayerOpcodeKind::From,
        Some("RUN") => LayerOpcodeKind::Run,
        Some("COPY" | "ADD") => LayerOpcodeKind::Copy,
        Some("CMD" | "ENTRYPOINT") => LayerOpcodeKind::Cmd,
        Some(
            "ENV" | "LABEL" | "WORKDIR" | "USER" | "EXPOSE" | "VOLUME" | "ARG" | "SHELL"
            | "STOPSIGNAL" | "HEALTHCHECK" | "ONBUILD" | "MAINTAINER",
        ) => LayerOpcodeKind::Meta,
        _ if had_nop => LayerOpcodeKind::Meta,
        _ if had_sh => LayerOpcodeKind::Run,
        _ => LayerOpcodeKind::Run,
    };

    let is_shell_run = opcode == LayerOpcodeKind::Run && !is_exec_json;
    let body_lines = if is_shell_run {
        let formatted = format_shell_body(work);
        if formatted.is_empty() {
            vec![cleaned_instr.clone()]
        } else {
            formatted
        }
    } else if matches!(explicit_op, Some("ENV" | "LABEL")) && cleaned_instr.len() > LINE_WRAP_LIMIT
    {
        wrap_key_value_instruction(explicit_op.unwrap_or("ENV"), work)
    } else {
        vec![cleaned_instr.clone()]
    };

    let summary = if is_shell_run {
        extract_run_summary(&body_lines).unwrap_or_else(|| cleaned_instr.clone())
    } else {
        cleaned_instr
    };

    let mut formatted_lines = Vec::with_capacity(build_args.len() + body_lines.len());
    for arg in &build_args {
        formatted_lines.push(format!("# ARG {arg}"));
    }
    formatted_lines.extend(body_lines);

    ParsedLayerCommand {
        opcode,
        summary,
        build_args,
        formatted_lines,
    }
}

/// Auto-formats an entire reconstructed Dockerfile with clean indentation and `\` continuations.
pub fn format_dockerfile(dockerfile: &str) -> String {
    if dockerfile.trim().is_empty() {
        return String::new();
    }

    struct Block {
        text: String,
        multi_line: bool,
    }

    let mut blocks: Vec<Block> = Vec::new();
    for raw_line in dockerfile.lines() {
        let cleaned = strip_buildkit_comment_suffix(raw_line.trim());
        if cleaned.is_empty() {
            continue;
        }
        let parsed = parse_layer_command(cleaned);
        let is_shell_run = parsed.opcode == LayerOpcodeKind::Run
            && !strip_ascii_prefix_ci(cleaned.trim_start(), "RUN ")
                .map(|r| r.trim_start().starts_with('['))
                .unwrap_or(false)
            && !cleaned.trim_start().starts_with('[');

        if is_shell_run {
            let cmd_lines: Vec<&str> = parsed
                .formatted_lines
                .iter()
                .filter(|line| !line.starts_with("# ARG "))
                .map(String::as_str)
                .collect();
            let mut block_lines = Vec::with_capacity(parsed.build_args.len() + cmd_lines.len());
            for arg in &parsed.build_args {
                block_lines.push(format!("ARG {arg}"));
            }
            if cmd_lines.is_empty() {
                block_lines.push(format!("RUN {}", parsed.summary));
            } else if cmd_lines.len() == 1 {
                block_lines.push(format!("RUN {}", cmd_lines[0]));
            } else {
                block_lines.push(format!("RUN {} \\", cmd_lines[0].trim_end_matches(" \\")));
                for line in &cmd_lines[1..cmd_lines.len() - 1] {
                    block_lines.push(format!("{INDENT}{} \\", line.trim_end_matches(" \\")));
                }
                if let Some(last) = cmd_lines.last() {
                    block_lines.push(format!("{INDENT}{}", last.trim_end_matches(" \\")));
                }
            }
            let multi_line = block_lines.len() > 1;
            blocks.push(Block {
                text: block_lines.join("\n"),
                multi_line,
            });
        } else if parsed.formatted_lines.len() > 1 {
            blocks.push(Block {
                text: parsed.formatted_lines.join("\n"),
                multi_line: true,
            });
        } else {
            blocks.push(Block {
                text: parsed
                    .formatted_lines
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
                multi_line: false,
            });
        }
    }

    let mut out = String::new();
    for (idx, block) in blocks.iter().enumerate() {
        if idx > 0 {
            if blocks[idx - 1].multi_line || block.multi_line {
                out.push_str("\n\n");
            } else {
                out.push('\n');
            }
        }
        out.push_str(&block.text);
    }
    out
}

fn format_shell_body(body: &str) -> Vec<String> {
    let raw_segments = split_shell_segments(body);
    let segments = normalize_control_flow_segments(raw_segments);
    if segments.is_empty() {
        return Vec::new();
    }

    let mut raw_lines: Vec<String> = Vec::new();
    let mut indent_level = 0usize;

    for seg in &segments {
        let first_kw = first_word(&seg.code);
        if matches!(first_kw, "elif" | "else" | "fi" | "done" | "esac") {
            indent_level = indent_level.saturating_sub(1);
        }

        let indent = INDENT.repeat(indent_level);
        let suffix = seg.op.as_suffix();
        let line_with_op = format!("{}{}", seg.code, suffix);
        let is_control_head =
            matches!(first_kw, "case" | "if" | "elif" | "while" | "until" | "for");

        if indent.len() + line_with_op.len() > LINE_WRAP_LIMIT
            && seg.op != ShellOp::DoubleSemi
            && !is_control_head
        {
            let tokens = tokenize_shell_words(&seg.code);
            let has_pipe_or_redir = tokens.iter().any(|tok| is_pipe_or_redirect(tok));
            if tokens.len() >= 4 && !has_pipe_or_redir {
                let mut cur = format!("{indent}{} {}", tokens[0], tokens[1]);
                for tok in &tokens[2..] {
                    if cur.len() + 1 + tok.len() <= TOKEN_CONT_LIMIT {
                        cur.push(' ');
                        cur.push_str(tok);
                    } else {
                        raw_lines.push(cur);
                        cur = format!("{indent}{INDENT}{tok}");
                    }
                }
                raw_lines.push(format!("{cur}{suffix}"));
            } else {
                raw_lines.push(format!("{indent}{line_with_op}"));
            }
        } else {
            raw_lines.push(format!("{indent}{line_with_op}"));
        }

        if opens_block(&seg.code) {
            indent_level += 1;
        }
    }

    // Add valid trailing ` \` continuations on every line except the final one.
    let mut lines: Vec<String> = Vec::with_capacity(raw_lines.len());
    let total = raw_lines.len();
    for (idx, line) in raw_lines.into_iter().enumerate() {
        for sub in line.split('\n') {
            lines.push(sub.to_owned());
        }
        if idx + 1 < total
            && let Some(last) = lines.last_mut()
        {
            last.push_str(" \\");
        }
    }
    lines
}

fn extract_run_summary(lines: &[String]) -> Option<String> {
    for line in lines {
        let stripped = line.trim().trim_end_matches('\\').trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let clean = stripped
            .trim_end_matches("&&")
            .trim_end_matches("||")
            .trim_end_matches(';')
            .trim();
        if matches!(
            clean,
            "set -e" | "set -ex" | "set -eux" | "set -eu" | "set -euo pipefail"
        ) {
            continue;
        }
        return Some(collapse_spaces(clean));
    }
    lines
        .first()
        .map(|line| collapse_spaces(line.trim().trim_end_matches('\\').trim()))
}

fn split_shell_segments(body: &str) -> Vec<ShellSegment> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut segments: Vec<ShellSegment> = Vec::new();
    let mut stack = vec![LexFrame::Normal];
    let mut root_paren_depth = 0usize;
    let mut current = String::new();

    let push_segment = |op: ShellOp, current: &mut String, segments: &mut Vec<ShellSegment>| {
        let code = current.trim().to_owned();
        if !code.is_empty() {
            segments.push(ShellSegment { code, op });
        } else if op != ShellOp::None
            && let Some(prev) = segments.last_mut()
            && prev.op == ShellOp::None
        {
            prev.op = op;
        }
        current.clear();
    };

    let mut idx = 0usize;
    while idx < len {
        let ch = trimmed[idx..].chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();
        let frame = *stack.last().unwrap_or(&LexFrame::Normal);

        match frame {
            LexFrame::Single => {
                if ch == '\r' || ch == '\n' {
                    if !current.ends_with(' ') {
                        current.push(' ');
                    }
                } else {
                    current.push(ch);
                    if ch == '\'' {
                        stack.pop();
                    }
                }
                idx += ch_len;
                continue;
            }
            LexFrame::Double => {
                if ch == '\\' && idx + 1 < len {
                    let next = trimmed[idx + 1..].chars().next().unwrap_or('\0');
                    if next == '\n' {
                        idx += 2;
                        continue;
                    }
                    if next == '\r' && bytes.get(idx + 2) == Some(&b'\n') {
                        idx += 3;
                        continue;
                    }
                    current.push('\\');
                    current.push(next);
                    idx += 1 + next.len_utf8();
                    continue;
                }
                if ch == '$' && bytes.get(idx + 1) == Some(&b'(') {
                    current.push_str("$(");
                    stack.push(LexFrame::CmdSubst { paren_depth: 0 });
                    idx += 2;
                    continue;
                }
                if ch == '`' {
                    current.push('`');
                    stack.push(LexFrame::Backtick);
                    idx += 1;
                    continue;
                }
                if ch == '\r' || ch == '\n' {
                    if !current.ends_with(' ') {
                        current.push(' ');
                    }
                } else {
                    current.push(ch);
                    if ch == '"' {
                        stack.pop();
                    }
                }
                idx += ch_len;
                continue;
            }
            LexFrame::Backtick => {
                if ch == '\\' && idx + 1 < len {
                    let next = trimmed[idx + 1..].chars().next().unwrap_or('\0');
                    current.push('\\');
                    current.push(next);
                    idx += 1 + next.len_utf8();
                    continue;
                }
                if ch == '\r' || ch == '\n' {
                    if !current.ends_with(' ') {
                        current.push(' ');
                    }
                } else {
                    current.push(ch);
                    if ch == '`' {
                        stack.pop();
                    }
                }
                idx += ch_len;
                continue;
            }
            LexFrame::Normal | LexFrame::CmdSubst { .. } => {}
        }

        if ch == '\\' && idx + 1 < len {
            let next = trimmed[idx + 1..].chars().next().unwrap_or('\0');
            if next == '\n' {
                idx += 2;
                continue;
            }
            if next == '\r' && bytes.get(idx + 2) == Some(&b'\n') {
                idx += 3;
                continue;
            }
            current.push('\\');
            current.push(next);
            idx += 1 + next.len_utf8();
            continue;
        }

        match ch {
            '\'' => {
                current.push('\'');
                stack.push(LexFrame::Single);
                idx += 1;
            }
            '"' => {
                current.push('"');
                stack.push(LexFrame::Double);
                idx += 1;
            }
            '`' => {
                current.push('`');
                stack.push(LexFrame::Backtick);
                idx += 1;
            }
            '$' if bytes.get(idx + 1) == Some(&b'(') => {
                current.push_str("$(");
                stack.push(LexFrame::CmdSubst { paren_depth: 0 });
                idx += 2;
            }
            '(' => {
                if let Some(LexFrame::CmdSubst { paren_depth }) = stack.last_mut() {
                    *paren_depth += 1;
                } else if stack.len() == 1 {
                    root_paren_depth += 1;
                }
                current.push('(');
                idx += 1;
            }
            ')' => {
                if let Some(LexFrame::CmdSubst { paren_depth }) = stack.last_mut() {
                    if *paren_depth == 0 {
                        stack.pop();
                    } else {
                        *paren_depth -= 1;
                    }
                } else if stack.len() == 1 {
                    root_paren_depth = root_paren_depth.saturating_sub(1);
                }
                current.push(')');
                idx += 1;
            }
            ' ' | '\t' | '\r' | '\n' => {
                if !current.is_empty() && !current.ends_with(' ') {
                    current.push(' ');
                }
                idx += ch_len;
            }
            ';' if stack.len() == 1 && root_paren_depth == 0 => {
                if bytes.get(idx + 1) == Some(&b';') {
                    push_segment(ShellOp::DoubleSemi, &mut current, &mut segments);
                    idx += 2;
                } else {
                    push_segment(ShellOp::Semi, &mut current, &mut segments);
                    idx += 1;
                }
            }
            '&' if stack.len() == 1
                && root_paren_depth == 0
                && bytes.get(idx + 1) == Some(&b'&') =>
            {
                push_segment(ShellOp::And, &mut current, &mut segments);
                idx += 2;
            }
            '|' if stack.len() == 1
                && root_paren_depth == 0
                && bytes.get(idx + 1) == Some(&b'|') =>
            {
                push_segment(ShellOp::Or, &mut current, &mut segments);
                idx += 2;
            }
            _ => {
                current.push(ch);
                idx += ch_len;
            }
        }
    }

    push_segment(ShellOp::None, &mut current, &mut segments);
    segments
}

fn normalize_control_flow_segments(mut segs: Vec<ShellSegment>) -> Vec<ShellSegment> {
    let mut out: Vec<ShellSegment> = Vec::with_capacity(segs.len());
    let mut i = 0usize;

    while i < segs.len() {
        let code = segs[i].code.clone();
        let op = segs[i].op;

        if let Some((header, rest)) = split_case_header(&code) {
            out.push(ShellSegment {
                code: header,
                op: ShellOp::None,
            });
            segs[i] = ShellSegment { code: rest, op };
            continue;
        }

        let head = first_word(&code);
        if matches!(head, "if" | "elif" | "while" | "until" | "for")
            && op == ShellOp::Semi
            && i + 1 < segs.len()
        {
            let next_code = segs[i + 1].code.trim().to_owned();
            let next_op = segs[i + 1].op;
            if let Some((kw, remainder)) = split_leading_block_keyword(&next_code) {
                out.push(ShellSegment {
                    code: format!("{code}; {kw}"),
                    op: ShellOp::None,
                });
                if remainder.is_empty() {
                    segs.remove(i + 1);
                } else {
                    segs[i + 1] = ShellSegment {
                        code: remainder,
                        op: next_op,
                    };
                }
                i += 1;
                continue;
            }
        }

        if let Some(rest) = code.strip_prefix("else ") {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                out.push(ShellSegment {
                    code: "else".to_owned(),
                    op: ShellOp::None,
                });
                segs[i] = ShellSegment {
                    code: trimmed.to_owned(),
                    op,
                };
                continue;
            }
        }

        out.push(ShellSegment { code, op });
        i += 1;
    }

    if let Some(last) = out.last_mut()
        && last.op == ShellOp::Semi
    {
        last.op = ShellOp::None;
    }
    out
}

fn split_case_header(code: &str) -> Option<(String, String)> {
    let trimmed = code.trim_start();
    if !trimmed.starts_with("case ") && !trimmed.starts_with("case\t") {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut stack = vec![LexFrame::Normal];
    let mut idx = 0usize;

    while idx < len {
        let ch = trimmed[idx..].chars().next().unwrap_or('\0');
        let frame = *stack.last().unwrap_or(&LexFrame::Normal);
        match frame {
            LexFrame::Single => {
                if ch == '\'' {
                    stack.pop();
                }
                idx += ch.len_utf8();
                continue;
            }
            LexFrame::Double => {
                if ch == '\\' && idx + 1 < len {
                    let next = trimmed[idx + 1..].chars().next().unwrap_or('\0');
                    idx += 1 + next.len_utf8();
                    continue;
                }
                if ch == '$' && bytes.get(idx + 1) == Some(&b'(') {
                    stack.push(LexFrame::CmdSubst { paren_depth: 0 });
                    idx += 2;
                    continue;
                }
                if ch == '`' {
                    stack.push(LexFrame::Backtick);
                    idx += 1;
                    continue;
                }
                if ch == '"' {
                    stack.pop();
                }
                idx += ch.len_utf8();
                continue;
            }
            LexFrame::Backtick => {
                if ch == '\\' && idx + 1 < len {
                    let next = trimmed[idx + 1..].chars().next().unwrap_or('\0');
                    idx += 1 + next.len_utf8();
                    continue;
                }
                if ch == '`' {
                    stack.pop();
                }
                idx += ch.len_utf8();
                continue;
            }
            LexFrame::Normal | LexFrame::CmdSubst { .. } => {}
        }

        if ch == '\\' && idx + 1 < len {
            let next = trimmed[idx + 1..].chars().next().unwrap_or('\0');
            idx += 1 + next.len_utf8();
            continue;
        }
        match ch {
            '\'' => stack.push(LexFrame::Single),
            '"' => stack.push(LexFrame::Double),
            '`' => stack.push(LexFrame::Backtick),
            '$' if bytes.get(idx + 1) == Some(&b'(') => {
                stack.push(LexFrame::CmdSubst { paren_depth: 0 });
                idx += 2;
                continue;
            }
            '(' => {
                if let Some(LexFrame::CmdSubst { paren_depth }) = stack.last_mut() {
                    *paren_depth += 1;
                }
            }
            ')' => {
                if let Some(LexFrame::CmdSubst { paren_depth }) = stack.last_mut() {
                    if *paren_depth == 0 {
                        stack.pop();
                    } else {
                        *paren_depth -= 1;
                    }
                }
            }
            ' ' if stack.len() == 1 && trimmed[idx..].starts_with(" in ") => {
                let header = trimmed[..idx + 3].trim().to_owned();
                let rest = trimmed[idx + 4..].trim().to_owned();
                if !rest.is_empty() {
                    return Some((header, rest));
                }
                return None;
            }
            _ => {}
        }
        idx += ch.len_utf8();
    }
    None
}

fn split_leading_block_keyword(code: &str) -> Option<(&'static str, String)> {
    for kw in ["then", "do"] {
        if code == kw {
            return Some((kw, String::new()));
        }
        if let Some(rest) = code.strip_prefix(kw)
            && rest.starts_with(char::is_whitespace)
        {
            return Some((kw, rest.trim().to_owned()));
        }
    }
    None
}

fn opens_block(code: &str) -> bool {
    let trimmed = code.trim();
    if trimmed == "else" {
        return true;
    }
    if let Some((prefix, last)) = trimmed.rsplit_once(';')
        && matches!(last.trim(), "then" | "do")
        && matches!(
            first_word(prefix.trim()),
            "if" | "elif" | "while" | "until" | "for"
        )
    {
        return true;
    }
    first_word(trimmed) == "case"
        && trimmed
            .split_whitespace()
            .last()
            .is_some_and(|tok| tok == "in")
}

fn tokenize_shell_words(code: &str) -> Vec<String> {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut tokens: Vec<String> = Vec::new();
    let mut stack = vec![LexFrame::Normal];
    let mut current = String::new();
    let mut idx = 0usize;

    while idx < len {
        let ch = code[idx..].chars().next().unwrap_or('\0');
        let frame = *stack.last().unwrap_or(&LexFrame::Normal);

        match frame {
            LexFrame::Single => {
                current.push(ch);
                if ch == '\'' {
                    stack.pop();
                }
                idx += ch.len_utf8();
                continue;
            }
            LexFrame::Double => {
                if ch == '\\' && idx + 1 < len {
                    let next = code[idx + 1..].chars().next().unwrap_or('\0');
                    current.push('\\');
                    current.push(next);
                    idx += 1 + next.len_utf8();
                    continue;
                }
                if ch == '$' && bytes.get(idx + 1) == Some(&b'(') {
                    current.push_str("$(");
                    stack.push(LexFrame::CmdSubst { paren_depth: 0 });
                    idx += 2;
                    continue;
                }
                if ch == '`' {
                    current.push('`');
                    stack.push(LexFrame::Backtick);
                    idx += 1;
                    continue;
                }
                current.push(ch);
                if ch == '"' {
                    stack.pop();
                }
                idx += ch.len_utf8();
                continue;
            }
            LexFrame::Backtick => {
                if ch == '\\' && idx + 1 < len {
                    let next = code[idx + 1..].chars().next().unwrap_or('\0');
                    current.push('\\');
                    current.push(next);
                    idx += 1 + next.len_utf8();
                    continue;
                }
                current.push(ch);
                if ch == '`' {
                    stack.pop();
                }
                idx += ch.len_utf8();
                continue;
            }
            LexFrame::Normal | LexFrame::CmdSubst { .. } => {}
        }

        if ch == '\\' && idx + 1 < len {
            let next = code[idx + 1..].chars().next().unwrap_or('\0');
            current.push('\\');
            current.push(next);
            idx += 1 + next.len_utf8();
            continue;
        }
        match ch {
            '\'' => {
                current.push('\'');
                stack.push(LexFrame::Single);
            }
            '"' => {
                current.push('"');
                stack.push(LexFrame::Double);
            }
            '`' => {
                current.push('`');
                stack.push(LexFrame::Backtick);
            }
            '$' if bytes.get(idx + 1) == Some(&b'(') => {
                current.push_str("$(");
                stack.push(LexFrame::CmdSubst { paren_depth: 0 });
                idx += 2;
                continue;
            }
            '(' => {
                if let Some(LexFrame::CmdSubst { paren_depth }) = stack.last_mut() {
                    *paren_depth += 1;
                }
                current.push('(');
            }
            ')' => {
                if let Some(LexFrame::CmdSubst { paren_depth }) = stack.last_mut() {
                    if *paren_depth == 0 {
                        stack.pop();
                    } else {
                        *paren_depth -= 1;
                    }
                }
                current.push(')');
            }
            ' ' | '\t' if stack.len() == 1 => {
                if !current.is_empty() {
                    tokens.push(take(&mut current));
                }
            }
            _ => current.push(ch),
        }
        idx += ch.len_utf8();
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_pipe_or_redirect(token: &str) -> bool {
    matches!(
        token,
        "|" | ">" | ">>" | "<" | ">&2" | "2>&1" | "1>&2" | "&>"
    )
}

fn wrap_key_value_instruction(op: &str, body: &str) -> Vec<String> {
    let words = tokenize_shell_words(body);
    let pairs: Vec<String> = words.into_iter().filter(|w| w != "\\").collect();
    if pairs.len() <= 1 {
        return vec![format!("{op} {}", collapse_spaces(body))];
    }
    let mut lines = Vec::with_capacity(pairs.len());
    for (idx, pair) in pairs.iter().enumerate() {
        let prefix = if idx == 0 {
            format!("{op} ")
        } else {
            INDENT.to_owned()
        };
        let suffix = if idx + 1 < pairs.len() { " \\" } else { "" };
        lines.push(format!("{prefix}{pair}{suffix}"));
    }
    lines
}

fn clean_expose_line(line: &str) -> String {
    let Some(rest) = strip_ascii_prefix_ci(line.trim(), "EXPOSE") else {
        return collapse_spaces(line);
    };
    let rest = rest.trim();
    if let Some(inner) = rest.strip_prefix("map[").and_then(|s| s.strip_suffix(']')) {
        let ports: Vec<&str> = inner
            .split_whitespace()
            .filter_map(|entry| entry.split(':').next())
            .filter(|port| !port.is_empty())
            .collect();
        if !ports.is_empty() {
            return format!("EXPOSE {}", ports.join(" "));
        }
    }
    format!("EXPOSE {}", collapse_spaces(rest))
}

fn unwrap_buildkit_pipe_shell(input: &str) -> Option<(Vec<String>, &str)> {
    let after_pipe = input.strip_prefix('|')?;
    let digits_end = after_pipe
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits_end == 0 {
        return None;
    }
    let arg_count: usize = after_pipe[..digits_end].parse().ok()?;
    let after_count = &after_pipe[digits_end..];
    if !after_count.starts_with(char::is_whitespace) {
        return None;
    }

    let mut shell_offset: Option<(usize, usize)> = None;
    for marker in SHELL_MARKERS {
        if let Some(pos) = after_count.find(marker)
            && shell_offset.is_none_or(|(best, _)| pos < best)
        {
            shell_offset = Some((pos, marker.len()));
        }
    }
    let (marker_pos, marker_len) = shell_offset?;
    let arg_blob = after_count[..marker_pos].trim();
    let body = after_count[marker_pos + marker_len..].trim();
    let args = if arg_count > 0 && !arg_blob.is_empty() {
        split_buildkit_env_args(arg_blob)
    } else {
        Vec::new()
    };
    Some((args, body))
}

fn split_buildkit_env_args(arg_blob: &str) -> Vec<String> {
    let bytes = arg_blob.as_bytes();
    let len = bytes.len();
    let mut args = Vec::new();
    let mut stack = vec![LexFrame::Normal];
    let mut start = 0usize;
    let mut idx = 0usize;

    while idx < len {
        let ch = arg_blob[idx..].chars().next().unwrap_or('\0');
        let frame = *stack.last().unwrap_or(&LexFrame::Normal);
        match frame {
            LexFrame::Single => {
                if ch == '\'' {
                    stack.pop();
                }
            }
            LexFrame::Double => {
                if ch == '\\' && idx + 1 < len {
                    let next = arg_blob[idx + 1..].chars().next().unwrap_or('\0');
                    idx += 1 + next.len_utf8();
                    continue;
                }
                if ch == '"' {
                    stack.pop();
                }
            }
            LexFrame::Backtick | LexFrame::CmdSubst { .. } => {}
            LexFrame::Normal => {
                if ch == '\\' && idx + 1 < len {
                    let next = arg_blob[idx + 1..].chars().next().unwrap_or('\0');
                    idx += 1 + next.len_utf8();
                    continue;
                }
                if ch == '\'' {
                    stack.push(LexFrame::Single);
                } else if ch == '"' {
                    stack.push(LexFrame::Double);
                } else if ch.is_ascii_whitespace() {
                    let after_ws = arg_blob[idx..].trim_start();
                    if starts_with_env_assignment(after_ws) {
                        let piece = arg_blob[start..idx].trim();
                        if !piece.is_empty() {
                            args.push(piece.to_owned());
                        }
                        start = len - after_ws.len();
                        idx = start;
                        continue;
                    }
                }
            }
        }
        idx += ch.len_utf8();
    }

    let tail = arg_blob[start..].trim();
    if !tail.is_empty() {
        args.push(tail.to_owned());
    }
    args
}

fn starts_with_env_assignment(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    for ch in chars {
        if ch == '=' {
            return true;
        }
        if !(ch.is_ascii_alphanumeric() || ch == '_') {
            return false;
        }
    }
    false
}

fn take_instruction_keyword(input: &str) -> Option<(&'static str, &str)> {
    const KEYWORDS: [&str; 18] = [
        "RUN",
        "COPY",
        "ADD",
        "ENV",
        "ARG",
        "WORKDIR",
        "EXPOSE",
        "CMD",
        "ENTRYPOINT",
        "USER",
        "LABEL",
        "VOLUME",
        "FROM",
        "SHELL",
        "STOPSIGNAL",
        "HEALTHCHECK",
        "ONBUILD",
        "MAINTAINER",
    ];
    for kw in KEYWORDS {
        let Some(after) = strip_ascii_prefix_ci(input, kw) else {
            continue;
        };
        if after.is_empty() || after.starts_with(char::is_whitespace) {
            return Some((kw, after.trim_start()));
        }
    }
    None
}

fn strip_ascii_prefix_ci<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    input
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &input[prefix.len()..])
}

fn strip_buildkit_comment_suffix(s: &str) -> &str {
    let trimmed = s.trim_end();
    let suffix = "# buildkit";
    if trimmed.len() >= suffix.len() {
        let pos = trimmed.len() - suffix.len();
        if trimmed.is_char_boundary(pos)
            && trimmed[pos..].eq_ignore_ascii_case(suffix)
            && (pos == 0 || trimmed[..pos].ends_with(char::is_whitespace))
        {
            return trimmed[..pos].trim_end();
        }
    }
    trimmed
}

fn strip_shell_c_prefix(s: &str) -> Option<&str> {
    for prefix in SHELL_PREFIXES {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    None
}

fn strip_nop_shell_prefix(s: &str) -> Option<&str> {
    let body = strip_shell_c_prefix(s)?;
    strip_nop_prefix(body)
}

fn has_nop_shell_prefix(s: &str) -> bool {
    strip_nop_shell_prefix(s).is_some()
}

fn strip_nop_prefix(s: &str) -> Option<&str> {
    s.trim_start().strip_prefix("#(nop)").map(str::trim_start)
}

fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or_default()
}

fn collapse_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_redis_buildkit_command_preserving_quotes_and_extracting_args() {
        let raw = r#"RUN |2 REDIS_DOWNLOAD_URL=https://github.com/redis/redis/releases/download/8.12.0/redis-full-8.12.0.tar.gz REDIS_DOWNLOAD_SHA=e9d9fb3a6e62960f06b42014c88c4b14b37544cf30a4e3230f3d7cf7f9b9d123 /bin/sh -c set -eux; sed -ri 's!cd jemalloc && ./configure !&'"$extraJemallocConfigureFlags"' !' /usr/src/redis/deps/Makefile; grep -F "cd jemalloc && ./configure $extraJemallocConfigureFlags " /usr/src/redis/deps/Makefile; export BUILD_TLS=yes; export BUILD_COMPRESSION=yes; if [ "$BUILD_WITH_MODULES" = "yes" ]; then make -j "$(nproc)" all; fi # buildkit"#;

        let parsed = parse_layer_command(raw);
        assert_eq!(parsed.opcode, LayerOpcodeKind::Run);
        assert_eq!(
            parsed.build_args,
            vec![
                "REDIS_DOWNLOAD_URL=https://github.com/redis/redis/releases/download/8.12.0/redis-full-8.12.0.tar.gz",
                "REDIS_DOWNLOAD_SHA=e9d9fb3a6e62960f06b42014c88c4b14b37544cf30a4e3230f3d7cf7f9b9d123",
            ]
        );
        assert!(parsed.formatted_lines[0].starts_with("# ARG REDIS_DOWNLOAD_URL="));
        assert!(parsed.formatted_lines[1].starts_with("# ARG REDIS_DOWNLOAD_SHA="));

        // Quotes around `cd jemalloc && ./configure` must never fracture inside `'...'` or `"..."`.
        let joined = parsed.formatted_lines.join("\n");
        assert!(
            joined.contains(
                r#"sed -ri 's!cd jemalloc && ./configure !&'"$extraJemallocConfigureFlags"' !'"#
            ),
            "single-quoted regex must remain intact:\n{joined}"
        );
        assert!(
            joined
                .contains(r#"grep -F "cd jemalloc && ./configure $extraJemallocConfigureFlags ""#),
            "double-quoted pattern must remain intact:\n{joined}"
        );
        assert!(joined.contains("export BUILD_TLS=yes; \\"));
        assert!(joined.contains("export BUILD_COMPRESSION=yes; \\"));
        assert!(joined.contains("if [ \"$BUILD_WITH_MODULES\" = \"yes\" ]; then \\"));
        assert!(joined.contains("    make -j \"$(nproc)\" all; \\"));
        assert!(joined.ends_with("fi"));
        // Boilerplate `set -eux;` is skipped in the single-line timeline summary.
        assert!(parsed.summary.starts_with("sed -ri"));
    }

    #[test]
    fn preserves_subshell_and_control_flow_blocks() {
        let raw =
            "/bin/sh -c (cd /tmp && tar -xzf src.tgz) && for f in a b; do rm -rf \"$f\"; done";
        let parsed = parse_layer_command(raw);
        assert_eq!(parsed.opcode, LayerOpcodeKind::Run);
        assert_eq!(
            parsed.formatted_lines,
            vec![
                "(cd /tmp && tar -xzf src.tgz) && \\",
                "for f in a b; do \\",
                "    rm -rf \"$f\"; \\",
                "done",
            ]
        );
    }

    #[test]
    fn wraps_long_package_install_command_on_word_boundaries() {
        let raw = "RUN apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates curl git libssl-dev pkg-config && rm -rf /var/lib/apt/lists/*";
        let parsed = parse_layer_command(raw);
        assert_eq!(parsed.opcode, LayerOpcodeKind::Run);
        assert!(parsed.formatted_lines.len() >= 4);
        assert_eq!(parsed.formatted_lines[0], "apt-get update && \\");
        assert!(parsed.formatted_lines.iter().any(|l| l.starts_with("    ")));
        assert_eq!(
            parsed.formatted_lines.last().map(String::as_str),
            Some("rm -rf /var/lib/apt/lists/*")
        );
    }

    #[test]
    fn cleans_expose_map_and_exec_forms_and_formats_dockerfile() {
        assert_eq!(
            parse_layer_command("EXPOSE map[6379/tcp:{} 8080/tcp:{}]").summary,
            "EXPOSE 6379/tcp 8080/tcp"
        );
        assert_eq!(
            parse_layer_command("CMD [\"nginx\", \"-g\", \"daemon off;\"]").summary,
            "CMD [\"nginx\", \"-g\", \"daemon off;\"]"
        );

        let df = "FROM alpine:3.20 # buildkit\nRUN |1 VER=1.2 /bin/sh -c apk add --no-cache ca-certificates && echo ok\nEXPOSE map[8080/tcp:{}]";
        let formatted = format_dockerfile(df);
        assert_eq!(
            formatted,
            "FROM alpine:3.20\n\nARG VER=1.2\nRUN apk add --no-cache ca-certificates && \\\n    echo ok\n\nEXPOSE 8080/tcp"
        );
    }
}
