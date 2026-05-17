//! Direct AI-assisted image optimization analysis.
//!
//! Sends the full Markdown analysis of an image to an OpenAI-compatible
//! `/chat/completions` endpoint and prints the model's diagnostic report.
//! The previous snapshot of the same image (kept under
//! `~/.diving/ai_history/`) is included so the model can perform bloat /
//! regression comparison ("防劣化").

use crate::config;
use crate::i18n::{self, Lang};
use serde::{Deserialize, Serialize};
use std::fs;
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
}

impl AiConfig {
    /// Resolve config from explicit CLI args, falling back to the standard
    /// OpenAI env vars. Returns `None` when no API key is available, leaving
    /// the feature off (the normal terminal/TUI path is used instead).
    pub fn resolve(
        api_key: Option<&str>,
        base_url: Option<&str>,
        model: Option<&str>,
    ) -> Option<AiConfig> {
        let api_key = first_non_empty(api_key, "OPENAI_API_KEY")?;
        let base_url =
            first_non_empty(base_url, "OPENAI_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.into());
        let model = first_non_empty(model, "OPENAI_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into());
        Some(AiConfig {
            api_key,
            base_url,
            model,
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

// Fixed DevSecOps expert persona + rules + output format (verbatim, Chinese).
// The model is expected to answer in the same language as this prompt.
const SYSTEM_PROMPT: &str = r#"你现在是一位极其务实、精通 Docker 底层架构（特别是 OverlayFS 分层文件系统）的 DevSecOps 资深专家。

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

fn build_user_message(prev: Option<&str>, current: &str) -> String {
    match prev {
        Some(p) => {
            format!("【上一次的分析记录】\n{p}\n\n【本次的分析记录】\n{current}")
        }
        None => format!("（无上一次的分析记录）\n\n【本次的分析记录】\n{current}"),
    }
}

fn history_dir() -> PathBuf {
    config::get_config_path().join("ai_history")
}

/// Map an image reference to a safe, collision-resistant filename.
fn history_key(image: &str) -> String {
    let mut key: String = image
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
/// OpenAI-compatible endpoint and return the model's report. The snapshot is
/// refreshed with the current analysis so the next run can diff against it.
pub async fn analyze_with_ai(
    image: &str,
    current_md: &str,
    cfg: &AiConfig,
    lang: Lang,
) -> Result<String, String> {
    // Best-effort history: read the prior snapshot, then overwrite it so the
    // next run compares against this analysis. History I/O never aborts.
    let prev = read_history(image);
    write_history(image, current_md);

    let user_content = build_user_message(prev.as_deref(), current_md);

    eprintln!("{}", i18n::tr(lang, "ai.analyzing"));
    if prev.is_some() {
        eprintln!("{}", i18n::tr(lang, "ai.compare"));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;

    let body = ChatRequest {
        model: &cfg.model,
        messages: vec![
            ChatMessage {
                role: "system",
                content: SYSTEM_PROMPT,
            },
            ChatMessage {
                role: "user",
                content: &user_content,
            },
        ],
        temperature: 0.2,
        stream: false,
    };

    let resp = client
        .post(cfg.endpoint())
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
    Ok(content)
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
        };
        assert_eq!(cfg.endpoint(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn endpoint_keeps_full_endpoint_verbatim() {
        let cfg = AiConfig {
            api_key: "k".into(),
            base_url: "https://host/v1/chat/completions".into(),
            model: "m".into(),
        };
        assert_eq!(cfg.endpoint(), "https://host/v1/chat/completions");
    }

    #[test]
    fn resolve_uses_explicit_values_without_touching_env() {
        let cfg = AiConfig::resolve(Some("sk-x"), Some("https://h/v1"), Some("gpt-test"))
            .expect("explicit key resolves");
        assert_eq!(cfg.api_key, "sk-x");
        assert_eq!(cfg.base_url, "https://h/v1");
        assert_eq!(cfg.model, "gpt-test");
        assert_eq!(cfg.endpoint(), "https://h/v1/chat/completions");
    }

    #[test]
    fn history_key_sanitizes_unsafe_chars() {
        assert_eq!(history_key("redis:alpine"), "redis_alpine.md");
        assert_eq!(
            history_key("registry.example.com/user/img:v1?arch=arm64"),
            "registry.example.com_user_img_v1_arch_arm64.md"
        );
        assert_eq!(history_key(""), "_.md");
    }

    #[test]
    fn user_message_marks_presence_of_prior_record() {
        let with_prev = build_user_message(Some("OLD"), "NEW");
        assert!(with_prev.contains("【上一次的分析记录】"));
        assert!(with_prev.contains("OLD"));
        assert!(with_prev.contains("【本次的分析记录】"));

        let no_prev = build_user_message(None, "NEW");
        assert!(no_prev.contains("（无上一次的分析记录）"));
        assert!(!no_prev.contains("【上一次的分析记录】"));
    }
}
