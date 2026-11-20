use bytesize::ByteSize;
use diving_core::{fill, tr, DockerAnalyzeResult, Lang};

/// Short analysis summary pushed to chat bots when AI report generation is off.
pub fn wecom_summary(result: &DockerAnalyzeResult, lang: Lang) -> String {
    let summary = result.summary();
    let mut s = String::new();
    if let Some(as_of) = &result.stale_as_of {
        s.push_str(&format!("> {}\n\n", fill(tr(lang, "md.stale"), &[as_of])));
    }
    s.push_str(&format!(
        "**{}:** {} %\n**{}:** {} bytes ({})\n**{}:** {}\n**{}:** {}\n",
        tr(lang, "md.f.eff"),
        summary.score,
        tr(lang, "md.f.wasted"),
        summary.wasted_size,
        ByteSize(summary.wasted_size),
        tr(lang, "md.f.usize"),
        ByteSize(result.total_size),
        tr(lang, "md.f.csize"),
        ByteSize(result.size),
    ));
    if !result.recommendations.is_empty() {
        s.push_str(&format!("\n### {}\n", tr(lang, "md.recs")));
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
            s.push_str(&format!("- {} {}{}\n", tag, r.title, saved));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wecom_summary_flags_a_stale_result_first() {
        let fresh = DockerAnalyzeResult::default();
        assert!(wecom_summary(&fresh, Lang::En).starts_with("**Efficiency"));

        let stale = DockerAnalyzeResult {
            stale_as_of: Some("2026-10-06T04:22:25+00:00".to_string()),
            ..Default::default()
        };
        let summary = wecom_summary(&stale, Lang::En);
        assert!(
            summary.starts_with(
                "> Registry unavailable — this is the analysis cached at 2026-10-06T04:22:25+00:00"
            ),
            "{summary}"
        );
        assert!(summary.contains("**Efficiency"));
    }
}
