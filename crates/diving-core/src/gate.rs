use crate::diff::ImageDiff;
use crate::i18n::{fill, tr, Lang};
use crate::image::{DockerAnalyzeResult, DockerAnalyzeSummary};
use crate::recommend::{
    gate_violations, SEVERITY_HIGH, SEVERITY_INFO, SEVERITY_LOW, SEVERITY_MEDIUM,
};
use bytesize::ByteSize;
use serde::{Deserialize, Serialize};

/// Severity threshold for [`GateConfig::fail_on_severity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeverityThreshold {
    High,
    Medium,
    Low,
    Info,
}

impl SeverityThreshold {
    pub fn as_str(&self) -> &'static str {
        match self {
            SeverityThreshold::High => SEVERITY_HIGH,
            SeverityThreshold::Medium => SEVERITY_MEDIUM,
            SeverityThreshold::Low => SEVERITY_LOW,
            SeverityThreshold::Info => SEVERITY_INFO,
        }
    }
}

/// Thresholds evaluated by [`evaluate_gate`] and [`evaluate_diff_gate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateConfig {
    /// Minimum efficiency score in `0.0..=1.0` (default `0.95`).
    pub lowest_efficiency: f64,
    /// Maximum wasted bytes across layers (default `20 MiB`).
    pub highest_wasted_bytes: u64,
    /// Maximum wasted percentage in `0.0..=1.0` (default `0.10`).
    pub highest_user_wasted_percent: f64,
    /// Optional uncompressed image size cap.
    pub highest_image_size: Option<u64>,
    /// Optional growth cap used by `evaluate_diff_gate`.
    pub highest_size_increase: Option<u64>,
    /// Optional recommendation severity threshold.
    pub fail_on_severity: Option<SeverityThreshold>,
    /// Normalized lowercase recommendation ids excluded from `fail_on_severity`.
    pub ignore_recommendations: Vec<String>,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            lowest_efficiency: 0.95,
            highest_wasted_bytes: 20 * 1024 * 1024,
            highest_user_wasted_percent: 0.1,
            highest_image_size: None,
            highest_size_increase: None,
            fail_on_severity: None,
            ignore_recommendations: vec![],
        }
    }
}

/// One failed CI check: the i18n key of its message plus the arguments that
/// follow the `{0}` label (e.g. `"FAIL"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateFailure {
    pub key: &'static str,
    pub args: Vec<String>,
}

impl GateFailure {
    pub fn render(&self, lang: Lang, label: &str) -> String {
        let mut args = vec![label];
        args.extend(self.args.iter().map(String::as_str));
        fill(tr(lang, self.key), &args)
    }

    pub fn message(&self, lang: Lang) -> String {
        self.render(lang, tr(lang, "cli.fail"))
    }
}

/// Evaluate all configured CI checks against `result` and `summary`. Empty means pass.
pub fn evaluate_gate(
    result: &DockerAnalyzeResult,
    summary: &DockerAnalyzeSummary,
    lang: Lang,
    config: &GateConfig,
) -> Vec<GateFailure> {
    let lowest_efficiency = (config.lowest_efficiency * 100.0) as u64;
    let highest_wasted_bytes = config.highest_wasted_bytes;
    let highest_user_wasted_percent = config.highest_user_wasted_percent;
    let mut failures = vec![];
    if summary.score < lowest_efficiency {
        failures.push(GateFailure {
            key: "cli.check.eff",
            args: vec![lowest_efficiency.to_string()],
        });
    }
    if summary.wasted_size > highest_wasted_bytes {
        failures.push(GateFailure {
            key: "cli.check.bytes",
            args: vec![ByteSize(highest_wasted_bytes).to_string()],
        });
    }
    if summary.wasted_percent > highest_user_wasted_percent {
        failures.push(GateFailure {
            key: "cli.check.pct",
            args: vec![format!("{highest_user_wasted_percent:.2}")],
        });
    }
    if let Some(limit) = config.highest_image_size {
        if result.total_size > limit {
            failures.push(GateFailure {
                key: "cli.check.size",
                args: vec![
                    ByteSize(result.total_size).to_string(),
                    ByteSize(limit).to_string(),
                ],
            });
        }
    }
    if let Some(threshold) = config.fail_on_severity {
        let ignored: Vec<String> = config
            .ignore_recommendations
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let count = gate_violations(&result.recommendations, threshold.as_str(), &ignored).len();
        if count > 0 {
            failures.push(GateFailure {
                key: "cli.check.severity",
                args: vec![
                    count.to_string(),
                    tr(lang, &format!("sev.{}", threshold.as_str())).to_string(),
                ],
            });
        }
    }
    failures
}

/// Evaluate the diff CI check (`highest_size_increase`). Empty means pass.
pub fn evaluate_diff_gate(
    diff: &ImageDiff,
    highest_size_increase: Option<u64>,
) -> Vec<GateFailure> {
    match highest_size_increase {
        Some(limit) if diff.size_delta > limit as i64 => vec![GateFailure {
            key: "cli.check.growth",
            args: vec![
                ByteSize(diff.size_delta.unsigned_abs()).to_string(),
                ByteSize(limit).to_string(),
            ],
        }],
        _ => vec![],
    }
}

/// Render the gate outcome as Markdown (used in summaries and notifications).
pub fn gate_verdict_md(failures: &[GateFailure], lang: Lang) -> String {
    if failures.is_empty() {
        return format!("**{}**", tr(lang, "cli.gate.pass"));
    }
    let label = tr(lang, "cli.fail");
    let mut md = format!("**{}**", tr(lang, "cli.gate.fail"));
    for failure in failures {
        md.push_str(&format!("\n- {}", failure.render(lang, label)));
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommend::Recommendation;

    fn failed_keys(result: &DockerAnalyzeResult, config: &GateConfig) -> Vec<&'static str> {
        let summary = result.summary();
        evaluate_gate(result, &summary, Lang::En, config)
            .iter()
            .map(|f| f.key)
            .collect()
    }

    #[test]
    fn diff_gate_fails_only_on_growth_past_the_limit() {
        let grew = |size_delta| ImageDiff {
            size_delta,
            ..Default::default()
        };
        let mib = 1024 * 1024;
        assert!(evaluate_diff_gate(&grew(30 * mib), None).is_empty());
        assert!(evaluate_diff_gate(&grew(-30 * mib), Some(0)).is_empty());
        assert!(evaluate_diff_gate(&grew(20 * mib), Some(20 * mib as u64)).is_empty());
        let failures = evaluate_diff_gate(&grew(30 * mib), Some(20 * mib as u64));
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].render(Lang::En, "FAIL"),
            "FAIL: image growth check, 30.0 MiB larger uncompressed, highest: 20.0 MiB"
        );
    }

    #[test]
    fn image_size_cap_compares_the_uncompressed_size() {
        let result = DockerAnalyzeResult {
            size: 200 * 1024 * 1024,
            total_size: 600 * 1024 * 1024,
            ..Default::default()
        };
        let defaults = GateConfig::default();
        assert!(failed_keys(&result, &defaults).is_empty());
        let cap = |mib: u64| GateConfig {
            highest_image_size: Some(mib * 1024 * 1024),
            ..GateConfig::default()
        };
        assert_eq!(failed_keys(&result, &cap(500)), ["cli.check.size"]);
        assert!(failed_keys(&result, &cap(600)).is_empty());

        let summary = result.summary();
        let failure = &evaluate_gate(&result, &summary, Lang::En, &cap(500))[0];
        assert_eq!(
            failure.render(Lang::En, "FAIL"),
            "FAIL: image size check, 600.0 MiB uncompressed, highest: 500.0 MiB"
        );
    }

    #[test]
    fn severity_gate_ignores_accepted_ids() {
        let result = DockerAnalyzeResult {
            recommendations: vec![
                Recommendation {
                    id: "runasroot".into(),
                    severity: SEVERITY_MEDIUM.into(),
                    ..Default::default()
                },
                Recommendation {
                    id: "secfiles".into(),
                    severity: SEVERITY_HIGH.into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let cfg = GateConfig {
            fail_on_severity: Some(SeverityThreshold::High),
            ignore_recommendations: vec!["secfiles".into()],
            ..Default::default()
        };
        assert!(failed_keys(&result, &cfg).is_empty());
    }

    #[test]
    fn gate_verdict_lists_each_failed_check_under_the_verdict() {
        assert_eq!(gate_verdict_md(&[], Lang::En), "**CI gate: passed**");

        let failures = [
            GateFailure {
                key: "cli.check.eff",
                args: vec!["95".to_string()],
            },
            GateFailure {
                key: "cli.check.severity",
                args: vec!["2".to_string(), "high".to_string()],
            },
        ];
        assert_eq!(
            gate_verdict_md(&failures, Lang::En),
            "**CI gate: failed**\n\
             - FAIL: lowest efficiency check, lowest: 95\n\
             - FAIL: recommendation severity check, 2 at high or above"
        );
        assert!(gate_verdict_md(&failures, Lang::Zh).starts_with("**CI 卡口：未通过**"));
    }
}
