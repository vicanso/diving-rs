//! Integration tests for `diving-core`'s library API: security defaults,
//! multi-instance isolation, cache lifecycle, progress callbacks, `diff`, and gates.

mod common;

use common::{build_fixture_tar, build_fixture_v2_tar};
use diving_core::{
    evaluate_diff_gate, evaluate_gate, AnalyzeOptions, Analyzer, CacheConfig, Error, GateConfig,
    Lang, ProgressEvent, SensitivePatterns, SeverityThreshold,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn default_analyzer_blocks_local_sources_and_disallowed_registries() {
    let tmp = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), build_fixture_tar()).unwrap();
    let file_ref = format!("file://{}", tmp.path().display());

    let default_analyzer = Analyzer::builder().build().unwrap();
    let err = default_analyzer
        .analyze(&file_ref, AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::LocalSourceDisabled { .. }));
    assert!(err.is_forbidden());

    let err = default_analyzer
        .analyze("docker://redis:alpine", AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::LocalSourceDisabled { .. }));

    let allowlisted = Analyzer::builder()
        .allow_local_sources(true)
        .registry_allowlist(["ghcr.io"])
        .build()
        .unwrap();
    let err = allowlisted
        .analyze(&file_ref, AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::DisallowedRegistry { .. }));
    assert!(err.is_forbidden());

    let err = allowlisted
        .analyze("redis:alpine", AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::DisallowedRegistry { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_analyzer_instances_remain_isolated_and_clean_up() {
    let tmp = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), build_fixture_tar()).unwrap();
    let image = format!("file://{}", tmp.path().display());

    let mut ignore_pem = SensitivePatterns::new();
    ignore_pem.add_ignore("**/secret.pem").unwrap();

    let mut extra_rule = SensitivePatterns::new();
    extra_rule
        .add_pattern("**/config.yml", "Custom config file")
        .unwrap();

    let a1 = Analyzer::builder()
        .allow_local_file(true)
        .sensitive_patterns(ignore_pem)
        .build()
        .unwrap();

    let dir2 = tempfile::tempdir().unwrap();
    let a2 = Analyzer::builder()
        .cache(
            CacheConfig::persistent(dir2.path().join("layers"), dir2.path().join("analysis"))
                .layer_ttl(Duration::from_secs(3600))
                .max_layer_cache_size(Some(1024 * 1024 * 10)),
        )
        .allow_local_file(true)
        .sensitive_patterns(extra_rule)
        .build()
        .unwrap();

    let (r1, r2) = tokio::join!(
        a1.analyze(&image, AnalyzeOptions::default().verify_dup(false)),
        a2.analyze(&image, AnalyzeOptions::default().verify_dup(false)),
    );
    let r1 = r1.unwrap();
    let r2 = r2.unwrap();

    assert!(!r1
        .sensitive_files
        .iter()
        .any(|s| s.path == "app/secret.pem"));
    assert!(r2
        .sensitive_files
        .iter()
        .any(|s| s.path == "app/secret.pem"));
    assert!(r2
        .sensitive_files
        .iter()
        .any(|s| s.path == "etc/config.yml" && s.reason == "Custom config file"));

    let stats = a2.cleanup_caches().await.unwrap();
    assert_eq!(stats.removed_blobs, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn analyzer_diff_and_gates_work_end_to_end() {
    let v1 = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    let v2 = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(v1.path(), build_fixture_tar()).unwrap();
    std::fs::write(v2.path(), build_fixture_v2_tar()).unwrap();

    let events = Arc::new(Mutex::new(Vec::<ProgressEvent>::new()));
    let events_cb = events.clone();

    let analyzer = Analyzer::builder()
        .allow_local_file(true)
        .singleflight(false)
        .build()
        .unwrap();

    let base_ref = format!("file://{}", v1.path().display());
    let target_ref = format!("file://{}", v2.path().display());

    let diff = analyzer
        .diff(
            &base_ref,
            &target_ref,
            AnalyzeOptions::default()
                .verify_dup(false)
                .on_progress(Arc::new(move |ev| {
                    events_cb.lock().unwrap().push(ev);
                })),
        )
        .await
        .unwrap();

    assert_eq!(diff.shared_layers, 1);
    assert_eq!(diff.added.count, 2);

    let diff_failures = evaluate_diff_gate(&diff, Some(1024));
    assert_eq!(diff_failures.len(), 1);

    let res = analyzer
        .analyze(&base_ref, AnalyzeOptions::default().verify_dup(false))
        .await
        .unwrap();
    let summary = res.summary();
    let gate_cfg = GateConfig {
        fail_on_severity: Some(SeverityThreshold::High),
        ..GateConfig::default()
    };
    let failures = evaluate_gate(&res, &summary, Lang::En, &gate_cfg);
    assert!(
        failures.iter().any(|f| f.key == "cli.check.severity"),
        "expected secret file to trip severity gate"
    );

    // Local source emits zero registry-network progress events.
    assert!(events.lock().unwrap().is_empty());
}
