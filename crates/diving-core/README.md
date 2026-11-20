# diving-core

Core Docker / OCI container image layer analysis engine from [diving-rs](https://github.com/vicanso/diving-rs).

Provides an instance-isolated, async Rust API (`Analyzer`) with no global state, no stdin/stdout/stderr side effects, and secure-by-default server policies (remote registries only unless local sources or `~/.docker/config.json` are explicitly enabled).

## Quick Example

```rust,no_run
use diving_core::{AnalyzeOptions, Analyzer, CacheConfig, Lang, to_summary_markdown};
use std::time::Duration;

#[tokio::main]
async fn main() -> diving_core::Result<()> {
    let analyzer = Analyzer::builder()
        .cache(CacheConfig::persistent("/tmp/diving/layers", "/tmp/diving/analysis"))
        .registry_allowlist(["index.docker.io", "ghcr.io"])
        .max_concurrent_analyses(4)
        .memory_cache_ttl(Duration::from_secs(60))
        .build()?;

    let result = analyzer
        .analyze("redis:alpine", AnalyzeOptions::default().lang(Lang::En))
        .await?;

    println!("{}", to_summary_markdown(&result, Lang::En, None));
    Ok(())
}
```
