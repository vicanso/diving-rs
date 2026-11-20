use std::sync::Arc;

/// Progress events emitted during an image analysis.
///
/// Library callers stay silent on stdout/stderr; CLI or custom hosts can attach a
/// [`ProgressCallback`] via [`crate::AnalyzeOptions::on_progress`] to display or
/// forward progress updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    Authenticating {
        registry: String,
    },
    AnalysisCacheHit {
        digest: String,
    },
    FetchingManifest,
    LayersDiscovered {
        count: usize,
        total_bytes: u64,
    },
    DownloadingLayer {
        digest: String,
        size: u64,
        compression: &'static str,
    },
    LayerCacheHit {
        digest: String,
        size: u64,
        compression: &'static str,
    },
    UsingStaleCache {
        as_of: String,
    },
}

/// Callback invoked synchronously when analysis reaches a progress milestone.
pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

pub(crate) fn emit_progress(cb: Option<&ProgressCallback>, event: ProgressEvent) {
    if let Some(cb) = cb {
        cb(event);
    }
}
