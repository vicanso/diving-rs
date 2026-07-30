//! Direct AI-assisted image optimization analysis.
//!
//! Sends the full Markdown analysis of an image to an OpenAI-compatible
//! `/chat/completions` endpoint and prints the model's diagnostic report.
//! The previous snapshot of the same image (kept under
//! `~/.diving/ai_history/`) is included so the model can perform bloat /
//! regression comparison ("防劣化").

use crate::config;
use crate::i18n::{self, Lang};
use crate::image::{get_file_content_from_layer, parse_image_info, DockerAnalyzeResult};
use crate::store::get_blob_path;
use crate::util::get_http_client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;
use tracing::warn;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o";

/// Resolved AI endpoint configuration. AI analysis is enabled only when an
/// API key is present (CLI arg or `OPENAI_API_KEY`).
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// User-supplied override for the system prompt. When `Some`, it fully
    /// replaces the built-in zh/en DevSecOps template; when `None`, the
    /// built-in `system_prompt(lang)` is used.
    pub system_prompt: Option<String>,
}

impl AiConfig {
    /// Resolve config from explicit CLI args, falling back to the standard
    /// OpenAI env vars. Returns `None` when no API key is available, leaving
    /// the feature off (the normal terminal/TUI path is used instead).
    pub fn resolve(
        api_key: Option<&str>,
        base_url: Option<&str>,
        model: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Option<AiConfig> {
        let api_key = first_non_empty(api_key, "OPENAI_API_KEY")?;
        let base_url =
            first_non_empty(base_url, "OPENAI_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.into());
        let model = first_non_empty(model, "OPENAI_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into());
        let system_prompt = first_non_empty(system_prompt, "OPENAI_SYSTEM_PROMPT");
        Some(AiConfig {
            api_key,
            base_url,
            model,
            system_prompt,
        })
    }

    /// The chat-completions endpoint. If the user already passed a full
    /// endpoint we use it verbatim; otherwise append the standard path.
    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        }
    }
}

fn first_non_empty(explicit: Option<&str>, env_key: &str) -> Option<String> {
    if let Some(v) = explicit {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Ok(v) = std::env::var(env_key) {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

// Fixed DevSecOps expert persona + rules + output format. The model answers in
// the same language as the prompt, so we pick the prompt by resolved `Lang`.
fn system_prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => SYSTEM_PROMPT_ZH,
        Lang::En => SYSTEM_PROMPT_EN,
    }
}

const SYSTEM_PROMPT_ZH: &str = r#"你现在是一位极其务实、精通 Docker 底层架构（特别是 OverlayFS 分层文件系统）的 DevSecOps 资深专家。

我将为你提供同一 Docker 镜像的深度分析数据。请严格遵循“异常驱动”与“防劣化”原则进行对比诊断。

【⚠️ 绝对执行的分析铁律 (CRITICAL RULES)】
1. 极度静默：表现良好的指标（如体积减小或持平、浪费率为 0%、非 root 运行、无密钥暴露）绝对禁止提及。不写任何“未发现问题”、“表现优秀”的废话。
2. 体积劣化追踪（Bloat Detection）：若有上一次的分析记录则要严格对比新老版本的体积大小。如果新版本发生体积异常膨胀，必须精准穿透至引发膨胀的具体 Layer 和指令，揪出占用空间的具体大文件（如：错误引入的编译链、未忽略的 .git 目录、静态资源冗余等）。
3. OverlayFS 穿透判定（防伪优化）：
   - 遇到包管理器缓存或“Wasted space”警告时，必须查验自定义层。
   - 如果自定义层仅有 COPY 等指令，未执行包管理器操作，说明冗余 100% 来自基础镜像。此时**绝对禁止**建议追加跨层的 `RUN rm -rf`！
   - 针对基础镜像的固化冗余，唯一建议是“更换精简基底”，或标注“可接受的基础镜像原生冗余”。
4. 权限对齐检查：对比新旧版本，严查新引入的 `COPY/ADD` 文件所有权（Owner）是否与运行时用户（User）存在倒挂风险。

请严格按以下精简格式输出报告（如果未发现体积劣化、且无需要动手修复的真实异常，请直接回复：“🟢 镜像无劣化，健康通过”）：

### 🚨 核心异常与劣化痛点
- [精确指出体积膨胀的具体数值及肇事 Layer/文件，或其他新增的安全异常。一句废话不要有]

### 🛠️ 必须执行的修复代码
- [给出优化后的 Dockerfile 片段，只写需要改动、增加 .dockerignore 或清理的那几行]"#;

const SYSTEM_PROMPT_EN: &str = r#"You are an extremely pragmatic senior DevSecOps expert with deep mastery of Docker's underlying architecture (especially the OverlayFS layered filesystem).

I will give you in-depth analysis data for the same Docker image. Perform a comparative diagnosis strictly following the "anomaly-driven" and "anti-regression" principles.

[⚠️ ABSOLUTELY ENFORCED ANALYSIS RULES (CRITICAL RULES)]
1. Extreme silence: Metrics that look good (size shrank or stayed flat, 0% waste, non-root runtime, no secret exposure) MUST NEVER be mentioned. Do not write any filler such as "no issues found" or "looks great".
2. Bloat Detection: If a previous analysis record exists, rigorously compare the size of the new vs. old version. If the new version shows abnormal size growth, you must precisely penetrate to the exact Layer and instruction that caused it, and pin down the specific large files eating the space (e.g. a wrongly introduced build toolchain, an un-ignored .git directory, redundant static assets, etc.).
3. OverlayFS penetration judgment (anti-fake-optimization):
   - When you see a package-manager cache or a "Wasted space" warning, you MUST inspect the custom layers.
   - If the custom layers contain only COPY and similar instructions and run no package-manager operations, the redundancy comes 100% from the base image. In that case it is **absolutely forbidden** to suggest adding a cross-layer `RUN rm -rf`!
   - For redundancy baked into the base image, the only valid advice is "switch to a slimmer base", or label it "acceptable native redundancy of the base image".
4. Permission alignment check: Comparing the old and new versions, strictly check whether the ownership (Owner) of newly introduced `COPY/ADD` files is inverted against the runtime user (User).

Output the report strictly in the concise format below (if no size regression is found and there is no real anomaly that needs a hands-on fix, reply with exactly: "🟢 Image has no regression, healthy and passing"):

### 🚨 Core anomalies & regression pain points
- [State the exact size-growth numbers and the offending Layer/file, or any newly introduced security anomaly. Not one word of filler.]

### 🛠️ Fix code that must be applied
- [Give the optimized Dockerfile snippet — only the lines that must change, the .dockerignore entries to add, or the cleanup steps.]"#;

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatChoiceMsg {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMsg,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

fn build_user_message(lang: Lang, prev: Option<&str>, current: &str) -> String {
    let (prev_hdr, cur_hdr, none_hdr) = match lang {
        Lang::Zh => (
            "【上一次的分析记录】",
            "【本次的分析记录】",
            "（无上一次的分析记录）",
        ),
        Lang::En => (
            "[Previous analysis]",
            "[Current analysis]",
            "(No previous analysis on record)",
        ),
    };
    match prev {
        Some(p) => format!("{prev_hdr}\n{p}\n\n{cur_hdr}\n{current}"),
        None => format!("{none_hdr}\n\n{cur_hdr}\n{current}"),
    }
}

fn history_dir() -> PathBuf {
    config::get_config_path().join("ai_history")
}

/// Map an image reference to a safe, collision-resistant filename.
///
/// The key is the image *identity* — registry / user / name (+ arch) — with
/// the **tag and digest deliberately dropped**. Internal/CI images are
/// retagged on every build (e.g. `…/finance-operations:v1.0.0-05151400648`),
/// so keying on the tag would create a brand-new snapshot every run and the
/// bloat/regression comparison could never trigger. Arch is kept because a
/// different architecture is a genuinely different image, not a regression.
fn history_key(image: &str) -> String {
    let info = parse_image_info(image);
    // Strip an `@sha256:…` digest if it rode along on the name component.
    let name = info.name.split('@').next().unwrap_or(info.name.as_str());
    let mut identity = format!("{}/{}/{}", info.registry, info.user, name);
    if !info.arch.is_empty() {
        identity.push('@');
        identity.push_str(&info.arch);
    }
    let mut key: String = identity
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(180)
        .collect();
    if key.is_empty() {
        key.push('_');
    }
    format!("{key}.md")
}

fn read_history(image: &str) -> Option<String> {
    fs::read_to_string(history_dir().join(history_key(image))).ok()
}

fn write_history(image: &str, md: &str) {
    let dir = history_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        warn!(err = e.to_string(), "create ai_history dir fail");
        return;
    }
    if let Err(e) = fs::write(dir.join(history_key(image)), md) {
        warn!(err = e.to_string(), "write ai_history snapshot fail");
    }
}

/// Send the current analysis (and the previous snapshot, if any) to the
/// OpenAI-compatible endpoint and return the model's report. On success the
/// snapshot is refreshed with the current analysis so the next run can diff
/// against it; on failure the previous baseline is left untouched so the
/// regression comparison is not lost.
///
/// When `skip_history` is set (`--no-ai-history`), the prior snapshot is NOT
/// read, so the model does no regression comparison this run. The current
/// analysis is still written (on success) so later runs have a fresh baseline.
pub async fn analyze_with_ai(
    image: &str,
    current_md: &str,
    cfg: &AiConfig,
    lang: Lang,
    skip_history: bool,
) -> Result<String, String> {
    // Best-effort history read; the snapshot itself is only refreshed after
    // a successful AI response (see the end of this function).
    let prev = if skip_history {
        None
    } else {
        read_history(image)
    };

    let user_content = build_user_message(lang, prev.as_deref(), current_md);

    eprintln!("{}", i18n::tr(lang, "ai.analyzing"));
    if prev.is_some() {
        eprintln!("{}", i18n::tr(lang, "ai.compare"));
    }

    // User override takes precedence; otherwise pick the language-matched
    // built-in DevSecOps template.
    let sys_prompt = cfg
        .system_prompt
        .as_deref()
        .unwrap_or_else(|| system_prompt(lang));
    let body = ChatRequest {
        model: &cfg.model,
        messages: vec![
            ChatMessage {
                role: "system",
                content: sys_prompt,
            },
            ChatMessage {
                role: "user",
                content: &user_content,
            },
        ],
        temperature: 0.2,
        stream: false,
    };

    // Inference can be slow; the per-request timeout overrides the
    // shared client's default-less setting.
    let resp = get_http_client()
        .post(cfg.endpoint())
        .timeout(Duration::from_secs(5 * 60))
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", text.trim()));
    }

    let parsed: ChatResponse =
        serde_json::from_str(&text).map_err(|e| format!("{e}: {}", text.trim()))?;
    let content = parsed
        .choices
        .into_iter()
        .find_map(|c| c.message.content)
        .unwrap_or_default();
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("empty AI response".to_string());
    }
    // 请求成功才刷新快照；失败保留上一次的基线，下次运行仍可做防劣化对比。
    write_history(image, current_md);
    Ok(content)
}

// Per-script and overall caps so a huge startup script can't blow the model's
// context window.
const SCRIPT_MAX_BYTES: usize = 16 * 1024;
const MAX_SCRIPTS: usize = 4;
// 从层里读取候选脚本时的硬上限：真实的启动脚本不会超过 1MB，超过的
// 几乎必然是误判的二进制，直接跳过，不浪费内存去读。
const SCRIPT_READ_CAP: u64 = 1024 * 1024;

// Interpreters/launchers that are not themselves the script of interest — when
// ENTRYPOINT is `["bash", "/app/start.sh"]` we want `start.sh`, not `bash`.
const NON_SCRIPT: &[&str] = &[
    "sh",
    "bash",
    "dash",
    "ash",
    "ksh",
    "zsh",
    "env",
    "exec",
    "tini",
    "/bin/sh",
    "/bin/bash",
    "/usr/bin/env",
    "/sbin/tini",
    "/usr/bin/tini",
];
const SCRIPT_EXTS: &[&str] = &[
    ".sh", ".bash", ".dash", ".ksh", ".py", ".rb", ".pl", ".js", ".ts",
];

fn looks_like_script(tok: &str) -> bool {
    if tok.is_empty() || tok.starts_with('-') || NON_SCRIPT.contains(&tok) {
        return false;
    }
    let lower = tok.to_ascii_lowercase();
    tok.contains('/') || SCRIPT_EXTS.iter().any(|e| lower.ends_with(e))
}

/// Parse a Docker instruction argument list: the JSON exec form `["a","b"]`
/// (what image history records) or a plain shell string.
fn parse_arg_list(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(raw) {
            return v;
        }
    }
    raw.split_whitespace().map(|s| s.to_string()).collect()
}

/// Pull ENTRYPOINT/CMD argument tokens out of the reconstructed Dockerfile.
/// The last ENTRYPOINT and last CMD win (later instructions override earlier).
fn entrypoint_cmd_tokens(dockerfile: &str) -> Vec<String> {
    let mut entry: Option<String> = None;
    let mut cmd: Option<String> = None;
    for line in dockerfile.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("ENTRYPOINT ") {
            entry = Some(rest.trim().to_string());
        } else if let Some(rest) = l.strip_prefix("CMD ") {
            cmd = Some(rest.trim().to_string());
        }
    }
    let mut out = Vec::new();
    for raw in [entry, cmd].into_iter().flatten() {
        out.extend(parse_arg_list(&raw));
    }
    out
}

// A NUL byte in the head strongly implies a compiled binary, not a script.
fn is_probably_text(bytes: &[u8]) -> bool {
    !bytes[..bytes.len().min(8000)].contains(&0)
}

/// Read `token`'s bytes from the cached layer blobs. The topmost layer that
/// contains it wins (later layers override earlier ones). Best-effort: any
/// I/O error just means "not here, keep looking".
fn read_from_layers(result: &DockerAnalyzeResult, token: &str) -> Option<Vec<u8>> {
    let trimmed = token.trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    // `get_file_content_from_layer` 单趟内已同时匹配带/不带 `./` 前缀的
    // 拼写，每层最多解压一次。
    for layer in result.layers.iter().rev() {
        let blob = get_blob_path(&layer.digest);
        if !blob.is_file() {
            continue;
        }
        let Ok(file) = fs::File::open(&blob) else {
            continue;
        };
        if let Ok(bytes) = get_file_content_from_layer(
            BufReader::new(file),
            &layer.media_type,
            trimmed,
            SCRIPT_READ_CAP,
        ) {
            return Some(bytes);
        }
    }
    None
}

/// Locate the ENTRYPOINT/CMD startup script(s), read their content from the
/// layers, and render a Markdown section to append to the AI payload so the
/// model can reason about what the container actually runs. Returns an empty
/// string when nothing script-like can be located. Decompresses layers, so
/// callers should run it off the async path.
pub fn entrypoint_scripts_md(result: &DockerAnalyzeResult, lang: Lang) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut blocks: Vec<String> = Vec::new();
    for tok in entrypoint_cmd_tokens(&result.dockerfile) {
        if blocks.len() >= MAX_SCRIPTS {
            break;
        }
        if !looks_like_script(&tok) || !seen.insert(tok.clone()) {
            continue;
        }
        let Some(bytes) = read_from_layers(result, &tok) else {
            continue;
        };
        if !is_probably_text(&bytes) {
            continue;
        }
        let truncated = bytes.len() > SCRIPT_MAX_BYTES;
        let slice = &bytes[..bytes.len().min(SCRIPT_MAX_BYTES)];
        let mut body = String::from_utf8_lossy(slice).into_owned();
        if truncated {
            body.push('\n');
            body.push_str(i18n::tr(lang, "ai.script.truncated"));
        }
        blocks.push(format!("### `{tok}`\n\n```sh\n{body}\n```"));
    }
    if blocks.is_empty() {
        return String::new();
    }
    format!(
        "## {}\n\n{}",
        i18n::tr(lang, "ai.script.title"),
        blocks.join("\n\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_appends_standard_path() {
        let cfg = AiConfig {
            api_key: "k".into(),
            base_url: "https://api.openai.com/v1/".into(),
            model: "m".into(),
            system_prompt: None,
        };
        assert_eq!(cfg.endpoint(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn endpoint_keeps_full_endpoint_verbatim() {
        let cfg = AiConfig {
            api_key: "k".into(),
            base_url: "https://host/v1/chat/completions".into(),
            model: "m".into(),
            system_prompt: None,
        };
        assert_eq!(cfg.endpoint(), "https://host/v1/chat/completions");
    }

    #[test]
    fn resolve_uses_explicit_values_without_touching_env() {
        let cfg = AiConfig::resolve(Some("sk-x"), Some("https://h/v1"), Some("gpt-test"), None)
            .expect("explicit key resolves");
        assert_eq!(cfg.api_key, "sk-x");
        assert_eq!(cfg.base_url, "https://h/v1");
        assert_eq!(cfg.model, "gpt-test");
        assert_eq!(cfg.endpoint(), "https://h/v1/chat/completions");
    }

    #[test]
    fn history_key_drops_tag_and_sanitizes() {
        // Tag dropped; key is the registry/repo identity, path-safe.
        assert_eq!(
            history_key("redis:alpine"),
            "https___index.docker.io_v2_library_redis.md"
        );
        // Private registry + arch kept, tag dropped.
        assert_eq!(
            history_key("registry.example.com/user/img:v1?arch=arm64"),
            "https___registry.example.com_v2_user_img_arm64.md"
        );
        // Degenerate input still yields a safe, non-empty name.
        assert_eq!(history_key(""), "https___index.docker.io_v2_library_.md");
    }

    #[test]
    fn history_key_is_stable_across_changing_tags() {
        // The reported case: an internal image retagged every CI build must
        // map to ONE snapshot so successive builds remain comparable.
        let a = history_key("dockertest.gf.com.cn/gfstore/finance-operations:v1.0.0-05151400648");
        let b = history_key("dockertest.gf.com.cn/gfstore/finance-operations:v2.3.1-09301122334");
        assert_eq!(a, b);
        assert!(!a.contains("05151400648"));
        assert_eq!(
            a,
            "https___dockertest.gf.com.cn_v2_gfstore_finance-operations.md"
        );
    }

    #[test]
    fn user_message_marks_presence_of_prior_record() {
        let with_prev = build_user_message(Lang::Zh, Some("OLD"), "NEW");
        assert!(with_prev.contains("【上一次的分析记录】"));
        assert!(with_prev.contains("OLD"));
        assert!(with_prev.contains("【本次的分析记录】"));

        let no_prev = build_user_message(Lang::Zh, None, "NEW");
        assert!(no_prev.contains("（无上一次的分析记录）"));
        assert!(!no_prev.contains("【上一次的分析记录】"));
    }

    #[test]
    fn user_message_is_localized_for_english() {
        let with_prev = build_user_message(Lang::En, Some("OLD"), "NEW");
        assert!(with_prev.contains("[Previous analysis]"));
        assert!(with_prev.contains("OLD"));
        assert!(with_prev.contains("[Current analysis]"));
        assert!(!with_prev.contains("【本次的分析记录】"));

        let no_prev = build_user_message(Lang::En, None, "NEW");
        assert!(no_prev.contains("(No previous analysis on record)"));
        assert!(!no_prev.contains("[Previous analysis]"));
    }

    #[test]
    fn system_prompt_is_selected_by_language() {
        assert!(system_prompt(Lang::Zh).contains("DevSecOps 资深专家"));
        assert!(system_prompt(Lang::En).contains("senior DevSecOps expert"));
        assert!(system_prompt(Lang::En).contains("🟢 Image has no regression"));
    }

    #[test]
    fn resolve_picks_up_explicit_system_prompt() {
        let cfg = AiConfig::resolve(Some("KEY"), None, None, Some("CUSTOM PROMPT"))
            .expect("api_key was provided");
        assert_eq!(cfg.system_prompt.as_deref(), Some("CUSTOM PROMPT"));
    }

    #[test]
    fn resolve_trims_and_rejects_blank_system_prompt() {
        // Empty / whitespace-only override is treated as "not provided",
        // so the built-in language template wins later in analyze_with_ai.
        let cfg =
            AiConfig::resolve(Some("KEY"), None, None, Some("   ")).expect("api_key was provided");
        assert!(cfg.system_prompt.is_none());
    }

    #[test]
    fn parse_arg_list_handles_exec_and_shell_form() {
        assert_eq!(
            parse_arg_list(r#"["docker-entrypoint.sh", "redis-server"]"#),
            vec!["docker-entrypoint.sh", "redis-server"]
        );
        assert_eq!(
            parse_arg_list("nginx -g daemon off;"),
            vec!["nginx", "-g", "daemon", "off;"]
        );
    }

    #[test]
    fn entrypoint_cmd_tokens_last_instruction_wins() {
        let df = "FROM x\n\
                  ENTRYPOINT [\"/old.sh\"]\n\
                  CMD [\"a\"]\n\
                  ENTRYPOINT [\"docker-entrypoint.sh\"]\n\
                  CMD [\"redis-server\"]";
        assert_eq!(
            entrypoint_cmd_tokens(df),
            vec!["docker-entrypoint.sh", "redis-server"]
        );
    }

    #[test]
    fn looks_like_script_filters_interpreters_and_flags() {
        assert!(looks_like_script("/usr/local/bin/docker-entrypoint.sh"));
        assert!(looks_like_script("entrypoint.sh"));
        assert!(looks_like_script("/app/run"));
        assert!(!looks_like_script("redis-server"));
        assert!(!looks_like_script("bash"));
        assert!(!looks_like_script("/bin/sh"));
        assert!(!looks_like_script("-c"));
        assert!(!looks_like_script(""));
    }

    #[test]
    fn binary_content_is_rejected() {
        assert!(is_probably_text(b"#!/bin/sh\nexec \"$@\"\n"));
        assert!(!is_probably_text(b"\x7fELF\x00\x01binary"));
    }
}
