//! Pure Rust core library for analyzing Docker / OCI container image layers.
//!
//! Create an [`Analyzer`] via [`Analyzer::builder()`] to inspect images from
//! registries, local `file://` archives/layouts, or `docker://` / `podman://`
//! clients with isolated disk caches and configurable security policies.

mod analyzer;
mod diff;
mod error;
mod gate;
mod i18n;
mod image;
mod markdown;
mod progress;
mod recommend;
mod sensitive;
mod store;
pub mod trace;

#[doc(hidden)]
pub use tracing as __tracing;

pub use analyzer::{AnalyzeOptions, Analyzer, AnalyzerBuilder, PreviewOptions};
pub use diff::{
    diff_images, diff_images_with_options, to_diff_markdown, BaseLayerMode, BaseLayerSkipReason,
    ChangeSet, ConfigDiff, DiffLayer, DiffOptions, DiffSide, DirChange, FileChange, ImageDiff,
    KeyValueDiff, ParityStatus, ParitySummary, RuntimeDiff, ScalarChange,
    DIFF_CONTENT_HASH_MAX_BYTES,
};
pub use error::{Error, Result};
pub use gate::{
    evaluate_diff_gate, evaluate_gate, gate_verdict_md, GateConfig, GateFailure, SeverityThreshold,
};
pub use i18n::{fill, tr, Lang};
pub use image::{
    final_filesystem, host_arch, is_local_registry, is_private_or_loopback_host, list_local_images,
    normalize_registry_host_pattern, parse_image_info, parse_local_images_output, registry_host,
    registry_host_matches, resolve_explicit as resolve_explicit_credentials, AnalyzeReport,
    BigModifiedFileInfo, DockerAnalyzeResult, DockerAnalyzeSummary, DuplicateGroup, DuplicatePath,
    FileTreeItem, FinalFile, ImageFileInfo, ImageFileSummary, ImageFileWastedSummary, ImageInfo,
    ImageLayer, ImagePreview, LocalContainerImage, LocalRuntime, Op, RegistryCredentials,
    RegistryRule, RuntimeCompat, SensitiveFileInfo, UnstrippedBinary, WasteChange,
    REGISTRY_DOCKER_HUB, REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE, REGISTRY_LOCAL_PODMAN,
};
pub use markdown::{to_markdown, to_summary_markdown};
pub use progress::{ProgressCallback, ProgressEvent};
pub use recommend::{
    build_recommendations, gate_violations, Recommendation, CATEGORY_NECESSITY, CATEGORY_SECURITY,
    CATEGORY_SIZE, SEVERITY_HIGH, SEVERITY_INFO, SEVERITY_LOW, SEVERITY_MEDIUM,
};
pub use sensitive::{SensitivePatterns, SensitiveRule};
pub use store::{
    is_safe_blob_id, sha256_hex, tmp_sibling_path, CacheConfig, CacheMode, CleanupStats,
};
