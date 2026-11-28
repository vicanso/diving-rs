use std::env;
use std::fs;
use std::io::{self, Write};
use std::sync::Arc;

use bytesize::ByteSize;
use clap::{CommandFactory, Parser, Subcommand};
use colored::Colorize;
use diving_app::ai::{analyze_with_ai, entrypoint_scripts_md, AiConfig};
use diving_app::config::LoadedConfig;
use diving_app::logging::{EXIT_ERROR, EXIT_GATE_FAILED};
use diving_app::notify::{build_report_url, send_all, NotifyConfig, NotifyContext, NotifyInputs};
use diving_app::summary::wecom_summary;
use diving_core::trace::{generate_trace_id, TRACE_ID};
use diving_core::{
    evaluate_diff_gate, evaluate_gate, fill, gate_verdict_md, resolve_explicit_credentials,
    to_diff_markdown, to_markdown, to_summary_markdown, tr, AnalyzeOptions as CoreAnalyzeOptions,
    Analyzer, BaseLayerMode, DiffOptions, DockerAnalyzeResult, GateFailure, Lang, ProgressCallback,
    ProgressEvent, RegistryCredentials,
};
use tracing::error;

use crate::ui;

/// A tool for exploring, diffing, and linting Docker/OCI container image layers.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    image: Option<String>,
    /// Config file path (or $DIVING_CONFIG). Defaults to ~/.diving/config.yml
    #[arg(short, long, global = true)]
    config: Option<String>,
    /// The result output file
    #[arg(short, long, global = true)]
    output_file: Option<String>,
    /// Also write a short Markdown summary (headline numbers, largest files,
    /// recommendations, and the CI verdict under CI=true), e.g. for a
    /// pull-request comment. `-` prints it to stdout
    #[arg(long, global = true)]
    summary: Option<String>,
    /// Include base image layers in the analysis (auto-detected and hidden by default)
    #[arg(long)]
    no_skip_base: bool,
    /// Skip cross-layer duplicate file detection (and its analysis cache),
    /// trading the dup card for a faster cold analysis. Useful in CI.
    #[arg(long, global = true)]
    no_verify_dup: bool,
    /// Skip reading the previous AI-history snapshot — the AI report does no
    /// regression comparison this run (the current analysis is still recorded
    /// for future runs). Only affects AI mode (`--ai-api-key`).
    #[arg(long)]
    no_ai_history: bool,
    /// Recommendation output language: en or zh (overrides $DIVING_LANG/$LANG)
    #[arg(long, global = true)]
    lang: Option<String>,
    /// OpenAI-compatible API key; enables AI analysis (or $OPENAI_API_KEY)
    #[arg(long)]
    ai_api_key: Option<String>,
    /// OpenAI-compatible API base URL (or $OPENAI_BASE_URL, default OpenAI)
    #[arg(long)]
    ai_base_url: Option<String>,
    /// AI model name (or $OPENAI_MODEL, default gpt-4o)
    #[arg(long)]
    ai_model: Option<String>,
    /// AI system prompt override; replaces the built-in DevSecOps template (or $OPENAI_SYSTEM_PROMPT)
    #[arg(long)]
    ai_system_prompt: Option<String>,
    /// WeCom (企业微信) group-bot webhook URL or key; pushes the result there (or $WECOM_WEBHOOK)
    #[arg(long)]
    wecom_webhook: Option<String>,
    /// Feishu/Lark (飞书) custom bot webhook URL or hook ID (or $FEISHU_WEBHOOK)
    #[arg(long)]
    feishu_webhook: Option<String>,
    /// Feishu/Lark bot signing secret (or $FEISHU_SECRET)
    #[arg(long)]
    feishu_secret: Option<String>,
    /// DingTalk (钉钉) custom bot webhook URL or access_token (or $DINGTALK_WEBHOOK)
    #[arg(long)]
    dingtalk_webhook: Option<String>,
    /// DingTalk bot signing secret (`SEC...`, or $DINGTALK_SECRET)
    #[arg(long)]
    dingtalk_secret: Option<String>,
    /// Slack incoming webhook URL (or $SLACK_WEBHOOK)
    #[arg(long)]
    slack_webhook: Option<String>,
    /// Generic outbound HTTP JSON webhook URL for analysis notifications (or $NOTIFY_WEBHOOK)
    #[arg(long)]
    notify_webhook: Option<String>,
    /// HMAC-SHA256 secret signing the outbound `X-Diving-Signature` header (or $NOTIFY_WEBHOOK_SECRET)
    #[arg(long)]
    notify_webhook_secret: Option<String>,
    /// Public base URL of the web UI (e.g. `https://diving.example.com`) for notification links (or $DIVING_PUBLIC_URL)
    #[arg(long)]
    public_url: Option<String>,
    /// Registry username for private pulls (or $REGISTRY_USERNAME). Also
    /// falls back to `~/.docker/config.json` / credential helpers when unset.
    #[arg(long, short = 'u', global = true)]
    username: Option<String>,
    /// Registry password / PAT (or $REGISTRY_PASSWORD). Prefer
    /// `--password-stdin` in scripts so the secret is not in argv.
    #[arg(long, global = true)]
    password: Option<String>,
    /// Read the registry password from stdin (one line). Mutually useful with
    /// `--username` / `$REGISTRY_USERNAME`.
    #[arg(long, global = true)]
    password_stdin: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Compare two images: size change, shared and new layers, added /
    /// removed / resized files, config/runtime parity, and recommendation changes.
    /// Prints Markdown; `-o` writes JSON (or Markdown for `.md`). Under CI=true
    /// it fails when the target (or upper layers when base layers are ignored)
    /// grew more than `highest_size_increase` in config.yml
    Diff {
        /// The image to compare against, e.g. `app:1.0` or `app:main`
        base: String,
        /// The image to check, e.g. `app:1.1` or `app:pr-42` (omit when using `--archs`)
        target: Option<String>,
        /// Compare two architectures of the same image, e.g. `--archs amd64,arm64`
        #[arg(long, conflicts_with = "target", required_unless_present = "target")]
        archs: Option<String>,
        /// Force skipping detected base layers even for same-arch images
        #[arg(long, conflicts_with_all = ["no_ignore_base_layers", "skip_base_layers"])]
        ignore_base_layers: bool,
        /// Disable automatic base-layer skipping on cross-arch diffs
        #[arg(long, conflicts_with_all = ["ignore_base_layers", "skip_base_layers"])]
        no_ignore_base_layers: bool,
        /// Skip the first N file-carrying layers on each side
        #[arg(long, conflicts_with_all = ["ignore_base_layers", "no_ignore_base_layers"])]
        skip_base_layers: Option<usize>,
    },
}

impl Args {
    /// The `--config` path, if given.
    pub fn config_file(&self) -> Option<&str> {
        self.config.as_deref()
    }
}

fn is_ci() -> bool {
    env::var_os("CI").unwrap_or_default() == "true"
}

fn short_digest(digest: &str) -> &str {
    if digest.starts_with("sha256:") {
        digest.get(..19).unwrap_or(digest)
    } else {
        digest.get(..12).unwrap_or(digest)
    }
}

fn cli_progress_callback(lang: Lang) -> ProgressCallback {
    Arc::new(move |event| match event {
        ProgressEvent::Authenticating { registry } => {
            eprintln!("{}", fill(tr(lang, "prog.auth"), &[&registry]));
        }
        ProgressEvent::AnalysisCacheHit { digest } => {
            eprintln!(
                "{}",
                fill(tr(lang, "prog.cache.hit"), &[short_digest(&digest)])
            );
        }
        ProgressEvent::FetchingManifest => {
            eprintln!("{}", tr(lang, "prog.manifest"));
        }
        ProgressEvent::LayersDiscovered { count, total_bytes } => {
            eprintln!(
                "{}",
                fill(
                    tr(lang, "prog.layers"),
                    &[&count.to_string(), &ByteSize(total_bytes).to_string()]
                )
            );
        }
        ProgressEvent::DownloadingLayer {
            digest,
            size,
            compression,
        } => {
            eprintln!(
                "{}",
                fill(
                    tr(lang, "prog.download"),
                    &[
                        short_digest(&digest),
                        &ByteSize(size).to_string(),
                        compression
                    ]
                )
            );
        }
        ProgressEvent::LayerCacheHit {
            digest,
            size,
            compression,
        } => {
            eprintln!(
                "{}",
                fill(
                    tr(lang, "prog.cached"),
                    &[
                        short_digest(&digest),
                        &ByteSize(size).to_string(),
                        compression
                    ]
                )
            );
        }
        ProgressEvent::LayerCompleted { .. } => {}
        ProgressEvent::UsingStaleCache { as_of } => {
            eprintln!("{}", fill(tr(lang, "prog.cache.stale"), &[&as_of]));
        }
    })
}

struct AnalyzeRunOptions {
    image: String,
    output_file: String,
    summary_file: Option<String>,
    skip_base: bool,
    verify_dup: bool,
    no_ai_history: bool,
    lang: Lang,
    ai_cfg: Option<AiConfig>,
    notify_cfg: NotifyConfig,
    credentials: Option<RegistryCredentials>,
}

struct DiffRunOptions {
    base: String,
    target: String,
    diff_opts: DiffOptions,
    output_file: String,
    summary_file: Option<String>,
    verify_dup: bool,
    lang: Lang,
    credentials: Option<RegistryCredentials>,
}

fn resolve_base_layer_mode(ignore: bool, no_ignore: bool, skip: Option<usize>) -> BaseLayerMode {
    if let Some(count) = skip {
        BaseLayerMode::Skip(count)
    } else if ignore {
        BaseLayerMode::ForceAuto
    } else if no_ignore {
        BaseLayerMode::Off
    } else {
        BaseLayerMode::Auto
    }
}

fn with_arch_query(image: &str, arch: &str) -> String {
    if let Some((head, query)) = image.split_once('?') {
        let mut params: Vec<String> = query
            .split('&')
            .filter(|kv| !kv.is_empty() && !kv.starts_with("arch="))
            .map(ToString::to_string)
            .collect();
        params.push(format!("arch={arch}"));
        format!("{head}?{}", params.join("&"))
    } else {
        format!("{image}?arch={arch}")
    }
}

fn resolve_diff_targets(
    base: String,
    target: Option<String>,
    archs: Option<String>,
) -> Result<(String, String), String> {
    match (target, archs) {
        (Some(t), None) => Ok((base, t)),
        (None, Some(spec)) => {
            let parts: Vec<&str> = spec
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if parts.len() != 2 {
                return Err(
                    "--archs expects two comma-separated architectures, e.g. --archs amd64,arm64"
                        .to_string(),
                );
            }
            Ok((
                with_arch_query(&base, parts[0]),
                with_arch_query(&base, parts[1]),
            ))
        }
        _ => Err("provide either <TARGET> or --archs <ARCH_A,ARCH_B>".to_string()),
    }
}

fn unwrap_result(arc: Arc<DockerAnalyzeResult>) -> DockerAnalyzeResult {
    Arc::try_unwrap(arc).unwrap_or_else(|shared| (*shared).clone())
}

async fn analyze_for_cli(
    analyzer: &Analyzer,
    image: &str,
    lang: Lang,
    verify_dup: bool,
    credentials: Option<RegistryCredentials>,
) -> Result<DockerAnalyzeResult, String> {
    eprintln!("{}", fill(tr(lang, "cli.analyzing"), &[image]));
    // CI gates evaluate the current image directly; interactive runs can fall
    // back to the last cached analysis of the reference if the registry is down.
    let opts = CoreAnalyzeOptions::default()
        .lang(lang)
        .verify_dup(verify_dup)
        .fallback_to_last_known(!is_ci())
        .credentials(credentials)
        .on_progress(cli_progress_callback(lang));
    let arc = analyzer
        .analyze(image, opts)
        .await
        .map_err(|e| e.to_string())?;
    Ok(unwrap_result(arc))
}

fn write_statusln(to_stderr: bool, out: &mut dyn Write, err: &mut dyn Write, line: &str) {
    if to_stderr {
        let _ = writeln!(err, "{line}");
    } else {
        let _ = writeln!(out, "{line}");
    }
}

fn statusln(to_stderr: bool, line: &str) {
    let mut out = io::stdout();
    let mut err = io::stderr();
    write_statusln(to_stderr, &mut out, &mut err, line);
}

fn write_gate_failures(
    failures: &[GateFailure],
    lang: Lang,
    to_stderr: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) {
    let fail = tr(lang, "cli.fail").red().to_string();
    for failure in failures {
        write_statusln(to_stderr, out, err, &failure.render(lang, &fail));
    }
}

fn print_gate_failures(failures: &[GateFailure], lang: Lang, to_stderr: bool) {
    let mut out = io::stdout();
    let mut err = io::stderr();
    write_gate_failures(failures, lang, to_stderr, &mut out, &mut err);
}

fn write_output_target(
    target: &str,
    content: &str,
    separate_stdout: bool,
    out: &mut dyn Write,
) -> Result<bool, String> {
    if target == "-" {
        if separate_stdout {
            writeln!(out, "\n---\n").map_err(|err| err.to_string())?;
        }
        write!(out, "{content}").map_err(|err| err.to_string())?;
        if !content.ends_with('\n') {
            writeln!(out).map_err(|err| err.to_string())?;
        }
        return Ok(true);
    }
    fs::write(target, content).map_err(|err| err.to_string())?;
    Ok(false)
}

async fn analyze(
    analyzer: &Analyzer,
    loaded: &LoadedConfig,
    opts: AnalyzeRunOptions,
) -> Result<bool, String> {
    let AnalyzeRunOptions {
        image,
        output_file,
        summary_file,
        skip_base,
        verify_dup,
        no_ai_history,
        lang,
        ai_cfg,
        notify_cfg,
        credentials,
    } = opts;
    let stdout_piped = output_file == "-" || summary_file.as_deref() == Some("-");
    analyzer.cleanup_caches().await.map_err(|e| e.to_string())?;
    let result = analyze_for_cli(analyzer, &image, lang, verify_dup, credentials).await?;

    let gate_cfg = loaded.config.gate_config();
    let summary = result.summary();
    let failures = evaluate_gate(&result, &summary, lang, &gate_cfg);
    let gate_passed = failures.is_empty();
    let verdict = is_ci().then(|| gate_verdict_md(&failures, lang));

    let reported = ai_cfg.is_some() || !notify_cfg.is_empty();
    let ai_report = if let Some(ai_cfg) = ai_cfg {
        let mut md = to_markdown(&result, skip_base, lang);
        let scripts =
            tokio::task::block_in_place(|| entrypoint_scripts_md(analyzer, &result, lang));
        if !scripts.is_empty() {
            md.push_str("\n\n");
            md.push_str(&scripts);
        }
        let report = analyze_with_ai(
            analyzer.http_client(),
            &loaded.config_dir,
            &image,
            &md,
            &ai_cfg,
            lang,
            no_ai_history,
        )
        .await
        .map_err(|e| fill(tr(lang, "ai.fail"), &[&e]))?;
        statusln(
            stdout_piped,
            &tr(lang, "ai.report").bold().green().to_string(),
        );
        statusln(stdout_piped, &report);
        Some(report)
    } else {
        None
    };

    if !notify_cfg.is_empty() {
        let fail_label = tr(lang, "cli.fail");
        let failure_lines: Vec<String> = failures
            .iter()
            .map(|f| f.render(lang, fail_label))
            .collect();
        let summary_md = wecom_summary(&result, lang);
        let report_url = notify_cfg
            .public_url
            .as_deref()
            .and_then(|base| build_report_url(base, &image, &result.arch));
        let ctx = NotifyContext {
            image: &image,
            result: &result,
            summary: &summary,
            gate_passed,
            gate_verdict_md: verdict.as_deref(),
            gate_failures: &failure_lines,
            ai_report: ai_report.as_deref(),
            summary_md: &summary_md,
            report_url: report_url.as_deref(),
            lang,
        };
        push_notifications(analyzer, &notify_cfg, &ctx, stdout_piped).await?;
    }

    let wants_output = !output_file.is_empty() || summary_file.is_some();
    if reported && !is_ci() && !wants_output {
        return Ok(true);
    }

    if is_ci() || wants_output {
        statusln(
            stdout_piped,
            &tr(lang, "cli.result").bold().green().to_string(),
        );
        statusln(
            stdout_piped,
            &fill(tr(lang, "cli.efficiency"), &[&summary.score.to_string()]),
        );
        statusln(
            stdout_piped,
            &fill(
                tr(lang, "cli.wasted"),
                &[
                    &summary.wasted_size.to_string(),
                    &ByteSize(summary.wasted_size).to_string(),
                ],
            ),
        );
        if !result.recommendations.is_empty() {
            statusln(
                stdout_piped,
                &tr(lang, "cli.recs").bold().green().to_string(),
            );
            for r in &result.recommendations {
                let tag = format!(
                    "[{}/{}]",
                    tr(lang, &format!("sev.{}", r.severity)),
                    tr(lang, &format!("cat.{}", r.category)),
                );
                let saved = if r.est_saved_bytes > 0 {
                    fill(
                        tr(lang, "cli.saved"),
                        &[&ByteSize(r.est_saved_bytes).to_string()],
                    )
                } else {
                    String::new()
                };
                let colored = match r.severity.as_str() {
                    "high" => tag.red(),
                    "medium" => tag.yellow(),
                    "low" => tag.green(),
                    _ => tag.cyan(),
                };
                let note = if gate_cfg.ignore_recommendations.contains(&r.id) {
                    format!(" ({}, {})", r.id, tr(lang, "cli.ignored"))
                } else {
                    format!(" ({})", r.id)
                };
                statusln(
                    stdout_piped,
                    &format!("  {colored} {}{saved}{note}", r.title),
                );
            }
        }

        print_gate_failures(&failures, lang, stdout_piped);
        let mut out = io::stdout();
        let mut wrote_stdout = false;
        if !output_file.is_empty() {
            let is_markdown = output_file == "-"
                || output_file.ends_with(".md")
                || output_file.ends_with(".markdown");
            let content = if is_markdown {
                to_markdown(&result, skip_base, lang)
            } else {
                serde_json::to_string(&result.report(&summary)).map_err(|err| err.to_string())?
            };
            wrote_stdout = write_output_target(&output_file, &content, false, &mut out)?;
        }
        if let Some(target) = &summary_file {
            let md = to_summary_markdown(&result, lang, verdict.as_deref());
            write_output_target(target, &md, wrote_stdout, &mut out)?;
        }
        if is_ci() && !failures.is_empty() {
            return Ok(false);
        }
    } else {
        ui::run_app(result, lang).map_err(|item| item.to_string())?;
    }
    Ok(true)
}

async fn diff(
    analyzer: &Analyzer,
    loaded: &LoadedConfig,
    opts: DiffRunOptions,
) -> Result<bool, String> {
    let DiffRunOptions {
        base,
        target,
        diff_opts,
        output_file,
        summary_file,
        verify_dup,
        lang,
        credentials,
    } = opts;
    let stdout_piped =
        output_file.is_empty() || output_file == "-" || summary_file.as_deref() == Some("-");
    analyzer.cleanup_caches().await.map_err(|e| e.to_string())?;
    let base_res = analyze_for_cli(analyzer, &base, lang, verify_dup, credentials.clone()).await?;
    let target_res = analyze_for_cli(analyzer, &target, lang, verify_dup, credentials).await?;
    let diff = analyzer
        .compute_diff(
            &base,
            &target,
            Arc::new(base_res),
            Arc::new(target_res),
            diff_opts,
        )
        .await
        .map_err(|e| e.to_string())?;
    let gate_cfg = loaded.config.gate_config();
    let failures = evaluate_diff_gate(&diff, gate_cfg.highest_size_increase);
    let verdict = is_ci().then(|| gate_verdict_md(&failures, lang));
    let md = to_diff_markdown(&diff, lang, verdict.as_deref());

    let mut out = io::stdout();
    let wrote_stdout = if output_file.is_empty() || output_file == "-" {
        write_output_target("-", &md, false, &mut out)?
    } else {
        let shown_delta = if diff.base_layers_ignored {
            diff.upper_size_delta
        } else {
            diff.size_delta
        };
        statusln(
            stdout_piped,
            &tr(lang, "cli.result").bold().green().to_string(),
        );
        statusln(
            stdout_piped,
            &fill(
                tr(lang, "cli.diff.size"),
                &[
                    &ByteSize(diff.base.total_size).to_string(),
                    &ByteSize(diff.target.total_size).to_string(),
                    &format!(
                        "{}{}",
                        if shown_delta < 0 { "-" } else { "+" },
                        ByteSize(shown_delta.unsigned_abs())
                    ),
                ],
            ),
        );
        let content = if output_file.ends_with(".md") || output_file.ends_with(".markdown") {
            md.clone()
        } else {
            serde_json::to_string(&diff).map_err(|err| err.to_string())?
        };
        write_output_target(&output_file, &content, false, &mut out)?
    };
    if let Some(target) = &summary_file {
        write_output_target(target, &md, wrote_stdout, &mut out)?;
    }
    print_gate_failures(&failures, lang, stdout_piped);
    Ok(!(is_ci() && !failures.is_empty()))
}

fn exit_on(outcome: Result<bool, String>, lang: Lang) {
    match outcome {
        Ok(true) => {}
        Ok(false) => {
            error!("{}", tr(lang, "cli.cifail"));
            std::process::exit(EXIT_GATE_FAILED);
        }
        Err(err) => {
            error!(err, "analyze image fail");
            std::process::exit(EXIT_ERROR);
        }
    }
}

async fn push_notifications(
    analyzer: &Analyzer,
    cfg: &NotifyConfig,
    ctx: &NotifyContext<'_>,
    to_stderr: bool,
) -> Result<(), String> {
    let wecom_only = cfg.channel_names().as_slice() == ["wecom"];
    let (sending_key, sent_key, fail_key) = if wecom_only {
        ("wecom.sending", "wecom.sent", "wecom.fail")
    } else {
        ("notify.sending", "notify.sent", "notify.fail")
    };
    eprintln!("{}", tr(ctx.lang, sending_key));
    send_all(analyzer.http_client(), cfg, ctx)
        .await
        .map_err(|errs| {
            let joined = errs.join("; ");
            fill(tr(ctx.lang, fail_key), &[&joined])
        })?;
    statusln(
        to_stderr,
        &tr(ctx.lang, sent_key).bold().green().to_string(),
    );
    Ok(())
}

pub async fn run(args: Args, loaded: LoadedConfig, analyzer: Analyzer) {
    let lang = Lang::resolve(args.lang.as_deref());
    let ai_cfg = AiConfig::resolve(
        args.ai_api_key.as_deref(),
        args.ai_base_url.as_deref(),
        args.ai_model.as_deref(),
        args.ai_system_prompt.as_deref(),
    );
    let notify_cfg = NotifyConfig::resolve(
        NotifyInputs {
            wecom_webhook: args.wecom_webhook.as_deref(),
            feishu_webhook: args.feishu_webhook.as_deref(),
            feishu_secret: args.feishu_secret.as_deref(),
            dingtalk_webhook: args.dingtalk_webhook.as_deref(),
            dingtalk_secret: args.dingtalk_secret.as_deref(),
            slack_webhook: args.slack_webhook.as_deref(),
            notify_webhook: args.notify_webhook.as_deref(),
            notify_webhook_secret: args.notify_webhook_secret.as_deref(),
            public_url: args.public_url.as_deref(),
        },
        &loaded.config,
    );
    let skip_base = !args.no_skip_base;
    let verify_dup = !args.no_verify_dup;
    let credentials = resolve_explicit_credentials(
        args.username.as_deref(),
        args.password.as_deref(),
        args.password_stdin,
    );

    if let Some(Command::Diff {
        base,
        target,
        archs,
        ignore_base_layers,
        no_ignore_base_layers,
        skip_base_layers,
    }) = args.command
    {
        TRACE_ID
            .scope(generate_trace_id(), async {
                let outcome = match resolve_diff_targets(base, target, archs) {
                    Ok((base_ref, target_ref)) => {
                        let mode = resolve_base_layer_mode(
                            ignore_base_layers,
                            no_ignore_base_layers,
                            skip_base_layers,
                        );
                        diff(
                            &analyzer,
                            &loaded,
                            DiffRunOptions {
                                base: base_ref,
                                target: target_ref,
                                diff_opts: DiffOptions::default().base_layer_mode(mode),
                                output_file: args.output_file.unwrap_or_default(),
                                summary_file: args.summary,
                                verify_dup,
                                lang,
                                credentials,
                            },
                        )
                        .await
                    }
                    Err(err) => Err(err),
                };
                exit_on(outcome, lang);
            })
            .await;
    } else if let Some(value) = args.image.filter(|s| !s.trim().is_empty()) {
        TRACE_ID
            .scope(generate_trace_id(), async {
                let outcome = analyze(
                    &analyzer,
                    &loaded,
                    AnalyzeRunOptions {
                        image: value,
                        output_file: args.output_file.unwrap_or_default(),
                        summary_file: args.summary,
                        skip_base,
                        verify_dup,
                        no_ai_history: args.no_ai_history,
                        lang,
                        ai_cfg,
                        notify_cfg,
                        credentials,
                    },
                )
                .await;
                exit_on(outcome, lang);
            })
            .await;
    } else {
        let mut cmd = Args::command();
        let _ = cmd.write_help(&mut io::stderr());
        eprintln!();
        std::process::exit(EXIT_ERROR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_subcommand_takes_the_shared_flags_anywhere() {
        let args =
            Args::try_parse_from(["diving", "diff", "app:1", "app:2", "-o", "d.json"]).unwrap();
        assert!(matches!(
            &args.command,
            Some(Command::Diff { base, target: Some(t), archs: None, .. })
                if base == "app:1" && t == "app:2"
        ));
        assert_eq!(args.output_file.as_deref(), Some("d.json"));

        let args =
            Args::try_parse_from(["diving", "--lang", "zh", "diff", "a", "b", "-c", "x.yml"])
                .unwrap();
        assert_eq!(args.lang.as_deref(), Some("zh"));
        assert_eq!(args.config_file(), Some("x.yml"));

        let args = Args::try_parse_from(["diving", "redis:alpine", "-o", "-"]).unwrap();
        assert!(args.command.is_none());
        assert_eq!(args.image.as_deref(), Some("redis:alpine"));
    }

    #[test]
    fn diff_supports_archs_and_base_layer_flags() {
        let args = Args::try_parse_from([
            "diving",
            "diff",
            "redis:alpine",
            "--archs",
            "amd64,arm64",
            "--ignore-base-layers",
        ])
        .unwrap();
        let Some(Command::Diff {
            base,
            target,
            archs,
            ignore_base_layers,
            no_ignore_base_layers,
            skip_base_layers,
        }) = args.command
        else {
            panic!("expected Diff");
        };
        let (b, t) = resolve_diff_targets(base, target, archs).unwrap();
        assert_eq!(b, "redis:alpine?arch=amd64");
        assert_eq!(t, "redis:alpine?arch=arm64");
        assert_eq!(
            resolve_base_layer_mode(ignore_base_layers, no_ignore_base_layers, skip_base_layers),
            BaseLayerMode::ForceAuto
        );

        let args =
            Args::try_parse_from(["diving", "diff", "a:1", "a:2", "--skip-base-layers", "2"])
                .unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Diff {
                skip_base_layers: Some(2),
                ..
            })
        ));

        // Conflicting options are rejected cleanly.
        assert!(Args::try_parse_from([
            "diving",
            "diff",
            "app:latest",
            "app:other",
            "--archs",
            "amd64,arm64"
        ])
        .is_err());
        assert!(Args::try_parse_from([
            "diving",
            "diff",
            "app:latest",
            "app:other",
            "--ignore-base-layers",
            "--no-ignore-base-layers"
        ])
        .is_err());
    }

    #[test]
    fn removed_web_flags_are_rejected() {
        assert!(Args::try_parse_from(["diving", "--mode", "web"]).is_err());
        assert!(Args::try_parse_from(["diving", "--listen", "0.0.0.0:7001"]).is_err());
        assert!(Args::try_parse_from(["diving", "--mcp-token", "tok"]).is_err());
        assert!(Args::try_parse_from(["diving", "--webhook-token", "tok"]).is_err());
    }

    #[test]
    fn piped_stdout_separates_status_onto_stderr_and_divides_combined_reports() {
        use std::env::temp_dir;
        use std::process::id;

        let mut out = Vec::new();
        let mut err = Vec::new();
        write_statusln(true, &mut out, &mut err, "Analyze result:");
        write_gate_failures(
            &[GateFailure {
                key: "cli.check.eff",
                args: vec!["95".into()],
            }],
            Lang::En,
            true,
            &mut out,
            &mut err,
        );
        let wrote_report = write_output_target("-", "# Full Report", false, &mut out).unwrap();
        assert!(wrote_report);
        let wrote_summary =
            write_output_target("-", "# Summary Report", wrote_report, &mut out).unwrap();
        assert!(wrote_summary);

        let stdout_str = String::from_utf8(out).unwrap();
        let stderr_str = String::from_utf8(err).unwrap();
        assert_eq!(stdout_str, "# Full Report\n\n---\n\n# Summary Report\n");
        assert!(stderr_str.contains("Analyze result:"));
        assert!(stderr_str.contains("95"));

        // Non-piped file output keeps status on stdout and writes content to file.
        let dest = temp_dir().join(format!("diving-cli-test-summary-{}.md", id()));
        let mut out2 = Vec::new();
        let mut err2 = Vec::new();
        write_statusln(false, &mut out2, &mut err2, "Analyze result:");
        let wrote_file =
            write_output_target(dest.to_str().unwrap(), "# File Summary", false, &mut out2)
                .unwrap();
        assert!(!wrote_file);
        assert_eq!(String::from_utf8(out2).unwrap(), "Analyze result:\n");
        assert!(err2.is_empty());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "# File Summary");
        let _ = fs::remove_file(&dest);
    }

    #[test]
    fn blank_image_arg_is_treated_as_missing() {
        let args = Args::try_parse_from(["diving", "   "]).unwrap();
        assert!(args.image.filter(|s| !s.trim().is_empty()).is_none());
    }
}
