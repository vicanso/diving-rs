//! Rich file preview inspection shared by Diving Desktop and Diving Web.
//!
//! Identifies raster images, SVG vector graphics, gzip/zstd compressed text
//! assets, nested TAR/ZIP/JAR/APK archives, ELF/Wasm/SQLite/Mach-O/Java class
//! binaries, fonts, PDFs, audio/video media headers, and UTF-8 / formatted JSON
//! without pulling extra parsing dependencies into the workspace.

use std::cmp::Reverse;
use std::io::{Cursor, Read};
use std::str::from_utf8;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bytesize::ByteSize;
use flate2::read::MultiGzDecoder;
use goblin::elf::header::{
    class_to_str, et_to_str, EI_CLASS, EI_DATA, EI_OSABI, ELFDATA2LSB, ELFDATA2MSB, ELFOSABI_ARM,
    ELFOSABI_ARM_AEABI, ELFOSABI_FREEBSD, ELFOSABI_GNU, ELFOSABI_NONE, ELFOSABI_OPENBSD,
    ELFOSABI_SOLARIS, EM_386, EM_AARCH64, EM_ARM, EM_MIPS, EM_PPC64, EM_RISCV, EM_S390, EM_X86_64,
};
use goblin::elf::section_header::{SHT_NOBITS, SHT_SYMTAB};
use goblin::elf::Elf;
use serde::{Deserialize, Serialize};
use serde_json::{from_str as json_from_str, to_string_pretty, Value as JsonValue};
use tar::Archive;
use unix_mode::to_string as mode_to_string;
use zstd::Decoder as ZstdDecoder;

/// Maximum bytes read for plain-text and source files during inline preview.
pub const PREVIEW_TEXT_LIMIT: u64 = 256 * 1024;
/// Maximum bytes read for rich binary, image, media, and archive inspection.
pub const PREVIEW_RICH_LIMIT: u64 = 16 * 1024 * 1024;
/// Maximum bytes decompressed when previewing `.gz` / `.zst` single files.
pub const PREVIEW_DECOMPRESSED_TEXT_LIMIT: usize = 256 * 1024;
/// Maximum text bytes eligible for lexical JS/CSS/markup beautification.
const MAX_BEAUTIFY_CODE_BYTES: usize = 512 * 1024;
/// Maximum archive entries returned by inline TAR/ZIP inspection.
pub const PREVIEW_ARCHIVE_MAX_ENTRIES: usize = 500;
/// Maximum compressed/tar stream bytes scanned during nested archive listing.
pub const PREVIEW_ARCHIVE_SCAN_BYTES: u64 = 64 * 1024 * 1024;
/// EM_LOONGARCH machine id (kept explicit so older constants do not break builds).
const ELF_MACHINE_LOONGARCH: u16 = 258;
/// Zip Central Directory and Local File Header signatures.
const ZIP_EOCD_SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const ZIP_CD_SIG: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const ZIP_LFH_SIG: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
/// Minimum bytes needed before parsing an ELF header (keeps tiny test fixtures as Binary).
const MIN_ELF_HEADER_BYTES: usize = 52;

/// Visual metadata extracted from an image header or SVG root element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreviewMeta {
    pub format: String,
    pub mime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Metadata describing an audio or video container detected by magic header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPreviewMeta {
    pub media_type: String,
    pub format: String,
    pub mime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One entry inside a nested archive file (`.tar`, `.tar.gz`, `.zip`, `.jar`, `.apk`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntryPreview {
    pub path: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compressed_size: Option<u64>,
    pub mode: String,
    pub is_dir: bool,
}

/// Archive directory listing built without writing extracted files to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePreviewMeta {
    pub format: String,
    pub total_entries: usize,
    pub total_uncompressed_size: u64,
    pub truncated_entries: bool,
    pub entries: Vec<ArchiveEntryPreview>,
}

/// Key/value metadata row inside a structured binary inspection card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewField {
    pub label: String,
    pub value: String,
}

/// Named list section inside a structured binary inspection card (e.g. `NEEDED` libraries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredSection {
    pub title: String,
    pub items: Vec<String>,
}

/// Structured summary of an ELF, Wasm, SQLite, Java class, Mach-O, font, or PDF file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredPreviewMeta {
    pub format: String,
    pub summary: String,
    pub fields: Vec<PreviewField>,
    pub sections: Vec<StructuredSection>,
}

/// Outcome of classifying raw file bytes from a container layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectedPreview {
    Empty,
    Image {
        meta: ImagePreviewMeta,
        truncated: bool,
    },
    Svg {
        meta: ImagePreviewMeta,
        source: String,
        formatted_source: Option<String>,
        truncated: bool,
    },
    Media {
        meta: MediaPreviewMeta,
        truncated: bool,
    },
    Archive {
        meta: ArchivePreviewMeta,
    },
    Structured {
        meta: StructuredPreviewMeta,
    },
    Text {
        text: String,
        formatted_text: Option<String>,
        language: Option<&'static str>,
        compression: Option<&'static str>,
        shown_bytes: u64,
        total_size: u64,
        truncated: bool,
    },
    Binary {
        shown_bytes: u64,
        total_size: u64,
        truncated: bool,
    },
}

/// Chooses the byte budget for reading a file prior to preview classification.
pub fn preview_read_limit(file_path: &str) -> u64 {
    let base = file_path.rsplit('/').next().unwrap_or(file_path);
    let lower = base.to_ascii_lowercase();
    let trimmed = lower.strip_prefix('.').unwrap_or(&lower);
    let ext = trimmed.rsplit_once('.').map(|(_, ext)| ext);

    if matches!(ext, Some("gz" | "zst" | "svg" | "svgz")) {
        return PREVIEW_RICH_LIMIT;
    }

    if detect_code_language(file_path).is_some()
        || matches!(
            ext,
            Some(
                "txt"
                    | "log"
                    | "out"
                    | "csv"
                    | "tsv"
                    | "rst"
                    | "adoc"
                    | "asciidoc"
                    | "mdx"
                    | "tex"
                    | "pem"
                    | "pub"
                    | "asc"
                    | "csr"
                    | "man"
                    | "lic"
            )
        )
        || matches!(
            lower.as_str(),
            "license"
                | "licence"
                | "copying"
                | "notice"
                | "readme"
                | "changelog"
                | "changes"
                | "authors"
                | "contributors"
                | "3rdpartylicenses.txt"
                | ".gitignore"
                | ".gitattributes"
                | ".gitmodules"
                | ".dockerignore"
                | ".editorconfig"
                | ".npmrc"
                | ".yarnrc"
                | ".nvmrc"
                | ".browserslistrc"
        )
    {
        return PREVIEW_TEXT_LIMIT;
    }

    PREVIEW_RICH_LIMIT
}

/// Maps filenames and extensions to syntax highlighter language identifiers.
pub fn detect_code_language(path: &str) -> Option<&'static str> {
    let basename = path.rsplit('/').next().unwrap_or(path);
    let lower = basename.to_ascii_lowercase();
    if lower == "dockerfile"
        || lower == "containerfile"
        || lower.starts_with("dockerfile.")
        || lower.starts_with("containerfile.")
    {
        return Some("Dockerfile");
    }
    if lower == "makefile" || lower == "gnumakefile" {
        return Some("makefile");
    }
    if lower == "cmakelists.txt" {
        return Some("cmake");
    }
    if lower == "cargo.lock" {
        return Some("toml");
    }
    if matches!(
        lower.as_str(),
        ".bashrc"
            | ".bash_profile"
            | ".bash_logout"
            | ".profile"
            | ".zshrc"
            | ".zprofile"
            | ".ashrc"
            | "entrypoint"
    ) {
        return Some("bash");
    }
    if matches!(lower.as_str(), ".prettierrc" | ".eslintrc" | ".babelrc") {
        return Some("json");
    }
    if lower == ".env" || lower.starts_with(".env.") {
        return Some("ini");
    }

    let trimmed = lower.strip_prefix('.').unwrap_or(&lower);
    let ext = trimmed.rsplit_once('.')?.1;
    match ext {
        "rs" => Some("rust"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        "json" | "jsonc" | "webmanifest" | "map" => Some("json"),
        "yml" | "yaml" => Some("yaml"),
        "toml" => Some("toml"),
        "sh" | "bash" | "zsh" | "ash" | "dash" | "ksh" => Some("bash"),
        "ps1" | "psm1" => Some("powershell"),
        "py" | "pyi" => Some("python"),
        "go" => Some("go"),
        "md" | "markdown" | "mdx" => Some("markdown"),
        "xml" | "svg" | "xhtml" | "plist" => Some("xml"),
        "html" | "htm" | "vue" | "svelte" | "astro" => Some("html"),
        "css" | "scss" | "sass" | "less" => Some("css"),
        "sql" => Some("sql"),
        "conf" | "ini" | "env" | "cfg" | "cnf" | "properties" | "service" | "socket" | "timer" => {
            Some("ini")
        }
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => Some("cpp"),
        "java" | "gradle" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "cs" => Some("csharp"),
        "rb" | "erb" => Some("ruby"),
        "php" => Some("php"),
        "swift" => Some("swift"),
        "lua" => Some("lua"),
        "pl" | "pm" => Some("perl"),
        "r" => Some("r"),
        "proto" => Some("protobuf"),
        "graphql" | "gql" => Some("graphql"),
        "tf" | "tfvars" | "hcl" => Some("hcl"),
        "cmake" => Some("cmake"),
        "nix" => Some("nix"),
        _ => None,
    }
}

/// Encodes raw bytes as an RFC 2397 base64 `data:` URI for inline rendering.
pub fn encode_data_uri(mime: &str, bytes: &[u8]) -> String {
    let encoded = STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

/// Formats a [`StructuredPreviewMeta`] report into plain copyable text.
pub fn format_structured_text(meta: &StructuredPreviewMeta) -> String {
    let mut out = format!("{} — {}\n", meta.format, meta.summary);
    if !meta.fields.is_empty() {
        out.push('\n');
        for field in &meta.fields {
            out.push_str(&format!("{:<18} {}\n", field.label, field.value));
        }
    }
    for section in &meta.sections {
        if section.items.is_empty() {
            continue;
        }
        out.push_str(&format!("\n[{}]\n", section.title));
        for item in &section.items {
            out.push_str(&format!("  {item}\n"));
        }
    }
    out
}

/// Formats an [`ArchivePreviewMeta`] listing into plain copyable text.
pub fn format_archive_text(meta: &ArchivePreviewMeta) -> String {
    let mut out = format!(
        "{} ({} entries, {} uncompressed)\n\n",
        meta.format,
        meta.total_entries,
        ByteSize(meta.total_uncompressed_size)
    );
    for entry in &meta.entries {
        out.push_str(&format!(
            "{} {:>10}  {}\n",
            entry.mode,
            ByteSize(entry.size),
            entry.path
        ));
    }
    if meta.truncated_entries {
        out.push_str(&format!(
            "\n... showing first {} of {} entries\n",
            meta.entries.len(),
            meta.total_entries
        ));
    }
    out
}

/// Formats a [`MediaPreviewMeta`] header summary into plain copyable text.
pub fn format_media_text(meta: &MediaPreviewMeta, total_size: u64) -> String {
    let mut out = format!(
        "{} ({}) — {}\n",
        meta.format,
        meta.mime,
        ByteSize(total_size)
    );
    if let Some(detail) = &meta.detail {
        out.push_str(&format!("Details: {detail}\n"));
    }
    out
}

/// Inspects raw file bytes (up to read cap) and chooses the richest safe preview.
pub fn inspect_file_preview(path: &str, bytes: &[u8], total_size: u64) -> InspectedPreview {
    if bytes.is_empty() && total_size == 0 {
        return InspectedPreview::Empty;
    }

    let shown_bytes = bytes.len() as u64;
    let truncated = total_size > shown_bytes;

    if let Some(meta) = sniff_raster_image(bytes, total_size) {
        return InspectedPreview::Image { meta, truncated };
    }

    if let Some(inspected) = inspect_compressed(path, bytes, total_size, truncated) {
        return inspected;
    }

    if let Some(archive) = inspect_uncompressed_archive(path, bytes, truncated) {
        return InspectedPreview::Archive { meta: archive };
    }

    if let Some(media) = sniff_media(path, bytes) {
        return InspectedPreview::Media {
            meta: media,
            truncated,
        };
    }

    if let Some(structured) = inspect_structured_binary(path, bytes, total_size, truncated) {
        return InspectedPreview::Structured { meta: structured };
    }

    if !bytes.contains(&0) {
        if let Some(text) = decode_utf8_prefix(bytes, truncated) {
            return classify_utf8_text(path, text, None, total_size, truncated);
        }
    }

    InspectedPreview::Binary {
        shown_bytes,
        total_size,
        truncated,
    }
}

fn decode_utf8_prefix(bytes: &[u8], truncated: bool) -> Option<String> {
    match from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        Err(err) if truncated && err.error_len().is_none() && err.valid_up_to() > 0 => {
            from_utf8(&bytes[..err.valid_up_to()])
                .ok()
                .map(str::to_owned)
        }
        Err(_) => None,
    }
}

fn classify_utf8_text(
    path: &str,
    text: String,
    compression: Option<&'static str>,
    total_size: u64,
    truncated: bool,
) -> InspectedPreview {
    if !truncated && looks_like_svg(path, &text) {
        let meta = parse_svg_meta(&text);
        let formatted_source = if text.len() <= MAX_BEAUTIFY_CODE_BYTES && looks_minified(&text) {
            beautify_markup(&text)
        } else {
            None
        };
        return InspectedPreview::Svg {
            meta,
            source: text,
            formatted_source,
            truncated: false,
        };
    }

    let language = detect_code_language(path);
    let formatted_text = format_minified_text(language, &text, truncated);
    let shown_bytes = text.len() as u64;

    InspectedPreview::Text {
        text,
        formatted_text,
        language,
        compression,
        shown_bytes,
        total_size,
        truncated,
    }
}

fn pretty_json(text: &str) -> Option<String> {
    let val: JsonValue = json_from_str(text).ok()?;
    let pretty = to_string_pretty(&val).ok()?;
    if pretty.trim() == text.trim() {
        None
    } else {
        Some(pretty)
    }
}

fn looks_minified(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 120 {
        return false;
    }
    let mut non_empty_lines = 0_usize;
    let mut total_bytes = 0_usize;
    let mut long_line_bytes = 0_usize;
    for line in trimmed.lines() {
        let line_trimmed = line.trim();
        if line_trimmed.is_empty() {
            continue;
        }
        non_empty_lines += 1;
        let len = line.len();
        total_bytes += len;
        if len > 180 {
            long_line_bytes += len;
        }
    }
    if non_empty_lines == 0 {
        return false;
    }
    if non_empty_lines <= 3 && trimmed.len() >= 140 {
        return true;
    }
    if total_bytes / non_empty_lines > 160 {
        return true;
    }
    long_line_bytes.saturating_mul(100) / total_bytes.max(1) >= 65
}

fn format_minified_text(language: Option<&str>, text: &str, truncated: bool) -> Option<String> {
    match language {
        Some("json") if !truncated && text.len() <= 1_048_576 => pretty_json(text),
        Some("javascript" | "typescript" | "jsx" | "tsx")
            if text.len() <= MAX_BEAUTIFY_CODE_BYTES && looks_minified(text) =>
        {
            beautify_js_like(text)
        }
        Some("css") if text.len() <= MAX_BEAUTIFY_CODE_BYTES && looks_minified(text) => {
            beautify_css(text)
        }
        Some("xml" | "html")
            if !truncated && text.len() <= MAX_BEAUTIFY_CODE_BYTES && looks_minified(text) =>
        {
            beautify_markup(text)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BraceFrame {
    Block(usize),
    TemplateExpr,
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth.min(24) {
        out.push_str("  ");
    }
}

fn next_non_space(chars: &[char], mut idx: usize) -> Option<(usize, char)> {
    while let Some(&ch) = chars.get(idx) {
        if !matches!(ch, ' ' | '\t') {
            return Some((idx, ch));
        }
        idx += 1;
    }
    None
}

fn next_non_ws(chars: &[char], mut idx: usize) -> Option<(usize, char)> {
    while let Some(&ch) = chars.get(idx) {
        if !ch.is_whitespace() {
            return Some((idx, ch));
        }
        idx += 1;
    }
    None
}

fn ends_with_space_or_opener(out: &str) -> bool {
    matches!(
        out.chars().next_back(),
        None | Some(' ' | '\t' | '\n' | '(' | '[' | '{')
    )
}

fn allows_regex_after(prev_sig: Option<char>, prev_was_inc_dec: bool, last_word: &str) -> bool {
    if prev_was_inc_dec {
        return false;
    }
    if let Some(ch) = prev_sig {
        if matches!(
            ch,
            '(' | '['
                | '{'
                | ','
                | ';'
                | ':'
                | '='
                | '!'
                | '&'
                | '|'
                | '?'
                | '+'
                | '-'
                | '*'
                | '%'
                | '^'
                | '~'
                | '<'
                | '>'
                | '/'
        ) {
            return true;
        }
    } else {
        return true;
    }
    matches!(
        last_word,
        "return"
            | "typeof"
            | "void"
            | "delete"
            | "throw"
            | "case"
            | "in"
            | "of"
            | "instanceof"
            | "new"
            | "yield"
            | "await"
            | "else"
            | "do"
    )
}

fn scan_regex_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut in_class = false;
    let mut has_body = false;
    while let Some(&ch) = chars.get(i) {
        if ch == '\n' || ch == '\r' {
            return None;
        }
        if ch == '\\' {
            has_body = true;
            i += 2;
            continue;
        }
        if ch == '[' {
            in_class = true;
            has_body = true;
        } else if ch == ']' && in_class {
            in_class = false;
        } else if ch == '/' && !in_class {
            if !has_body {
                return None;
            }
            let mut end = i + 1;
            while let Some(&flag) = chars.get(end) {
                if flag.is_ascii_alphabetic() {
                    end += 1;
                } else {
                    break;
                }
            }
            return Some(end);
        } else {
            has_body = true;
        }
        i += 1;
    }
    None
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

fn scan_template_segment(
    chars: &[char],
    mut i: usize,
    out: &mut String,
    max_out: usize,
) -> (usize, bool) {
    while let Some(&ch) = chars.get(i) {
        if out.len() > max_out {
            return (i, false);
        }
        if ch == '\\' {
            out.push(ch);
            if let Some(&esc) = chars.get(i + 1) {
                out.push(esc);
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if ch == '$' && chars.get(i + 1) == Some(&'{') {
            out.push_str("${");
            return (i + 2, true);
        }
        out.push(ch);
        i += 1;
        if ch == '`' {
            return (i, false);
        }
    }
    (i, false)
}

fn beautify_js_like(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let max_out = (text.len().saturating_mul(4) + 4096).min(2 * 1024 * 1024);
    let mut out = String::with_capacity(text.len().saturating_mul(2).min(max_out));
    let mut i = 0_usize;
    let mut paren_depth = 0_usize;
    let mut brace_stack: Vec<BraceFrame> = Vec::new();
    let mut at_line_start = true;
    let mut prev_sig: Option<char> = None;
    let mut prev_was_inc_dec = false;
    let mut last_word = String::new();

    let block_depth = |stack: &[BraceFrame]| -> usize {
        stack
            .iter()
            .filter(|f| matches!(f, BraceFrame::Block(_)))
            .count()
    };

    while let Some(&ch) = chars.get(i) {
        if out.len() > max_out || brace_stack.len() > 128 {
            return None;
        }

        if ch == '\r' || ch == '\n' {
            if !at_line_start {
                out.push('\n');
                at_line_start = true;
            }
            if ch == '\r' && chars.get(i + 1) == Some(&'\n') {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if matches!(ch, ' ' | '\t') {
            if !at_line_start && !ends_with_space_or_opener(&out) {
                out.push(' ');
            }
            i += 1;
            continue;
        }

        if at_line_start && ch != '}' {
            push_indent(&mut out, block_depth(&brace_stack));
            at_line_start = false;
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            out.push(ch);
            i += 1;
            while let Some(&qch) = chars.get(i) {
                out.push(qch);
                i += 1;
                if qch == '\\' {
                    if let Some(&esc) = chars.get(i) {
                        out.push(esc);
                        i += 1;
                    }
                    continue;
                }
                if qch == quote {
                    break;
                }
            }
            prev_sig = Some(quote);
            prev_was_inc_dec = false;
            last_word.clear();
            continue;
        }

        if ch == '`' {
            out.push('`');
            i += 1;
            let (next_i, entered_expr) = scan_template_segment(&chars, i, &mut out, max_out);
            i = next_i;
            if entered_expr {
                brace_stack.push(BraceFrame::TemplateExpr);
                prev_sig = Some('{');
            } else {
                prev_sig = Some('`');
            }
            prev_was_inc_dec = false;
            last_word.clear();
            continue;
        }

        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            while let Some(&cch) = chars.get(i) {
                if cch == '\r' || cch == '\n' {
                    break;
                }
                out.push(cch);
                i += 1;
            }
            out.push('\n');
            at_line_start = true;
            if let Some(&nl) = chars.get(i) {
                if nl == '\r' && chars.get(i + 1) == Some(&'\n') {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        if ch == '/' && chars.get(i + 1) == Some(&'*') {
            out.push_str("/*");
            i += 2;
            let mut has_newline = false;
            while let Some(&cch) = chars.get(i) {
                if cch == '\n' {
                    has_newline = true;
                }
                out.push(cch);
                if cch == '*' && chars.get(i + 1) == Some(&'/') {
                    out.push('/');
                    i += 2;
                    break;
                }
                i += 1;
            }
            if has_newline {
                out.push('\n');
                at_line_start = true;
            }
            continue;
        }

        if ch == '/' && allows_regex_after(prev_sig, prev_was_inc_dec, &last_word) {
            if let Some(end) = scan_regex_end(&chars, i) {
                for &rch in &chars[i..end] {
                    out.push(rch);
                }
                i = end;
                prev_sig = Some('/');
                prev_was_inc_dec = false;
                last_word.clear();
                continue;
            }
        }

        match ch {
            '(' => {
                paren_depth += 1;
                out.push('(');
                prev_sig = Some('(');
                prev_was_inc_dec = false;
                last_word.clear();
                i += 1;
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                if out.ends_with(' ') {
                    out.pop();
                }
                out.push(')');
                prev_sig = Some(')');
                prev_was_inc_dec = false;
                last_word.clear();
                i += 1;
            }
            '{' => {
                if let Some((close_idx, '}')) = next_non_ws(&chars, i + 1) {
                    if !ends_with_space_or_opener(&out) && prev_sig != Some('{') {
                        out.push(' ');
                    }
                    out.push_str("{}");
                    prev_sig = Some('}');
                    prev_was_inc_dec = false;
                    last_word.clear();
                    i = close_idx + 1;
                    continue;
                }
                if !ends_with_space_or_opener(&out) {
                    out.push(' ');
                }
                out.push_str("{\n");
                brace_stack.push(BraceFrame::Block(paren_depth));
                at_line_start = true;
                prev_sig = Some('{');
                prev_was_inc_dec = false;
                last_word.clear();
                i += 1;
            }
            '}' => {
                if matches!(brace_stack.last(), Some(BraceFrame::TemplateExpr)) {
                    brace_stack.pop();
                    if out.ends_with(' ') {
                        out.pop();
                    }
                    out.push('}');
                    let (next_i, entered_expr) =
                        scan_template_segment(&chars, i + 1, &mut out, max_out);
                    i = next_i;
                    if entered_expr {
                        brace_stack.push(BraceFrame::TemplateExpr);
                        prev_sig = Some('{');
                    } else {
                        prev_sig = Some('`');
                    }
                    prev_was_inc_dec = false;
                    last_word.clear();
                    continue;
                }

                if matches!(brace_stack.last(), Some(BraceFrame::Block(_))) {
                    brace_stack.pop();
                }
                while out.ends_with(' ') {
                    out.pop();
                }
                let depth = block_depth(&brace_stack);
                if !at_line_start && !out.ends_with('\n') {
                    out.push('\n');
                }
                push_indent(&mut out, depth);
                out.push('}');
                at_line_start = false;
                prev_sig = Some('}');
                prev_was_inc_dec = false;
                last_word.clear();
                i += 1;

                let next_char = next_non_ws(&chars, i).map(|(_, c)| c);
                if matches!(next_char, Some(';' | ',' | ')' | ']')) {
                    if next_char == Some(',') {
                        // Keep `},` compact inside calls/arrays.
                    }
                } else if next_char.is_some() {
                    out.push('\n');
                    at_line_start = true;
                }
            }
            ';' => {
                let top_block_paren = brace_stack
                    .iter()
                    .rev()
                    .find_map(|f| match f {
                        BraceFrame::Block(d) => Some(*d),
                        BraceFrame::TemplateExpr => None,
                    })
                    .unwrap_or(0);
                if out.ends_with(' ') {
                    out.pop();
                }
                if paren_depth > top_block_paren {
                    out.push(';');
                    if let Some((_, next_c)) = next_non_space(&chars, i + 1) {
                        if next_c != ')' {
                            out.push(' ');
                        }
                    }
                } else {
                    out.push(';');
                    if next_non_ws(&chars, i + 1).is_some() {
                        out.push('\n');
                        at_line_start = true;
                    }
                }
                prev_sig = Some(';');
                prev_was_inc_dec = false;
                last_word.clear();
                i += 1;
            }
            ',' => {
                if out.ends_with(' ') {
                    out.pop();
                }
                out.push(',');
                if let Some((_, next_c)) = next_non_space(&chars, i + 1) {
                    if !matches!(next_c, ')' | ']' | '}' | '\n' | '\r') {
                        out.push(' ');
                    }
                }
                prev_sig = Some(',');
                prev_was_inc_dec = false;
                last_word.clear();
                i += 1;
            }
            _ => {
                prev_was_inc_dec =
                    (ch == '+' && prev_sig == Some('+')) || (ch == '-' && prev_sig == Some('-'));
                if is_ident_char(ch) {
                    if !prev_sig.is_some_and(is_ident_char) {
                        last_word.clear();
                    }
                    if last_word.len() < 32 {
                        last_word.push(ch);
                    }
                } else {
                    last_word.clear();
                }
                out.push(ch);
                prev_sig = Some(ch);
                i += 1;
            }
        }
    }

    let trimmed_out = out.trim_end();
    if trimmed_out == text.trim() || trimmed_out.lines().count() <= text.lines().count() {
        None
    } else {
        let mut owned = trimmed_out.to_owned();
        owned.push('\n');
        Some(owned)
    }
}

fn beautify_css(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let max_out = (text.len().saturating_mul(4) + 4096).min(2 * 1024 * 1024);
    let mut out = String::with_capacity(text.len().saturating_mul(2).min(max_out));
    let mut i = 0_usize;
    let mut indent = 0_usize;
    let mut paren_depth = 0_usize;
    let mut at_line_start = true;

    while let Some(&ch) = chars.get(i) {
        if out.len() > max_out {
            return None;
        }
        if ch == '\r' || ch == '\n' {
            if !at_line_start {
                out.push('\n');
                at_line_start = true;
            }
            if ch == '\r' && chars.get(i + 1) == Some(&'\n') {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if matches!(ch, ' ' | '\t') {
            if !at_line_start && !ends_with_space_or_opener(&out) {
                out.push(' ');
            }
            i += 1;
            continue;
        }

        if at_line_start && ch != '}' {
            push_indent(&mut out, indent);
            at_line_start = false;
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            out.push(ch);
            i += 1;
            while let Some(&qch) = chars.get(i) {
                out.push(qch);
                i += 1;
                if qch == '\\' {
                    if let Some(&esc) = chars.get(i) {
                        out.push(esc);
                        i += 1;
                    }
                    continue;
                }
                if qch == quote {
                    break;
                }
            }
            continue;
        }

        if ch == '/' && chars.get(i + 1) == Some(&'*') {
            out.push_str("/*");
            i += 2;
            while let Some(&cch) = chars.get(i) {
                out.push(cch);
                if cch == '*' && chars.get(i + 1) == Some(&'/') {
                    out.push('/');
                    i += 2;
                    break;
                }
                i += 1;
            }
            if next_non_ws(&chars, i).is_some() {
                out.push('\n');
                at_line_start = true;
            }
            continue;
        }

        match ch {
            '(' => {
                paren_depth += 1;
                out.push('(');
                i += 1;
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                if out.ends_with(' ') {
                    out.pop();
                }
                out.push(')');
                i += 1;
            }
            '{' => {
                if !ends_with_space_or_opener(&out) {
                    out.push(' ');
                }
                out.push_str("{\n");
                indent += 1;
                at_line_start = true;
                i += 1;
            }
            '}' => {
                indent = indent.saturating_sub(1);
                while out.ends_with(' ') {
                    out.pop();
                }
                if !at_line_start && !out.ends_with('\n') {
                    out.push('\n');
                }
                push_indent(&mut out, indent);
                out.push('}');
                at_line_start = false;
                i += 1;
                if next_non_ws(&chars, i).is_some() {
                    out.push('\n');
                    at_line_start = true;
                }
            }
            ';' if paren_depth == 0 => {
                if out.ends_with(' ') {
                    out.pop();
                }
                out.push(';');
                i += 1;
                if next_non_ws(&chars, i).is_some() {
                    out.push('\n');
                    at_line_start = true;
                }
            }
            ':' if paren_depth == 0 && indent > 0 => {
                out.push(':');
                if let Some((_, next_c)) = next_non_space(&chars, i + 1) {
                    if next_c != ':' && next_c != '\n' {
                        out.push(' ');
                    }
                }
                i += 1;
            }
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }

    let trimmed_out = out.trim_end();
    if trimmed_out == text.trim() || trimmed_out.lines().count() <= text.lines().count() {
        None
    } else {
        let mut owned = trimmed_out.to_owned();
        owned.push('\n');
        Some(owned)
    }
}

fn is_html_void_tag(tag_name: &str) -> bool {
    matches!(
        tag_name.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn find_closing_tag_bracket(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = start;
    let mut quote: Option<u8> = None;
    while let Some(&b) = bytes.get(i) {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn extract_tag_name(tag: &str) -> &str {
    let inner = tag
        .strip_prefix("</")
        .or_else(|| tag.strip_prefix('<'))
        .unwrap_or(tag)
        .trim_start();
    inner
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
}

fn beautify_markup(text: &str) -> Option<String> {
    let max_out = (text.len().saturating_mul(3) + 4096).min(2 * 1024 * 1024);
    let mut out = String::with_capacity(text.len().saturating_mul(2).min(max_out));
    let mut pos = 0_usize;
    let mut indent = 0_usize;
    let mut prev_ended_gt = false;

    while pos < text.len() {
        if out.len() > max_out {
            return None;
        }
        if let Some(rel) = text[pos..].find('<') {
            let tag_start = pos + rel;
            let between = &text[pos..tag_start];
            let has_text = !between.trim().is_empty();
            if has_text {
                out.push_str(between);
                prev_ended_gt = false;
            }

            let tag_end = if text[tag_start..].starts_with("<!--") {
                match text[tag_start + 4..].find("-->") {
                    Some(end) => tag_start + 4 + end + 2,
                    None => text.len() - 1,
                }
            } else if text[tag_start..].starts_with("<![CDATA[") {
                match text[tag_start + 9..].find("]]>") {
                    Some(end) => tag_start + 9 + end + 2,
                    None => text.len() - 1,
                }
            } else {
                find_closing_tag_bracket(text, tag_start + 1).unwrap_or(text.len() - 1)
            };

            let tag = &text[tag_start..=tag_end];
            let is_closing = tag.starts_with("</");
            let is_special = tag.starts_with("<?") || tag.starts_with("<!");
            let is_self_closing = tag.ends_with("/>");
            let tag_name = extract_tag_name(tag);

            if is_closing {
                indent = indent.saturating_sub(1);
            }
            if prev_ended_gt && !has_text {
                out.push('\n');
                push_indent(&mut out, indent);
            }
            out.push_str(tag);
            prev_ended_gt = true;

            pos = tag_end + 1;
            if !is_closing && !is_special && !is_self_closing && !is_html_void_tag(tag_name) {
                if tag_name.eq_ignore_ascii_case("script") || tag_name.eq_ignore_ascii_case("style")
                {
                    let end_pat = if tag_name.eq_ignore_ascii_case("script") {
                        "</script>"
                    } else {
                        "</style>"
                    };
                    if let Some(close_rel) = text[pos..].to_ascii_lowercase().find(end_pat) {
                        let raw_body = &text[pos..pos + close_rel];
                        out.push_str(raw_body);
                        pos += close_rel;
                        prev_ended_gt = !raw_body.trim().is_empty();
                        indent += 1;
                        continue;
                    }
                }
                indent += 1;
            }
        } else {
            out.push_str(&text[pos..]);
            break;
        }
    }

    let trimmed_out = out.trim();
    if trimmed_out == text.trim() || trimmed_out.lines().count() <= text.lines().count() {
        None
    } else {
        let mut owned = trimmed_out.to_owned();
        owned.push('\n');
        Some(owned)
    }
}

fn lower_extension(path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let trimmed = base.strip_prefix('.').unwrap_or(&base);
    trimmed.rsplit_once('.').map(|(_, ext)| ext.to_owned())
}

fn looks_like_svg(path: &str, text: &str) -> bool {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let Some(svg_idx) = find_svg_tag_start(trimmed) else {
        return false;
    };
    let is_svg_ext = lower_extension(path).as_deref() == Some("svg");
    if is_svg_ext {
        return true;
    }
    svg_idx == 0
        || trimmed.starts_with("<?xml")
        || trimmed.starts_with("<!DOCTYPE svg")
        || trimmed.starts_with("<!--")
}

fn find_svg_tag_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut pos = 0;
    while let Some(rel) = text[pos..].find("<svg") {
        let idx = pos + rel;
        let next = bytes.get(idx + 4).copied();
        if next.is_some_and(|b| b.is_ascii_whitespace() || b == b'>' || b == b'/') {
            return Some(idx);
        }
        pos = idx + 4;
    }
    None
}

fn parse_svg_meta(source: &str) -> ImagePreviewMeta {
    let open_tag = find_svg_tag_start(source).and_then(|start| {
        let rest = &source[start..];
        rest.find('>').map(|end| &rest[..=end])
    });
    let mut width = open_tag
        .and_then(|tag| extract_xml_attr(tag, "width"))
        .and_then(|v| parse_svg_length(&v));
    let mut height = open_tag
        .and_then(|tag| extract_xml_attr(tag, "height"))
        .and_then(|v| parse_svg_length(&v));
    let view_box = open_tag.and_then(|tag| extract_xml_attr(tag, "viewBox"));

    if width.is_none() || height.is_none() {
        if let Some(vb) = view_box.as_deref() {
            let nums: Vec<f64> = vb
                .split(|c: char| c.is_ascii_whitespace() || c == ',')
                .filter(|part| !part.is_empty())
                .filter_map(|part| part.parse::<f64>().ok())
                .collect();
            if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
                if width.is_none() {
                    width = Some(nums[2].round().clamp(1.0, 16384.0) as u32);
                }
                if height.is_none() {
                    height = Some(nums[3].round().clamp(1.0, 16384.0) as u32);
                }
            }
        }
    }

    ImagePreviewMeta {
        format: "SVG".to_owned(),
        mime: "image/svg+xml".to_owned(),
        width,
        height,
        detail: view_box.map(|vb| format!("viewBox {vb}")),
    }
}

fn extract_xml_attr(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = tag[search_from..].find(name) {
        let start = search_from + rel;
        let prev_ok = start == 0
            || bytes
                .get(start - 1)
                .is_some_and(|b| b.is_ascii_whitespace());
        let after_name = start + name.len();
        if prev_ok {
            let rest = tag[after_name..].trim_start();
            if let Some(after_eq) = rest.strip_prefix('=') {
                let val_rest = after_eq.trim_start();
                let quote = val_rest.as_bytes().first().copied()?;
                if quote == b'"' || quote == b'\'' {
                    let inner = &val_rest[1..];
                    if let Some(end) = inner.find(quote as char) {
                        return Some(inner[..end].trim().to_owned());
                    }
                }
            }
        }
        search_from = after_name;
    }
    None
}

fn parse_svg_length(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if trimmed.ends_with('%') {
        return None;
    }
    let num_str = trimmed
        .strip_suffix("px")
        .or_else(|| trimmed.strip_suffix("pt"))
        .unwrap_or(trimmed)
        .trim();
    let parsed = num_str.parse::<f64>().ok()?;
    if parsed.is_finite() && parsed > 0.0 {
        Some(parsed.round().clamp(1.0, 16384.0) as u32)
    } else {
        None
    }
}

fn read_u16_be(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_be(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> Option<i32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn sniff_raster_image(bytes: &[u8], total_size: u64) -> Option<ImagePreviewMeta> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let (width, height, detail) = if bytes.get(12..16) == Some(b"IHDR") {
            let w = read_u32_be(bytes, 16);
            let h = read_u32_be(bytes, 20);
            let depth = bytes.get(24).copied();
            let color = bytes.get(25).copied().map(|c| match c {
                0 => "Grayscale",
                2 => "RGB",
                3 => "Indexed",
                4 => "Grayscale+Alpha",
                6 => "RGBA",
                _ => "PNG",
            });
            let detail = match (depth, color) {
                (Some(d), Some(c)) => Some(format!("{d}-bit {c}")),
                _ => None,
            };
            (w, h, detail)
        } else {
            (None, None, None)
        };
        return Some(ImagePreviewMeta {
            format: "PNG".to_owned(),
            mime: "image/png".to_owned(),
            width,
            height,
            detail,
        });
    }

    if bytes.starts_with(b"\xff\xd8\xff") {
        let (width, height, detail) = parse_jpeg_dimensions(bytes);
        return Some(ImagePreviewMeta {
            format: "JPEG".to_owned(),
            mime: "image/jpeg".to_owned(),
            width,
            height,
            detail,
        });
    }

    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        let width = read_u16_le(bytes, 6).map(u32::from);
        let height = read_u16_le(bytes, 8).map(u32::from);
        let version = from_utf8(&bytes[..6]).ok().map(str::to_owned);
        return Some(ImagePreviewMeta {
            format: "GIF".to_owned(),
            mime: "image/gif".to_owned(),
            width,
            height,
            detail: version,
        });
    }

    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let (width, height, detail) = parse_webp_dimensions(bytes);
        return Some(ImagePreviewMeta {
            format: "WebP".to_owned(),
            mime: "image/webp".to_owned(),
            width,
            height,
            detail,
        });
    }

    if let Some((width, height, bpp)) = parse_bmp_dimensions(bytes) {
        return Some(ImagePreviewMeta {
            format: "BMP".to_owned(),
            mime: "image/bmp".to_owned(),
            width: Some(width),
            height: Some(height),
            detail: Some(format!("{bpp}-bit")),
        });
    }

    if let Some((width, height, count, kind_label)) = parse_ico_dimensions(bytes, total_size) {
        return Some(ImagePreviewMeta {
            format: kind_label.to_owned(),
            mime: "image/x-icon".to_owned(),
            width: Some(width),
            height: Some(height),
            detail: Some(format!(
                "{count} {}",
                if count == 1 { "icon" } else { "icons" }
            )),
        });
    }

    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" {
            return Some(ImagePreviewMeta {
                format: "AVIF".to_owned(),
                mime: "image/avif".to_owned(),
                width: None,
                height: None,
                detail: None,
            });
        }
        if brand == b"heic" || brand == b"heix" || brand == b"hevc" || brand == b"mif1" {
            return Some(ImagePreviewMeta {
                format: "HEIC".to_owned(),
                mime: "image/heic".to_owned(),
                width: None,
                height: None,
                detail: None,
            });
        }
    }

    if bytes.starts_with(b"II*\x00") || bytes.starts_with(b"MM\x00*") {
        let endian = if bytes.starts_with(b"II") {
            "Little-endian TIFF"
        } else {
            "Big-endian TIFF"
        };
        return Some(ImagePreviewMeta {
            format: "TIFF".to_owned(),
            mime: "image/tiff".to_owned(),
            width: None,
            height: None,
            detail: Some(endian.to_owned()),
        });
    }

    None
}

fn parse_jpeg_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>, Option<String>) {
    let mut i = 2;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] != 0xff {
            i += 1;
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let marker = bytes[i];
        i += 1;

        if marker == 0x00 || marker == 0x01 || (0xd0..=0xd9).contains(&marker) {
            if marker == 0xd9 {
                break;
            }
            continue;
        }
        if marker == 0xda {
            break;
        }

        let Some(seg_len) = read_u16_be(bytes, i) else {
            break;
        };
        if seg_len < 2 {
            break;
        }
        let is_sof = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if is_sof && i + 7 < bytes.len() {
            let precision = bytes[i + 2];
            let height = read_u16_be(bytes, i + 3).map(u32::from);
            let width = read_u16_be(bytes, i + 5).map(u32::from);
            let components = bytes[i + 7];
            let encoding = if marker == 0xc2 {
                "Progressive"
            } else {
                "Baseline"
            };
            let detail = Some(format!("{encoding}, {precision}-bit × {components} ch"));
            return (width, height, detail);
        }
        let next = i.saturating_add(usize::from(seg_len));
        if next > bytes.len() {
            break;
        }
        i = next;
    }
    (None, None, None)
}

fn parse_webp_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>, Option<String>) {
    match bytes.get(12..16) {
        Some(b"VP8 ") if bytes.len() >= 30 && bytes.get(23..26) == Some(&[0x9d, 0x01, 0x2a]) => {
            let w = read_u16_le(bytes, 26).map(|v| u32::from(v & 0x3fff));
            let h = read_u16_le(bytes, 28).map(|v| u32::from(v & 0x3fff));
            (w, h, Some("Lossy VP8".to_owned()))
        }
        Some(b"VP8L") if bytes.len() >= 25 && bytes.get(20).copied() == Some(0x2f) => {
            let packed = read_u32_le(bytes, 21).unwrap_or(0);
            let w = (packed & 0x3fff) + 1;
            let h = ((packed >> 14) & 0x3fff) + 1;
            (Some(w), Some(h), Some("Lossless VP8L".to_owned()))
        }
        Some(b"VP8X") if bytes.len() >= 30 => {
            let w = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]) + 1;
            let h = u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]) + 1;
            (Some(w), Some(h), Some("Extended VP8X".to_owned()))
        }
        _ => (None, None, None),
    }
}

fn parse_bmp_dimensions(bytes: &[u8]) -> Option<(u32, u32, u16)> {
    if !bytes.starts_with(b"BM") || bytes.len() < 26 {
        return None;
    }
    let dib_size = read_u32_le(bytes, 14)?;
    if !matches!(dib_size, 12 | 40 | 52 | 56 | 108 | 124) {
        return None;
    }
    let (w, h, bpp) = if dib_size == 12 {
        let w = u32::from(read_u16_le(bytes, 18)?);
        let h = u32::from(read_u16_le(bytes, 20)?);
        let bpp = read_u16_le(bytes, 24).unwrap_or(24);
        (w, h, bpp)
    } else {
        let w = read_i32_le(bytes, 18)?.unsigned_abs();
        let h = read_i32_le(bytes, 22)?.unsigned_abs();
        let bpp = read_u16_le(bytes, 28).unwrap_or(24);
        (w, h, bpp)
    };
    if w == 0 || h == 0 || w > 65535 || h > 65535 {
        return None;
    }
    Some((w, h, bpp))
}

fn parse_ico_dimensions(bytes: &[u8], total_size: u64) -> Option<(u32, u32, u16, &'static str)> {
    let kind_code = bytes.get(0..4)?;
    let kind_label = match kind_code {
        [0, 0, 1, 0] => "ICO",
        [0, 0, 2, 0] => "CUR",
        _ => return None,
    };
    let count = read_u16_le(bytes, 4)?;
    if count == 0 || count > 64 {
        return None;
    }
    let dir_end = 6 + usize::from(count) * 16;
    if bytes.len() < dir_end || total_size < dir_end as u64 {
        return None;
    }

    let mut max_w = 0_u32;
    let mut max_h = 0_u32;
    for idx in 0..usize::from(count) {
        let entry = &bytes[6 + idx * 16..6 + (idx + 1) * 16];
        if entry[3] != 0 {
            return None;
        }
        let img_size = read_u32_le(entry, 8)?;
        let img_offset = read_u32_le(entry, 12)?;
        if img_size == 0 || (img_offset as usize) < dir_end {
            return None;
        }
        let w = if entry[0] == 0 {
            256
        } else {
            u32::from(entry[0])
        };
        let h = if entry[1] == 0 {
            256
        } else {
            u32::from(entry[1])
        };
        max_w = max_w.max(w);
        max_h = max_h.max(h);
    }

    Some((max_w, max_h, count, kind_label))
}

fn inspect_compressed(
    path: &str,
    bytes: &[u8],
    total_size: u64,
    input_truncated: bool,
) -> Option<InspectedPreview> {
    let is_gzip = bytes.starts_with(&[0x1f, 0x8b, 0x08]);
    let is_zstd = bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]);
    if !is_gzip && !is_zstd {
        return None;
    }

    let compression_name = if is_gzip { "gzip" } else { "zstd" };
    let lower = path.to_ascii_lowercase();
    let likely_tar = lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.zst")
        || lower.ends_with(".tzst")
        || lower.ends_with(".apk")
        || lower.ends_with(".crate");

    if !input_truncated && likely_tar {
        if let Some(archive) = inspect_tar_compressed(bytes, is_gzip) {
            return Some(InspectedPreview::Archive { meta: archive });
        }
    }

    let (decomp, decomp_truncated) = if is_gzip {
        decompress_limited(
            MultiGzDecoder::new(Cursor::new(bytes)),
            PREVIEW_DECOMPRESSED_TEXT_LIMIT,
        )
    } else {
        let Ok(decoder) = ZstdDecoder::new(Cursor::new(bytes)) else {
            return None;
        };
        decompress_limited(decoder, PREVIEW_DECOMPRESSED_TEXT_LIMIT)
    }?;

    if !input_truncated && decomp.len() >= 265 && &decomp[257..262] == b"ustar" {
        if let Some(archive) = inspect_tar_compressed(bytes, is_gzip) {
            return Some(InspectedPreview::Archive { meta: archive });
        }
    }

    if decomp.is_empty() || decomp.contains(&0) {
        return None;
    }

    let truncated = input_truncated || decomp_truncated;
    let text = decode_utf8_prefix(&decomp, truncated)?;
    let inner_path = strip_compression_suffix(path);
    Some(classify_utf8_text(
        &inner_path,
        text,
        Some(compression_name),
        total_size,
        truncated,
    ))
}

fn decompress_limited<R: Read>(reader: R, limit: usize) -> Option<(Vec<u8>, bool)> {
    let mut limited = reader.take((limit + 1) as u64);
    let mut buf = Vec::new();
    if limited.read_to_end(&mut buf).is_err() && buf.is_empty() {
        return None;
    }
    let truncated = buf.len() > limit;
    if truncated {
        buf.truncate(limit);
    }
    Some((buf, truncated))
}

fn strip_compression_suffix(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".svgz") {
        let stem_len = path.len().saturating_sub(5);
        return format!("{}.svg", &path[..stem_len]);
    }
    if lower.ends_with(".gz") {
        return path[..path.len().saturating_sub(3)].to_owned();
    }
    if lower.ends_with(".zst") {
        return path[..path.len().saturating_sub(4)].to_owned();
    }
    path.to_owned()
}

fn inspect_tar_compressed(bytes: &[u8], is_gzip: bool) -> Option<ArchivePreviewMeta> {
    if is_gzip {
        let decoder = MultiGzDecoder::new(Cursor::new(bytes)).take(PREVIEW_ARCHIVE_SCAN_BYTES);
        inspect_tar_reader(decoder, "TAR.GZ")
    } else {
        let decoder = ZstdDecoder::new(Cursor::new(bytes))
            .ok()?
            .take(PREVIEW_ARCHIVE_SCAN_BYTES);
        inspect_tar_reader(decoder, "TAR.ZST")
    }
}

fn inspect_uncompressed_archive(
    path: &str,
    bytes: &[u8],
    truncated: bool,
) -> Option<ArchivePreviewMeta> {
    if bytes.len() >= 265 && &bytes[257..262] == b"ustar" {
        return inspect_tar_reader(Cursor::new(bytes).take(PREVIEW_ARCHIVE_SCAN_BYTES), "TAR");
    }

    if bytes.starts_with(&ZIP_LFH_SIG) || bytes.starts_with(&ZIP_EOCD_SIG) {
        let fmt = zip_format_label(path);
        return inspect_zip_bytes(bytes, fmt, truncated);
    }

    None
}

fn zip_format_label(path: &str) -> &'static str {
    match lower_extension(path).as_deref() {
        Some("jar") => "JAR",
        Some("war") => "WAR",
        Some("ear") => "EAR",
        Some("whl") => "Wheel (ZIP)",
        Some("nupkg") => "NuGet (ZIP)",
        Some("apk") => "APK (ZIP)",
        _ => "ZIP",
    }
}

fn inspect_tar_reader<R: Read>(reader: R, format: &str) -> Option<ArchivePreviewMeta> {
    let mut archive = Archive::new(reader);
    let entries_iter = archive.entries().ok()?;
    let mut entries = Vec::new();
    let mut total_entries = 0_usize;
    let mut total_uncompressed_size = 0_u64;

    for entry_res in entries_iter {
        let Ok(entry) = entry_res else {
            break;
        };
        let header = entry.header();
        let entry_path = entry
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "<invalid path>".to_owned());
        let size = header.size().unwrap_or(0);
        let raw_mode = header.mode().unwrap_or(0);
        let is_dir = header.entry_type().is_dir();
        let full_mode = if is_dir {
            raw_mode | 0o040_000
        } else if header.entry_type().is_symlink() {
            raw_mode | 0o120_000
        } else {
            raw_mode | 0o100_000
        };
        let mode = mode_to_string(full_mode);

        total_entries += 1;
        total_uncompressed_size = total_uncompressed_size.saturating_add(size);

        if entries.len() < PREVIEW_ARCHIVE_MAX_ENTRIES {
            entries.push(ArchiveEntryPreview {
                path: entry_path,
                size,
                compressed_size: None,
                mode,
                is_dir,
            });
        }
    }

    if total_entries == 0 {
        return None;
    }

    Some(ArchivePreviewMeta {
        format: format.to_owned(),
        total_entries,
        total_uncompressed_size,
        truncated_entries: total_entries > entries.len(),
        entries,
    })
}

fn inspect_zip_bytes(
    bytes: &[u8],
    format: &str,
    input_truncated: bool,
) -> Option<ArchivePreviewMeta> {
    if !input_truncated {
        if let Some(meta) = inspect_zip_central_directory(bytes, format) {
            return Some(meta);
        }
    }
    inspect_zip_local_headers(bytes, format)
}

fn inspect_zip_central_directory(bytes: &[u8], format: &str) -> Option<ArchivePreviewMeta> {
    let min_tail = bytes.len().saturating_sub(65_557);
    let mut eocd_pos = None;
    let mut pos = bytes.len().saturating_sub(22);
    loop {
        if bytes.get(pos..pos + 4) == Some(&ZIP_EOCD_SIG) {
            let comment_len = usize::from(read_u16_le(bytes, pos + 20)?);
            if pos + 22 + comment_len == bytes.len() {
                eocd_pos = Some(pos);
                break;
            }
        }
        if pos <= min_tail {
            break;
        }
        pos -= 1;
    }
    let eocd = eocd_pos?;
    let total_cd_records = usize::from(read_u16_le(bytes, eocd + 10)?);
    let cd_size = read_u32_le(bytes, eocd + 12)? as usize;
    let cd_offset = read_u32_le(bytes, eocd + 16)? as usize;

    // ZIP64 uses sentinel 0xFFFF / 0xFFFFFFFF; fall back to local scan in that case.
    if total_cd_records == 0xffff || cd_size == u32::MAX as usize || cd_offset == u32::MAX as usize
    {
        return None;
    }
    let cd_end = cd_offset.checked_add(cd_size)?;
    if cd_end > bytes.len() {
        return None;
    }

    let mut cursor = cd_offset;
    let mut entries = Vec::new();
    let mut total_entries = 0_usize;
    let mut total_uncompressed_size = 0_u64;

    while cursor + 46 <= cd_end {
        if bytes.get(cursor..cursor + 4) != Some(&ZIP_CD_SIG) {
            break;
        }
        let creator_host = bytes[cursor + 5];
        let compressed_size = read_u32_le(bytes, cursor + 20)? as u64;
        let uncompressed_size = read_u32_le(bytes, cursor + 24)? as u64;
        let name_len = usize::from(read_u16_le(bytes, cursor + 28)?);
        let extra_len = usize::from(read_u16_le(bytes, cursor + 30)?);
        let comment_len = usize::from(read_u16_le(bytes, cursor + 32)?);
        let ext_attr = read_u32_le(bytes, cursor + 38)?;

        let name_start = cursor + 46;
        let next = name_start + name_len + extra_len + comment_len;
        if next > cd_end {
            break;
        }

        let name = String::from_utf8_lossy(&bytes[name_start..name_start + name_len]).into_owned();
        let is_dir = name.ends_with('/');
        let unix_mode_bits = if matches!(creator_host, 3 | 19) {
            ext_attr >> 16
        } else {
            0
        };
        let mode = if unix_mode_bits != 0 {
            mode_to_string(unix_mode_bits)
        } else if is_dir {
            "drwxr-xr-x".to_owned()
        } else {
            "-rw-r--r--".to_owned()
        };

        total_entries += 1;
        total_uncompressed_size = total_uncompressed_size.saturating_add(uncompressed_size);
        if entries.len() < PREVIEW_ARCHIVE_MAX_ENTRIES {
            entries.push(ArchiveEntryPreview {
                path: name,
                size: uncompressed_size,
                compressed_size: Some(compressed_size),
                mode,
                is_dir,
            });
        }
        cursor = next;
    }

    if total_entries == 0 {
        return None;
    }

    Some(ArchivePreviewMeta {
        format: format.to_owned(),
        total_entries,
        total_uncompressed_size,
        truncated_entries: total_entries > entries.len(),
        entries,
    })
}

fn inspect_zip_local_headers(bytes: &[u8], format: &str) -> Option<ArchivePreviewMeta> {
    let mut cursor = 0_usize;
    let mut entries = Vec::new();
    let mut total_entries = 0_usize;
    let mut total_uncompressed_size = 0_u64;

    while cursor + 30 <= bytes.len() {
        if bytes.get(cursor..cursor + 4) != Some(&ZIP_LFH_SIG) {
            break;
        }
        let flags = read_u16_le(bytes, cursor + 6)?;
        let comp_size = read_u32_le(bytes, cursor + 18)? as u64;
        let uncomp_size = read_u32_le(bytes, cursor + 22)? as u64;
        let name_len = usize::from(read_u16_le(bytes, cursor + 26)?);
        let extra_len = usize::from(read_u16_le(bytes, cursor + 28)?);
        let name_start = cursor + 30;
        let data_start = name_start + name_len + extra_len;
        if data_start > bytes.len() {
            break;
        }
        let name = String::from_utf8_lossy(&bytes[name_start..name_start + name_len]).into_owned();
        let is_dir = name.ends_with('/');

        total_entries += 1;
        total_uncompressed_size = total_uncompressed_size.saturating_add(uncomp_size);
        if entries.len() < PREVIEW_ARCHIVE_MAX_ENTRIES {
            entries.push(ArchiveEntryPreview {
                path: name,
                size: uncomp_size,
                compressed_size: Some(comp_size),
                mode: if is_dir { "drwxr-xr-x" } else { "-rw-r--r--" }.to_owned(),
                is_dir,
            });
        }

        // Data descriptor streams with bit 3 & zero local compressed size cannot be skipped safely.
        if (flags & 0x08) != 0 && comp_size == 0 && !is_dir {
            break;
        }
        let Some(next) = data_start.checked_add(comp_size as usize) else {
            break;
        };
        if next > bytes.len() {
            break;
        }
        cursor = next;
    }

    if total_entries == 0 {
        return None;
    }

    Some(ArchivePreviewMeta {
        format: format.to_owned(),
        total_entries,
        total_uncompressed_size,
        truncated_entries: true,
        entries,
    })
}

fn sniff_media(path: &str, bytes: &[u8]) -> Option<MediaPreviewMeta> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        let detail = parse_wav_detail(bytes);
        return Some(MediaPreviewMeta {
            media_type: "audio".to_owned(),
            format: "WAV".to_owned(),
            mime: "audio/wav".to_owned(),
            detail,
        });
    }

    if bytes.starts_with(b"fLaC") {
        let detail = parse_flac_detail(bytes);
        return Some(MediaPreviewMeta {
            media_type: "audio".to_owned(),
            format: "FLAC".to_owned(),
            mime: "audio/flac".to_owned(),
            detail,
        });
    }

    if bytes.starts_with(b"OggS") {
        let is_video = lower_extension(path).as_deref() == Some("ogv");
        return Some(MediaPreviewMeta {
            media_type: if is_video { "video" } else { "audio" }.to_owned(),
            format: "Ogg".to_owned(),
            mime: if is_video { "video/ogg" } else { "audio/ogg" }.to_owned(),
            detail: None,
        });
    }

    if bytes.starts_with(b"ID3")
        || (lower_extension(path).as_deref() == Some("mp3")
            && bytes.len() >= 2
            && bytes[0] == 0xff
            && (bytes[1] & 0xe0) == 0xe0)
    {
        let detail = if bytes.starts_with(b"ID3") && bytes.len() >= 5 {
            Some(format!("ID3v2.{}.{}", bytes[3], bytes[4]))
        } else {
            Some("MPEG audio frame".to_owned())
        };
        return Some(MediaPreviewMeta {
            media_type: "audio".to_owned(),
            format: "MP3".to_owned(),
            mime: "audio/mpeg".to_owned(),
            detail,
        });
    }

    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"M4A " || brand == b"M4B " {
            return Some(MediaPreviewMeta {
                media_type: "audio".to_owned(),
                format: "M4A".to_owned(),
                mime: "audio/mp4".to_owned(),
                detail: Some(String::from_utf8_lossy(brand).trim().to_owned()),
            });
        }
        if matches!(
            brand,
            b"isom" | b"iso2" | b"mp41" | b"mp42" | b"avc1" | b"qt  " | b"M4V "
        ) {
            return Some(MediaPreviewMeta {
                media_type: "video".to_owned(),
                format: if brand == b"qt  " { "MOV" } else { "MP4" }.to_owned(),
                mime: if brand == b"qt  " {
                    "video/quicktime"
                } else {
                    "video/mp4"
                }
                .to_owned(),
                detail: Some(format!("ftyp {}", String::from_utf8_lossy(brand).trim())),
            });
        }
    }

    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let probe = &bytes[..bytes.len().min(64)];
        if probe.windows(4).any(|w| w == b"webm") {
            return Some(MediaPreviewMeta {
                media_type: "video".to_owned(),
                format: "WebM".to_owned(),
                mime: "video/webm".to_owned(),
                detail: Some("EBML WebM container".to_owned()),
            });
        }
        if probe.windows(8).any(|w| w == b"matroska") {
            return Some(MediaPreviewMeta {
                media_type: "video".to_owned(),
                format: "MKV".to_owned(),
                mime: "video/x-matroska".to_owned(),
                detail: Some("Matroska container".to_owned()),
            });
        }
    }

    None
}

fn parse_wav_detail(bytes: &[u8]) -> Option<String> {
    let mut pos = 12_usize;
    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_size = read_u32_le(bytes, pos + 4)? as usize;
        let body = pos + 8;
        if chunk_id == b"fmt " && body + 16 <= bytes.len() {
            let channels = read_u16_le(bytes, body + 2)?;
            let sample_rate = read_u32_le(bytes, body + 4)?;
            let bits_per_sample = read_u16_le(bytes, body + 14)?;
            return Some(format!(
                "{sample_rate} Hz, {channels} ch, {bits_per_sample}-bit PCM"
            ));
        }
        let step = chunk_size + (chunk_size % 2);
        let Some(next) = body.checked_add(step) else {
            break;
        };
        pos = next;
    }
    None
}

fn parse_flac_detail(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 26 {
        return None;
    }
    let block_type = bytes[4] & 0x7f;
    if block_type != 0 {
        return None;
    }
    // STREAMINFO starts at byte 8: 10 bytes block sizes/frame sizes, then packed 8 bytes.
    let packed = u32::from_be_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    let sample_rate = packed >> 12;
    let channels = ((packed >> 9) & 0x07) + 1;
    let bits_per_sample = ((packed >> 4) & 0x1f) + 1;
    Some(format!(
        "{sample_rate} Hz, {channels} ch, {bits_per_sample}-bit lossless"
    ))
}

fn inspect_structured_binary(
    path: &str,
    bytes: &[u8],
    total_size: u64,
    input_truncated: bool,
) -> Option<StructuredPreviewMeta> {
    if bytes.starts_with(b"\x7fELF") && bytes.len() >= MIN_ELF_HEADER_BYTES {
        return inspect_elf(bytes, total_size, input_truncated);
    }

    if bytes.len() >= 8 && &bytes[0..4] == b"\x00asm" {
        return inspect_wasm(bytes);
    }

    if bytes.len() >= 100 && bytes.starts_with(b"SQLite format 3\x00") {
        return Some(inspect_sqlite(bytes, total_size));
    }

    if bytes.len() >= 8 && bytes[0..4] == [0xca, 0xfe, 0xba, 0xbe] {
        return inspect_cafebabe(bytes, path);
    }

    if bytes.len() >= 32
        && matches!(
            &bytes[0..4],
            [0xfe, 0xed, 0xfa, 0xcf]
                | [0xcf, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xce]
                | [0xce, 0xfa, 0xed, 0xfe]
        )
    {
        return inspect_macho(bytes);
    }

    if let Some(font) = inspect_font(path, bytes) {
        return Some(font);
    }

    if let Some(pdf) = inspect_pdf(bytes) {
        return Some(pdf);
    }

    None
}

fn field(label: &str, value: impl Into<String>) -> PreviewField {
    PreviewField {
        label: label.to_owned(),
        value: value.into(),
    }
}

fn elf_machine_name(machine: u16) -> String {
    match machine {
        EM_X86_64 => "x86_64 (AMD64)".to_owned(),
        EM_AARCH64 => "aarch64 (ARM64)".to_owned(),
        EM_386 => "x86 (32-bit)".to_owned(),
        EM_ARM => "ARM (32-bit)".to_owned(),
        EM_RISCV => "RISC-V".to_owned(),
        EM_PPC64 => "PowerPC64".to_owned(),
        EM_S390 => "s390x".to_owned(),
        EM_MIPS => "MIPS".to_owned(),
        ELF_MACHINE_LOONGARCH => "LoongArch".to_owned(),
        other => format!("Machine 0x{other:04x}"),
    }
}

fn elf_osabi_name(osabi: u8) -> &'static str {
    match osabi {
        ELFOSABI_NONE => "UNIX System V",
        ELFOSABI_GNU => "Linux / GNU",
        ELFOSABI_FREEBSD => "FreeBSD",
        ELFOSABI_OPENBSD => "OpenBSD",
        ELFOSABI_SOLARIS => "Solaris",
        ELFOSABI_ARM_AEABI => "ARM EABI",
        ELFOSABI_ARM => "ARM",
        _ => "Other",
    }
}

fn elf_endian_label(ei_data: u8) -> &'static str {
    match ei_data {
        ELFDATA2LSB => "Little-endian",
        ELFDATA2MSB => "Big-endian",
        _ => "Unknown",
    }
}

fn inspect_elf(
    bytes: &[u8],
    total_size: u64,
    input_truncated: bool,
) -> Option<StructuredPreviewMeta> {
    if !input_truncated {
        if let Ok(elf) = Elf::parse(bytes) {
            let class = class_to_str(elf.header.e_ident[EI_CLASS]);
            let endian = elf_endian_label(elf.header.e_ident[EI_DATA]);
            let kind = et_to_str(elf.header.e_type);
            let arch = elf_machine_name(elf.header.e_machine);
            let osabi = elf_osabi_name(elf.header.e_ident[EI_OSABI]);

            let mut debug_bytes = 0_u64;
            let mut sym_bytes = 0_u64;
            let mut has_symtab = false;
            let mut non_empty_sections: Vec<(String, u64)> = Vec::new();
            for sec in &elf.section_headers {
                let name = elf.shdr_strtab.get_at(sec.sh_name).unwrap_or_default();
                if name.starts_with(".debug_") || name.starts_with(".zdebug_") {
                    debug_bytes += sec.sh_size;
                }
                if sec.sh_type == SHT_SYMTAB || name == ".symtab" || name == ".strtab" {
                    has_symtab = true;
                    sym_bytes += sec.sh_size;
                }
                if !name.is_empty() && sec.sh_type != SHT_NOBITS && sec.sh_size > 0 {
                    non_empty_sections.push((name.to_owned(), sec.sh_size));
                }
            }
            non_empty_sections.sort_by_key(|(name, sz)| (Reverse(*sz), name.clone()));
            non_empty_sections.truncate(12);

            let stripped_label = if has_symtab || debug_bytes > 0 {
                format!(
                    "No (symbols {}, debug {})",
                    ByteSize(sym_bytes),
                    ByteSize(debug_bytes)
                )
            } else {
                "Yes".to_owned()
            };

            let mut fields = vec![
                field("Class / Endian", format!("{class} · {endian}")),
                field("ELF Type", kind),
                field("Architecture", arch.clone()),
                field("OS / ABI", osabi),
                field("Entrypoint", format!("0x{:x}", elf.entry)),
                field(
                    "Interpreter",
                    elf.interpreter
                        .unwrap_or("None (static / shared object)")
                        .to_owned(),
                ),
                field("Stripped", stripped_label),
            ];
            if let Some(soname) = elf.soname {
                fields.push(field("SONAME", soname));
            }

            let mut sections = Vec::new();
            if !elf.libraries.is_empty() {
                sections.push(StructuredSection {
                    title: format!("Needed Shared Libraries ({})", elf.libraries.len()),
                    items: elf.libraries.iter().map(|s| (*s).to_owned()).collect(),
                });
            }
            if !non_empty_sections.is_empty() {
                sections.push(StructuredSection {
                    title: "Largest Sections".to_owned(),
                    items: non_empty_sections
                        .into_iter()
                        .map(|(name, sz)| format!("{name:<20} {}", ByteSize(sz)))
                        .collect(),
                });
            }

            return Some(StructuredPreviewMeta {
                format: "ELF Binary".to_owned(),
                summary: format!("{class} {kind} · {arch} · {}", ByteSize(total_size)),
                fields,
                sections,
            });
        }
    }

    let header = Elf::parse_header(bytes).ok()?;
    let class = class_to_str(header.e_ident[EI_CLASS]);
    let endian = elf_endian_label(header.e_ident[EI_DATA]);
    let kind = et_to_str(header.e_type);
    let arch = elf_machine_name(header.e_machine);
    Some(StructuredPreviewMeta {
        format: "ELF Binary (Header)".to_owned(),
        summary: format!("{class} {kind} · {arch} · {}", ByteSize(total_size)),
        fields: vec![
            field("Class / Endian", format!("{class} · {endian}")),
            field("ELF Type", kind),
            field("Architecture", arch),
            field("OS / ABI", elf_osabi_name(header.e_ident[EI_OSABI])),
            field("Entrypoint", format!("0x{:x}", header.e_entry)),
            field("Program Headers", header.e_phnum.to_string()),
            field("Section Headers", header.e_shnum.to_string()),
        ],
        sections: Vec::new(),
    })
}

fn read_leb128_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
    let mut result = 0_u32;
    let mut shift = 0_u32;
    for _ in 0..5 {
        let byte = *bytes.get(*offset)?;
        *offset += 1;
        result |= u32::from(byte & 0x7f) << shift;
        if (byte & 0x80) == 0 {
            return Some(result);
        }
        shift += 7;
    }
    None
}

fn inspect_wasm(bytes: &[u8]) -> Option<StructuredPreviewMeta> {
    let version = read_u32_le(bytes, 4)?;
    let mut cursor = 8_usize;
    let mut section_items = Vec::new();
    let mut custom_sections = Vec::new();

    while cursor < bytes.len() && section_items.len() < 40 {
        let sec_id = bytes[cursor];
        cursor += 1;
        let Some(sec_len) = read_leb128_u32(bytes, &mut cursor) else {
            break;
        };
        let sec_end = cursor.saturating_add(sec_len as usize);
        let label = match sec_id {
            0 => {
                let mut name_cur = cursor;
                let custom_name = read_leb128_u32(bytes, &mut name_cur)
                    .and_then(|n| bytes.get(name_cur..name_cur.saturating_add(n as usize)))
                    .and_then(|s| from_utf8(s).ok())
                    .unwrap_or("custom");
                custom_sections.push(custom_name.to_owned());
                format!("Custom ({custom_name})")
            }
            1 => "Type".to_owned(),
            2 => "Import".to_owned(),
            3 => "Function".to_owned(),
            4 => "Table".to_owned(),
            5 => "Memory".to_owned(),
            6 => "Global".to_owned(),
            7 => "Export".to_owned(),
            8 => "Start".to_owned(),
            9 => "Element".to_owned(),
            10 => "Code".to_owned(),
            11 => "Data".to_owned(),
            12 => "DataCount".to_owned(),
            other => format!("Section {other}"),
        };
        section_items.push(format!("{label:<24} {}", ByteSize(u64::from(sec_len))));
        if sec_end > bytes.len() {
            break;
        }
        cursor = sec_end;
    }

    let mut sections = Vec::new();
    if !section_items.is_empty() {
        sections.push(StructuredSection {
            title: "Wasm Sections".to_owned(),
            items: section_items,
        });
    }

    Some(StructuredPreviewMeta {
        format: "WebAssembly".to_owned(),
        summary: format!(
            "Wasm v{version} · {} custom · {}",
            custom_sections.len(),
            ByteSize(bytes.len() as u64)
        ),
        fields: vec![
            field("Format", "WebAssembly Binary (wasm)"),
            field("Binary Version", version.to_string()),
            field("Total Bytes", ByteSize(bytes.len() as u64).to_string()),
        ],
        sections,
    })
}

fn inspect_sqlite(bytes: &[u8], total_size: u64) -> StructuredPreviewMeta {
    let raw_page_size = read_u16_be(bytes, 16).unwrap_or(0);
    let page_size: u32 = if raw_page_size == 1 {
        65_536
    } else {
        u32::from(raw_page_size)
    };
    let write_ver = match bytes.get(18).copied().unwrap_or(1) {
        1 => "Legacy (rollback journal)",
        2 => "WAL (Write-Ahead Log)",
        _ => "Unknown",
    };
    let page_count = read_u32_be(bytes, 28).unwrap_or(0);
    let freelist_pages = read_u32_be(bytes, 36).unwrap_or(0);
    let schema_cookie = read_u32_be(bytes, 40).unwrap_or(0);
    let schema_format = read_u32_be(bytes, 44).unwrap_or(0);
    let encoding = match read_u32_be(bytes, 56).unwrap_or(1) {
        1 => "UTF-8",
        2 => "UTF-16le",
        3 => "UTF-16be",
        _ => "Unknown",
    };
    let user_version = read_u32_be(bytes, 60).unwrap_or(0);
    let sqlite_ver = read_u32_be(bytes, 96).unwrap_or(0);
    let sqlite_ver_str = format!(
        "{}.{}.{}",
        sqlite_ver / 1_000_000,
        (sqlite_ver / 1_000) % 1_000,
        sqlite_ver % 1_000
    );

    StructuredPreviewMeta {
        format: "SQLite 3 Database".to_owned(),
        summary: format!(
            "{page_count} pages × {page_size} B · {encoding} · {}",
            ByteSize(total_size)
        ),
        fields: vec![
            field("Page Size", format!("{page_size} bytes")),
            field("Page Count", page_count.to_string()),
            field("Freelist Pages", freelist_pages.to_string()),
            field("Journal Mode", write_ver),
            field("Text Encoding", encoding),
            field(
                "Schema Version",
                format!("{schema_cookie} (format {schema_format})"),
            ),
            field("User Version", user_version.to_string()),
            field("SQLite Library", sqlite_ver_str),
        ],
        sections: Vec::new(),
    }
}

fn inspect_cafebabe(bytes: &[u8], path: &str) -> Option<StructuredPreviewMeta> {
    let major = read_u16_be(bytes, 6)?;
    let minor = read_u16_be(bytes, 4)?;
    let nfat = read_u32_be(bytes, 4)?;

    // Java class files have major >= 45; Mach-O universal FAT headers store nfat_arch (1..=16).
    let is_class_ext = lower_extension(path).as_deref() == Some("class");
    if is_class_ext || (major >= 45 && nfat > 16) {
        let java_release = match major {
            45..=48 => format!("Java 1.{}", major - 44),
            49..=70 => format!("Java {}", major - 44),
            other => format!("Bytecode v{other}.{minor}"),
        };
        let cp_count = read_u16_be(bytes, 8).unwrap_or(0);
        return Some(StructuredPreviewMeta {
            format: "Java Class File".to_owned(),
            summary: format!("{java_release} (class {major}.{minor})"),
            fields: vec![
                field("Target Runtime", java_release),
                field("Class Version", format!("{major}.{minor}")),
                field("Constant Pool", format!("{cp_count} entries")),
            ],
            sections: Vec::new(),
        });
    }

    let slice_count = usize::try_from(nfat).ok()?;
    if (1..=16).contains(&slice_count) && bytes.len() >= 8 + slice_count * 20 {
        let mut slices = Vec::with_capacity(slice_count);
        for i in 0..slice_count {
            let base = 8 + i * 20;
            let cpu = read_u32_be(bytes, base)?;
            let offset = read_u32_be(bytes, base + 8)?;
            let size = read_u32_be(bytes, base + 12)?;
            slices.push(format!(
                "{:<16} offset 0x{offset:x}, {}",
                macho_cpu_name(cpu),
                ByteSize(u64::from(size))
            ));
        }
        return Some(StructuredPreviewMeta {
            format: "Mach-O Universal Binary".to_owned(),
            summary: format!("{slice_count} architecture slices"),
            fields: vec![field("Format", "Apple Fat / Universal Binary")],
            sections: vec![StructuredSection {
                title: "Architecture Slices".to_owned(),
                items: slices,
            }],
        });
    }

    None
}

fn macho_cpu_name(cputype: u32) -> &'static str {
    match cputype {
        0x0100_0007 => "x86_64",
        0x0100_000c => "arm64",
        0x0200_000c => "arm64_32",
        0x0000_0007 => "x86 (i386)",
        0x0000_000c => "ARM",
        _ => "Unknown CPU",
    }
}

fn inspect_macho(bytes: &[u8]) -> Option<StructuredPreviewMeta> {
    let little = matches!(
        &bytes[0..4],
        [0xcf, 0xfa, 0xed, 0xfe] | [0xce, 0xfa, 0xed, 0xfe]
    );
    let is_64 = matches!(
        &bytes[0..4],
        [0xcf, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xcf]
    );
    let read32 = |offset: usize| {
        if little {
            read_u32_le(bytes, offset)
        } else {
            read_u32_be(bytes, offset)
        }
    };
    let cpu = macho_cpu_name(read32(4)?);
    let filetype = match read32(12)? {
        1 => "MH_OBJECT (Relocatable)",
        2 => "MH_EXECUTE (Executable)",
        6 => "MH_DYLIB (Shared Library)",
        7 => "MH_DYLINKER (Dynamic Linker)",
        8 => "MH_BUNDLE (Loadable Bundle)",
        _ => "Mach-O",
    };
    let ncmds = read32(16)?;
    let class = if is_64 { "64-bit" } else { "32-bit" };
    Some(StructuredPreviewMeta {
        format: "Mach-O Binary".to_owned(),
        summary: format!("{class} {cpu} · {filetype}"),
        fields: vec![
            field("Architecture", format!("{cpu} ({class})")),
            field("File Type", filetype),
            field("Load Commands", ncmds.to_string()),
        ],
        sections: Vec::new(),
    })
}

fn inspect_font(path: &str, bytes: &[u8]) -> Option<StructuredPreviewMeta> {
    if bytes.len() < 12 {
        return None;
    }
    let ext = lower_extension(path);
    let (format, is_woff2_or_1) = match &bytes[0..4] {
        b"wOFF" => ("WOFF Web Font", true),
        b"wOF2" => ("WOFF2 Web Font", true),
        b"OTTO" => ("OpenType Font (CFF)", false),
        b"ttcf" => ("TrueType Collection (TTC)", false),
        [0x00, 0x01, 0x00, 0x00] if matches!(ext.as_deref(), Some("ttf" | "ttc" | "otf")) => {
            ("TrueType Font (TTF)", false)
        }
        _ => return None,
    };

    let mut fields = vec![field("Format", format)];
    if is_woff2_or_1 && bytes.len() >= 16 {
        let num_tables = read_u16_be(bytes, 12).unwrap_or(0);
        let orig_size = read_u32_be(bytes, 16).unwrap_or(0);
        fields.push(field("SFNT Tables", num_tables.to_string()));
        if orig_size > 0 {
            fields.push(field(
                "Uncompressed SFNT",
                ByteSize(u64::from(orig_size)).to_string(),
            ));
        }
    } else {
        let num_tables = read_u16_be(bytes, 4).unwrap_or(0);
        fields.push(field("SFNT Tables", num_tables.to_string()));
    }

    Some(StructuredPreviewMeta {
        format: format.to_owned(),
        summary: format!("{format} · {}", ByteSize(bytes.len() as u64)),
        fields,
        sections: Vec::new(),
    })
}

fn inspect_pdf(bytes: &[u8]) -> Option<StructuredPreviewMeta> {
    if !bytes.starts_with(b"%PDF-") {
        return None;
    }
    let first_line_end = bytes
        .iter()
        .take(24)
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or_else(|| bytes.len().min(16));
    let version = from_utf8(&bytes[5..first_line_end]).unwrap_or("1.x").trim();
    Some(StructuredPreviewMeta {
        format: "PDF Document".to_owned(),
        summary: format!("PDF {version} · {}", ByteSize(bytes.len() as u64)),
        fields: vec![
            field("Format", "Portable Document Format (PDF)"),
            field("Version", format!("PDF {version}")),
        ],
        sections: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::{Builder as TarBuilder, Header as TarHeader};

    use super::*;

    fn minimal_png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        out.extend_from_slice(&13_u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    #[test]
    fn inspects_png_gif_webp_and_svg() {
        let png = minimal_png(128, 64);
        match inspect_file_preview("static/assets/icon.png", &png, png.len() as u64) {
            InspectedPreview::Image { meta, truncated } => {
                assert_eq!(meta.format, "PNG");
                assert_eq!(meta.mime, "image/png");
                assert_eq!(meta.width, Some(128));
                assert_eq!(meta.height, Some(64));
                assert_eq!(meta.detail.as_deref(), Some("8-bit RGBA"));
                assert!(!truncated);
            }
            other => panic!("expected PNG image preview, got {other:?}"),
        }

        let gif = b"GIF89a\x20\x00\x10\x00\x80\x00\x00";
        match inspect_file_preview("anim.gif", gif, gif.len() as u64) {
            InspectedPreview::Image { meta, .. } => {
                assert_eq!(meta.format, "GIF");
                assert_eq!(meta.width, Some(32));
                assert_eq!(meta.height, Some(16));
            }
            other => panic!("expected GIF image preview, got {other:?}"),
        }

        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="48" height="48"><circle cx="12" cy="12" r="10"/></svg>"#;
        match inspect_file_preview("icons/logo.svg", svg, svg.len() as u64) {
            InspectedPreview::Svg { meta, source, .. } => {
                assert_eq!(meta.format, "SVG");
                assert_eq!(meta.width, Some(48));
                assert_eq!(meta.height, Some(48));
                assert!(source.contains("<circle"));
            }
            other => panic!("expected SVG preview, got {other:?}"),
        }
    }

    #[test]
    fn inspects_gzipped_text_and_tar_gz_archives() {
        let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
        gz_encoder
            .write_all(br#"{"version":"2.4.1","env":"prod"}"#)
            .unwrap();
        let gz_bytes = gz_encoder.finish().unwrap();

        match inspect_file_preview("static/version.json.gz", &gz_bytes, gz_bytes.len() as u64) {
            InspectedPreview::Text {
                formatted_text,
                language,
                compression,
                ..
            } => {
                assert_eq!(language, Some("json"));
                assert_eq!(compression, Some("gzip"));
                assert!(formatted_text
                    .as_deref()
                    .unwrap()
                    .contains("\"version\": \"2.4.1\""));
            }
            other => panic!("expected decompressed JSON text, got {other:?}"),
        }

        let tar_gz_bytes = {
            let enc = GzEncoder::new(Vec::new(), Compression::default());
            let mut tar = TarBuilder::new(enc);
            let payload = b"console.log('ok');\n";
            let mut hdr = TarHeader::new_gnu();
            hdr.set_size(payload.len() as u64);
            hdr.set_mode(0o644);
            hdr.set_uid(0);
            hdr.set_gid(0);
            hdr.set_cksum();
            tar.append_data(&mut hdr, "bundle/app.js", &payload[..])
                .unwrap();
            let enc = tar.into_inner().unwrap();
            enc.finish().unwrap()
        };

        match inspect_file_preview(
            "dist/bundle.tar.gz",
            &tar_gz_bytes,
            tar_gz_bytes.len() as u64,
        ) {
            InspectedPreview::Archive { meta } => {
                assert_eq!(meta.format, "TAR.GZ");
                assert_eq!(meta.total_entries, 1);
                assert_eq!(meta.entries[0].path, "bundle/app.js");
                assert_eq!(meta.entries[0].size, 19);
            }
            other => panic!("expected TAR.GZ archive preview, got {other:?}"),
        }
    }

    #[test]
    fn inspects_zip_archive_central_directory() {
        let mut bytes = Vec::new();
        let name = b"META-INF/MANIFEST.MF";
        let payload = b"Manifest-Version: 1.0\n";
        // Local File Header
        bytes.extend_from_slice(&ZIP_LFH_SIG);
        bytes.extend_from_slice(&20_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(payload);

        let cd_offset = bytes.len() as u32;
        bytes.extend_from_slice(&ZIP_CD_SIG);
        bytes.extend_from_slice(&[20, 3]); // Unix creator
        bytes.extend_from_slice(&20_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&(0o100644_u32 << 16).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(name);
        let cd_size = (bytes.len() as u32) - cd_offset;

        bytes.extend_from_slice(&ZIP_EOCD_SIG);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&cd_size.to_le_bytes());
        bytes.extend_from_slice(&cd_offset.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());

        match inspect_file_preview("app/service.jar", &bytes, bytes.len() as u64) {
            InspectedPreview::Archive { meta } => {
                assert_eq!(meta.format, "JAR");
                assert_eq!(meta.total_entries, 1);
                assert_eq!(meta.entries[0].path, "META-INF/MANIFEST.MF");
                assert_eq!(meta.entries[0].mode, "-rw-r--r--");
            }
            other => panic!("expected JAR archive preview, got {other:?}"),
        }
    }

    #[test]
    fn inspects_elf_wasm_and_sqlite_headers() {
        let mut elf_hdr = [0_u8; 64];
        elf_hdr[0..4].copy_from_slice(b"\x7fELF");
        elf_hdr[4] = 2; // ELFCLASS64
        elf_hdr[5] = 1; // ELFDATA2LSB
        elf_hdr[6] = 1; // EV_CURRENT
        elf_hdr[7] = 3; // GNU/Linux
        elf_hdr[16..18].copy_from_slice(&3_u16.to_le_bytes()); // ET_DYN
        elf_hdr[18..20].copy_from_slice(&62_u16.to_le_bytes()); // EM_X86_64
        elf_hdr[20..24].copy_from_slice(&1_u32.to_le_bytes());
        elf_hdr[24..32].copy_from_slice(&0x401000_u64.to_le_bytes());

        match inspect_file_preview("usr/bin/server", &elf_hdr, 4096) {
            InspectedPreview::Structured { meta } => {
                assert!(meta.format.contains("ELF"));
                assert!(meta.summary.contains("x86_64"));
            }
            other => panic!("expected ELF structured preview, got {other:?}"),
        }

        let wasm = b"\x00asm\x01\x00\x00\x00\x01\x04\x01\x60\x00\x00";
        match inspect_file_preview("app/worker.wasm", wasm, wasm.len() as u64) {
            InspectedPreview::Structured { meta } => {
                assert_eq!(meta.format, "WebAssembly");
                assert!(meta.sections[0].items[0].contains("Type"));
            }
            other => panic!("expected Wasm preview, got {other:?}"),
        }

        let mut sqlite = [0_u8; 100];
        sqlite[0..16].copy_from_slice(b"SQLite format 3\x00");
        sqlite[16..18].copy_from_slice(&4096_u16.to_be_bytes());
        sqlite[28..32].copy_from_slice(&12_u32.to_be_bytes());
        sqlite[56..60].copy_from_slice(&1_u32.to_be_bytes());
        sqlite[96..100].copy_from_slice(&3_045_001_u32.to_be_bytes());
        match inspect_file_preview("var/lib/data.db", &sqlite, 49152) {
            InspectedPreview::Structured { meta } => {
                assert_eq!(meta.format, "SQLite 3 Database");
                assert!(meta.summary.contains("12 pages"));
            }
            other => panic!("expected SQLite preview, got {other:?}"),
        }
    }

    #[test]
    fn leaves_tiny_unknown_binary_and_audio_correctly_classified() {
        match inspect_file_preview("/bin/app", b"\x7fELF\x00\x01", 4096) {
            InspectedPreview::Binary { .. } => {}
            other => panic!("expected raw Binary for 6-byte ELF stub, got {other:?}"),
        }

        let mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x00";
        match inspect_file_preview("static/assets/mp3/beijiaosuo.mp3", mp3, 1024) {
            InspectedPreview::Media { meta, .. } => {
                assert_eq!(meta.media_type, "audio");
                assert_eq!(meta.format, "MP3");
                assert_eq!(meta.mime, "audio/mpeg");
            }
            other => panic!("expected MP3 media preview, got {other:?}"),
        }
    }

    #[test]
    fn beautifies_minified_js_css_and_svg_while_preserving_normal_code() {
        let min_js = br#"import{a as b}from"./c.js";const pat=/https?:\/\/a;b\/c/g;const tpl=`x=${({k:1})};`;for(let i=0;i<3;i++){console.log("{;}",pat,tpl)}const arr=[1,2].map(x=>{const y=x*2;return y});export{arr};"#;
        match inspect_file_preview("static/account.module.hash.js", min_js, min_js.len() as u64) {
            InspectedPreview::Text {
                text,
                formatted_text,
                language,
                ..
            } => {
                assert_eq!(language, Some("javascript"));
                assert_eq!(text.as_bytes(), min_js);
                let formatted = formatted_text.expect("minified JS must auto-format");
                assert!(formatted.contains('\n'));
                assert!(formatted.contains("/https?:\\/\\/a;b\\/c/g"));
                assert!(formatted.contains("`x=${"));
                assert!(
                    formatted.contains("for(let i=0; i<3; i++) {")
                        || formatted.contains("for (let i=0")
                        || formatted.contains("for(let i=0;")
                );
                assert!(formatted.contains("\n  const y=x*2;"));
            }
            other => panic!("expected minified JS Text preview, got {other:?}"),
        }

        let normal_js =
            b"export function greet(name) {\n  const msg = `Hello, ${name}`;\n  return msg;\n}\n";
        match inspect_file_preview("src/greet.js", normal_js, normal_js.len() as u64) {
            InspectedPreview::Text { formatted_text, .. } => {
                assert_eq!(formatted_text, None);
            }
            other => panic!("expected normal JS Text preview, got {other:?}"),
        }

        let min_css = br#".card{display:flex;background:url(data:image/png;base64,iVBOR;w0KGgo);color:#fff}@media(min-width:768px){.card{padding:12px 16px;margin:0 auto}}.btn:hover{opacity:.85}"#;
        match inspect_file_preview("static/styles/main.css", min_css, min_css.len() as u64) {
            InspectedPreview::Text {
                text,
                formatted_text,
                language,
                ..
            } => {
                assert_eq!(language, Some("css"));
                assert_eq!(text.as_bytes(), min_css);
                let formatted = formatted_text.expect("minified CSS must auto-format");
                assert!(formatted.contains("url(data:image/png;base64,iVBOR;w0KGgo);"));
                assert!(formatted.contains("  display: flex;"));
                assert!(formatted.contains("\n    padding: 12px 16px;"));
                assert!(formatted.contains("\n  }\n}"));
            }
            other => panic!("expected minified CSS Text preview, got {other:?}"),
        }

        let min_svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><g fill="none"><rect width="24" height="24"/><circle cx="12" cy="12" r="8"/><text x="2" y="2">OK</text></g></svg>"#;
        match inspect_file_preview("static/assets/badge.svg", min_svg, min_svg.len() as u64) {
            InspectedPreview::Svg {
                source,
                formatted_source,
                ..
            } => {
                assert_eq!(source.as_bytes(), min_svg);
                let formatted = formatted_source.expect("minified SVG must auto-format");
                assert!(formatted.contains("\n  <g fill=\"none\">"));
                assert!(formatted.contains("<text x=\"2\" y=\"2\">OK</text>"));
            }
            other => panic!("expected minified SVG preview, got {other:?}"),
        }
    }
}
