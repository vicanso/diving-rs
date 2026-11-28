use super::image_ref::{host_arch, platform_matches};
use super::layer::TarIndex;
use super::oci_image::{
    ImageManifest, ImageManifestConfig, ImageManifestLayer, MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
    MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_MANIFEST_LIST, MEDIA_TYPE_OCI_MANIFEST,
};
use crate::error::{Error, IoSnafu, JsonSnafu, Result};
use crate::tl_info;
use bytesize::ByteSize;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use home::home_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snafu::ResultExt;
use std::env;
use std::fs::{self, File};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use tokio::process::Command;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalManifest {
    #[serde(rename = "Config")]
    pub config: String,
    #[serde(rename = "RepoTags")]
    pub repo_tags: Vec<String>,
    #[serde(rename = "Layers")]
    pub layers: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LocalImage {
    pub config: String,
    pub layers: Vec<ImageManifestLayer>,
}

impl From<LocalImage> for ImageManifest {
    fn from(value: LocalImage) -> Self {
        ImageManifest {
            media_type: MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST.to_string(),
            schema_version: 2,
            config: ImageManifestConfig {
                digest: value.config,
                ..Default::default()
            },
            layers: value.layers,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OciDescriptor {
    #[serde(default)]
    media_type: String,
    digest: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    platform: Option<OciPlatform>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OciPlatform {
    #[serde(default)]
    architecture: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OciIndex {
    #[serde(default)]
    manifests: Vec<OciDescriptor>,
}

#[derive(Debug, Default, Deserialize)]
struct OciManifest {
    config: OciDescriptor,
    #[serde(default)]
    layers: Vec<OciDescriptor>,
}

fn oci_blob_path(digest: &str) -> String {
    format!("blobs/{}", digest.replacen(':', "/", 1))
}

pub(crate) const MANIFEST_ACCEPTS: [&str; 4] = [
    MEDIA_TYPE_IMAGE_INDEX,
    MEDIA_TYPE_MANIFEST_LIST,
    MEDIA_TYPE_OCI_MANIFEST,
    MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
];

pub(crate) fn is_index_media_type(media_type: &str) -> bool {
    media_type == MEDIA_TYPE_IMAGE_INDEX || media_type == MEDIA_TYPE_MANIFEST_LIST
}

pub(crate) fn cache_arch(content_type: Option<&str>, arch: &str) -> String {
    match content_type {
        Some(t) if !is_index_media_type(t) => String::new(),
        _ => arch.to_string(),
    }
}

fn pick_oci_descriptor<'a>(
    manifests: &'a [OciDescriptor],
    arch: &str,
) -> Option<&'a OciDescriptor> {
    let wanted = if arch.is_empty() { host_arch() } else { arch };
    let linux = |d: &&OciDescriptor| {
        d.platform
            .as_ref()
            .is_some_and(|p| p.os == "linux" && p.architecture != "unknown")
    };
    manifests
        .iter()
        .filter(linux)
        .find(|d| {
            d.platform
                .as_ref()
                .is_some_and(|p| platform_matches(wanted, &p.architecture, p.variant.as_deref()))
        })
        .or_else(|| manifests.iter().find(linux))
        .or_else(|| manifests.iter().find(|d| d.platform.is_none()))
}

pub(crate) fn read_oci_layout(index: &TarIndex, arch: &str) -> Result<LocalImage> {
    let root: OciIndex = serde_json::from_slice(&index.read("index.json")?).context(JsonSnafu {
        context: "oci_index",
    })?;
    let mut descriptor = pick_oci_descriptor(&root.manifests, arch)
        .cloned()
        .ok_or_else(|| Error::Other {
            message: "index.json lists no image manifest".to_string(),
        })?;
    if is_index_media_type(&descriptor.media_type) {
        let nested: OciIndex = serde_json::from_slice(
            &index.read(&oci_blob_path(&descriptor.digest))?,
        )
        .context(JsonSnafu {
            context: "oci_nested_index",
        })?;
        descriptor = pick_oci_descriptor(&nested.manifests, arch)
            .cloned()
            .ok_or_else(|| Error::Other {
                message: format!("image index {} lists no image manifest", descriptor.digest),
            })?;
    }
    let manifest: OciManifest = serde_json::from_slice(
        &index.read(&oci_blob_path(&descriptor.digest))?,
    )
    .context(JsonSnafu {
        context: "oci_manifest",
    })?;
    Ok(LocalImage {
        config: oci_blob_path(&manifest.config.digest),
        layers: manifest
            .layers
            .into_iter()
            .map(|layer| ImageManifestLayer {
                media_type: layer.media_type,
                digest: oci_blob_path(&layer.digest),
                size: layer.size,
            })
            .collect(),
    })
}

fn local_cli_search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = env::var_os("PATH")
        .map(|raw| env::split_paths(&raw).collect())
        .unwrap_or_default();
    let mut extra = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = home_dir() {
        extra.push(home.join(".orbstack/bin"));
        extra.push(home.join(".rd/bin"));
    }
    for dir in extra {
        if !paths.iter().any(|existing| existing == &dir) {
            paths.push(dir);
        }
    }
    paths
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && (meta.permissions().mode() & 0o111) != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn is_executable_in_dirs(cli: &str, dirs: &[PathBuf]) -> bool {
    let exact_hit = dirs.iter().any(|dir| is_executable_file(&dir.join(cli)));
    #[cfg(windows)]
    {
        if exact_hit {
            return true;
        }
        if Path::new(cli).extension().is_none() {
            let pathext = env::var_os("PATHEXT")
                .and_then(|v| v.into_string().ok())
                .unwrap_or_else(|| ".EXE;.CMD;.BAT;.COM".to_owned());
            for dir in dirs {
                for ext in pathext.split(';').filter(|e| !e.trim().is_empty()) {
                    let ext = ext.trim().trim_start_matches('.');
                    if dir.join(format!("{cli}.{ext}")).is_file() {
                        return true;
                    }
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        exact_hit
    }
}

fn local_cli_command(cli: &str) -> Command {
    let mut cmd = Command::new(cli);
    let paths = local_cli_search_paths();
    if let Ok(joined) = env::join_paths(&paths) {
        cmd.env("PATH", joined);
    }
    cmd
}

fn command_error_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("exit status {}", output.status)
    } else {
        stderr
    }
}

pub(crate) async fn save_local_image_to_file(cli: &str, image: &str, path: &Path) -> Result<()> {
    tl_info!(image = image, cli = cli, "saving image");
    let file = File::create(path).context(IoSnafu {})?;
    let status = local_cli_command(cli)
        .arg("save")
        .arg(image)
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|err| Error::Other {
            message: format!("failed to run `{cli} save`: {err}"),
        })?;
    if !status.success() {
        return Err(Error::Other {
            message: format!("{cli} save {image} failed ({status})"),
        });
    }
    tl_info!(image = image, "save image done");
    Ok(())
}

async fn run_container_save(image: &str, path: &Path, arch: Option<&str>) -> Result<Output> {
    let mut cmd = local_cli_command("container");
    cmd.args(["image", "save"]);
    if let Some(target_arch) = arch {
        cmd.arg("--arch").arg(target_arch);
    }
    cmd.arg("--output")
        .arg(path)
        .arg(image)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| Error::Other {
            message: format!("failed to run `container image save`: {err}"),
        })
}

pub(crate) async fn save_container_image_to_file(
    image: &str,
    arch: &str,
    path: &Path,
) -> Result<()> {
    tl_info!(image = image, cli = "container", "saving image");
    let preferred = if arch.is_empty() { host_arch() } else { arch };
    let mut output = run_container_save(image, path, Some(preferred)).await?;
    if !output.status.success() && arch.is_empty() {
        let fallback = run_container_save(image, path, None).await?;
        if fallback.status.success() || !fallback.stderr.is_empty() {
            output = fallback;
        }
    }
    if !output.status.success() {
        let detail = command_error_detail(&output);
        return Err(Error::Other {
            message: format!("container image save {image} failed: {detail}"),
        });
    }
    tl_info!(image = image, "save image done");
    Ok(())
}

/// Supported local container engine CLIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalRuntime {
    Docker,
    Podman,
    Container,
}

impl LocalRuntime {
    pub const ALL: [Self; 3] = [Self::Docker, Self::Podman, Self::Container];

    pub fn cli(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::Container => "container",
        }
    }

    pub fn scheme(self) -> &'static str {
        match self {
            Self::Docker => "docker://",
            Self::Podman => "podman://",
            Self::Container => "container://",
        }
    }

    /// Returns `true` when this runtime's CLI executable is present on standard search paths.
    pub fn is_installed(self) -> bool {
        is_executable_in_dirs(self.cli(), &local_cli_search_paths())
    }

    /// Orders runtimes with installed CLIs first while preserving `Docker, Podman, Container` order.
    pub fn ordered_by_installed_with(mut is_installed: impl FnMut(Self) -> bool) -> [Self; 3] {
        let mut installed = Vec::with_capacity(Self::ALL.len());
        let mut missing = Vec::with_capacity(Self::ALL.len());
        for rt in Self::ALL {
            if is_installed(rt) {
                installed.push(rt);
            } else {
                missing.push(rt);
            }
        }
        installed.extend(missing);
        [installed[0], installed[1], installed[2]]
    }

    /// Orders runtimes by local CLI availability on this machine.
    pub fn ordered_by_installed() -> [Self; 3] {
        Self::ordered_by_installed_with(Self::is_installed)
    }
}

/// Summary of an image stored in a local Docker, Podman, or Apple Container engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalContainerImage {
    pub runtime: LocalRuntime,
    pub repository: String,
    pub tag: String,
    pub id: String,
    pub size: String,
    pub created: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub created_at: Option<i64>,
}

impl LocalContainerImage {
    /// Returns the human-readable `repository:tag` label (or `id` when untagged).
    pub fn display_name(&self) -> String {
        if !self.repository.is_empty() && self.repository != "<none>" {
            if !self.tag.is_empty() && self.tag != "<none>" {
                return format!("{}:{}", self.repository, self.tag);
            }
            return self.repository.clone();
        }
        self.id.clone()
    }

    /// Returns the `docker://`, `podman://`, or `container://` image reference suitable for `Analyzer::analyze`.
    pub fn reference(&self) -> String {
        let scheme = self.runtime.scheme();
        if !self.repository.is_empty() && self.repository != "<none>" {
            if !self.tag.is_empty() && self.tag != "<none>" {
                return format!("{scheme}{}:{}", self.repository, self.tag);
            }
            if !self.id.is_empty() {
                return format!("{scheme}{}", self.id);
            }
            return format!("{scheme}{}", self.repository);
        }
        format!("{scheme}{}", self.id)
    }

    /// Returns the CLI argument (`repository:tag` or short `id`) without its runtime URI scheme.
    pub fn cli_target(&self) -> String {
        let full = self.reference();
        full.strip_prefix(self.runtime.scheme())
            .unwrap_or(&full)
            .to_string()
    }

    /// Returns the exact image size in bytes when known, or parses the formatted size string.
    pub fn effective_size_bytes(&self) -> Option<u64> {
        self.size_bytes
            .or_else(|| parse_human_size_bytes(&self.size))
    }

    /// Returns the Unix creation timestamp in seconds when known, or parses the display timestamp.
    pub fn effective_created_at(&self) -> Option<i64> {
        self.created_at
            .or_else(|| parse_created_timestamp(&self.created))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerImageRow {
    #[serde(default)]
    repository: String,
    #[serde(default)]
    tag: String,
    #[serde(default, rename = "ID")]
    id: String,
    #[serde(default)]
    size: String,
    #[serde(default)]
    created_since: String,
    #[serde(default)]
    created_at: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PodmanImageRow {
    #[serde(default)]
    id: String,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    repo_tags: Vec<String>,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    size: Option<Value>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    created_since: String,
    #[serde(default)]
    created_at: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerImageEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    configuration: ContainerConfiguration,
    #[serde(default)]
    variants: Vec<ContainerVariant>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerConfiguration {
    #[serde(default)]
    name: String,
    #[serde(default)]
    creation_date: String,
}

#[derive(Debug, Default, Deserialize)]
struct ContainerVariant {
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    platform: Option<OciPlatform>,
}

fn format_container_created(raw: &str) -> (String, Option<i64>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return (
            dt.format("%Y-%m-%d %H:%M").to_string(),
            Some(dt.timestamp()),
        );
    }
    let ts = parse_created_timestamp(trimmed);
    (trimmed.to_string(), ts)
}

fn normalize_container_name(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("docker.io/library/")
        .or_else(|| trimmed.strip_prefix("docker.io/"))
        .unwrap_or(trimmed)
}

fn pick_container_variant_size(variants: &[ContainerVariant]) -> (String, Option<u64>) {
    let is_real = |v: &&ContainerVariant| {
        v.platform.as_ref().is_some_and(|p| {
            !p.architecture.is_empty() && !p.architecture.eq_ignore_ascii_case("unknown")
        })
    };
    let host = host_arch();
    let chosen = variants
        .iter()
        .filter(is_real)
        .find(|v| {
            v.platform
                .as_ref()
                .is_some_and(|p| platform_matches(host, &p.architecture, p.variant.as_deref()))
        })
        .or_else(|| variants.iter().find(is_real))
        .or_else(|| variants.first());
    let size_bytes = chosen.and_then(|v| v.size);
    let formatted = size_bytes
        .map(|bytes| ByteSize::b(bytes).to_string())
        .unwrap_or_default();
    (formatted, size_bytes)
}

pub(crate) fn parse_human_size_bytes(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let split_at = trimmed
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(trimmed.len());
    let num_part = trimmed[..split_at].trim();
    if num_part.is_empty() {
        return None;
    }
    let value: f64 = num_part.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let unit = trimmed[split_at..].trim().to_ascii_lowercase();
    let multiplier: f64 = match unit.as_str() {
        "" | "b" | "byte" | "bytes" => 1.0,
        "k" | "kb" => 1_000.0,
        "ki" | "kib" => 1024.0,
        "m" | "mb" => 1_000_000.0,
        "mi" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" => 1_000_000_000.0,
        "gi" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" => 1_000_000_000_000.0,
        "ti" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let scaled = (value * multiplier).round();
    if !scaled.is_finite() || scaled < 0.0 || scaled > u64::MAX as f64 {
        return None;
    }
    Some(scaled as u64)
}

fn parse_relative_quantity(body: &str, now_secs: i64) -> Option<i64> {
    let mut parts = body.split_whitespace();
    let qty_raw = parts.next()?;
    let unit_raw = parts.next()?;
    let qty: i64 = match qty_raw {
        "a" | "an" => 1,
        other => other.parse().ok()?,
    };
    let unit_secs: i64 = match unit_raw.trim_end_matches('s') {
        "second" | "sec" => 1,
        "minute" | "min" => 60,
        "hour" | "hr" => 3_600,
        "day" => 86_400,
        "week" | "wk" => 7 * 86_400,
        "month" | "mo" => 30 * 86_400,
        "year" | "yr" => 365 * 86_400,
        _ => return None,
    };
    Some(now_secs.saturating_sub(qty.saturating_mul(unit_secs)))
}

fn parse_relative_created_ago(lower: &str, now_secs: i64) -> Option<i64> {
    match lower {
        "just now" | "now" | "less than a second ago" | "today" => return Some(now_secs),
        "yesterday" => return Some(now_secs - 86_400),
        _ => {}
    }
    let body = lower.strip_suffix("ago")?.trim();
    if let Some(rest) = body.strip_prefix("about ") {
        return parse_relative_quantity(rest.trim(), now_secs);
    }
    parse_relative_quantity(body, now_secs)
}

pub(crate) fn parse_created_timestamp(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.timestamp());
    }
    let mut tokens = trimmed.split_whitespace();
    if let (Some(date), Some(time), Some(offset)) = (tokens.next(), tokens.next(), tokens.next()) {
        let candidate = format!("{date} {time} {offset}");
        if let Ok(dt) = DateTime::parse_from_str(&candidate, "%Y-%m-%d %H:%M:%S %z") {
            return Some(dt.timestamp());
        }
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(trimmed, fmt) {
            return Some(dt.and_utc().timestamp());
        }
    }
    if let Some(dt) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
    {
        return Some(dt.and_utc().timestamp());
    }
    parse_relative_created_ago(&trimmed.to_ascii_lowercase(), Utc::now().timestamp())
}

fn short_image_id(raw: &str) -> String {
    let stripped = raw.trim().strip_prefix("sha256:").unwrap_or(raw.trim());
    stripped.chars().take(12).collect()
}

fn split_repo_tag(name: &str) -> (String, String) {
    let trimmed = name.trim();
    let after_slash = trimmed.rfind('/').map_or(0, |idx| idx + 1);
    if let Some(colon) = trimmed[after_slash..].rfind(':') {
        let idx = after_slash + colon;
        return (trimmed[..idx].to_string(), trimmed[idx + 1..].to_string());
    }
    (trimmed.to_string(), "<none>".to_string())
}

fn format_podman_size(value: Option<&Value>) -> (String, Option<u64>) {
    match value {
        Some(Value::String(s)) => (s.clone(), parse_human_size_bytes(s)),
        Some(Value::Number(n)) => {
            let bytes = n.as_u64();
            let text = bytes
                .map(|b| ByteSize::b(b).to_string())
                .unwrap_or_default();
            (text, bytes)
        }
        _ => (String::new(), None),
    }
}

fn is_dangling(repository: &str, tag: &str) -> bool {
    (repository.is_empty() || repository == "<none>") && (tag.is_empty() || tag == "<none>")
}

/// Parses stdout from `docker images --format "{{json .}}"`,
/// `podman images --format json`, or `container image list --format json`.
pub fn parse_local_images_output(
    runtime: LocalRuntime,
    stdout: &str,
) -> Result<Vec<LocalContainerImage>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    match runtime {
        LocalRuntime::Docker => {
            let mut images = Vec::new();
            for line in trimmed.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let row: DockerImageRow = serde_json::from_str(line).context(JsonSnafu {
                    context: "docker_images",
                })?;
                if is_dangling(&row.repository, &row.tag) {
                    continue;
                }
                let created = if row.created_since.is_empty() {
                    row.created_at.clone()
                } else {
                    row.created_since.clone()
                };
                let size_bytes = parse_human_size_bytes(&row.size);
                let created_at = parse_created_timestamp(&row.created_at)
                    .or_else(|| parse_created_timestamp(&row.created_since));
                images.push(LocalContainerImage {
                    runtime,
                    repository: row.repository,
                    tag: row.tag,
                    id: short_image_id(&row.id),
                    size: row.size,
                    created,
                    size_bytes,
                    created_at,
                });
            }
            Ok(images)
        }
        LocalRuntime::Podman => {
            let rows: Vec<PodmanImageRow> = serde_json::from_str(trimmed).context(JsonSnafu {
                context: "podman_images",
            })?;
            let mut images = Vec::new();
            for row in rows {
                let id = short_image_id(&row.id);
                let (size, size_bytes) = format_podman_size(row.size.as_ref());
                let created = if row.created_since.is_empty() {
                    row.created_at.clone()
                } else {
                    row.created_since.clone()
                };
                let created_at = row
                    .created
                    .or_else(|| parse_created_timestamp(&row.created_at))
                    .or_else(|| parse_created_timestamp(&row.created_since));
                if !row.repository.is_empty() {
                    if is_dangling(&row.repository, &row.tag) {
                        continue;
                    }
                    let tag = if row.tag.is_empty() {
                        "<none>".to_string()
                    } else {
                        row.tag
                    };
                    images.push(LocalContainerImage {
                        runtime,
                        repository: row.repository,
                        tag,
                        id,
                        size,
                        created,
                        size_bytes,
                        created_at,
                    });
                    continue;
                }
                let refs = if row.names.is_empty() {
                    &row.repo_tags
                } else {
                    &row.names
                };
                for name in refs {
                    let (repository, tag) = split_repo_tag(name);
                    if is_dangling(&repository, &tag) {
                        continue;
                    }
                    images.push(LocalContainerImage {
                        runtime,
                        repository,
                        tag,
                        id: id.clone(),
                        size: size.clone(),
                        created: created.clone(),
                        size_bytes,
                        created_at,
                    });
                }
            }
            Ok(images)
        }
        LocalRuntime::Container => {
            let entries: Vec<ContainerImageEntry> =
                serde_json::from_str(trimmed).context(JsonSnafu {
                    context: "container_images",
                })?;
            let mut images = Vec::new();
            for entry in entries {
                let name = normalize_container_name(&entry.configuration.name);
                if name.is_empty() {
                    continue;
                }
                let (repository, tag) = split_repo_tag(name);
                if is_dangling(&repository, &tag) {
                    continue;
                }
                let (size, size_bytes) = pick_container_variant_size(&entry.variants);
                let (created, created_at) =
                    format_container_created(&entry.configuration.creation_date);
                images.push(LocalContainerImage {
                    runtime,
                    repository,
                    tag,
                    id: short_image_id(&entry.id),
                    size,
                    created,
                    size_bytes,
                    created_at,
                });
            }
            Ok(images)
        }
    }
}

/// Lists local container images via the `docker`, `podman`, or `container` CLI.
pub async fn list_local_images(runtime: LocalRuntime) -> Result<Vec<LocalContainerImage>> {
    let cli = runtime.cli();
    let args: &[&str] = match runtime {
        LocalRuntime::Docker => &["images", "--format", "{{json .}}"],
        LocalRuntime::Podman => &["images", "--format", "json"],
        LocalRuntime::Container => &["image", "list", "--format", "json"],
    };
    let subcmd = match runtime {
        LocalRuntime::Container => format!("{cli} image list"),
        LocalRuntime::Docker | LocalRuntime::Podman => format!("{cli} images"),
    };
    let output = local_cli_command(cli)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| Error::Other {
            message: format!("failed to run `{subcmd}`: {err}"),
        })?;
    if !output.status.success() {
        let detail = command_error_detail(&output);
        return Err(Error::Other {
            message: format!("{subcmd} failed: {detail}"),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_local_images_output(runtime, &stdout)
}

/// Returns `true` when `message` indicates an image is still used by a container or multiple tags
/// and requires `--force` to remove.
pub fn is_image_in_use_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "using its referenced image",
        "image is being used by",
        "(must force)",
        "(must be forced)",
        "referenced in multiple repositories",
        "in use by a container",
        "in use by container",
        "currently in use",
        "known by multiple tags",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[derive(Debug, Default, Deserialize)]
struct ContainerPsEntry {
    #[serde(default)]
    configuration: ContainerPsConfiguration,
}

#[derive(Debug, Default, Deserialize)]
struct ContainerPsConfiguration {
    #[serde(default)]
    id: String,
    #[serde(default)]
    image: Option<ContainerPsImage>,
}

#[derive(Debug, Default, Deserialize)]
struct ContainerPsImage {
    #[serde(default)]
    reference: String,
    #[serde(default)]
    descriptor: Option<ContainerPsDescriptor>,
}

#[derive(Debug, Default, Deserialize)]
struct ContainerPsDescriptor {
    #[serde(default)]
    digest: String,
}

fn parse_container_in_use_ids(stdout: &str, target: &str, id: &str) -> Vec<String> {
    let Ok(entries) = serde_json::from_str::<Vec<ContainerPsEntry>>(stdout.trim()) else {
        return Vec::new();
    };
    let normalized_target = normalize_container_name(target);
    let normalized_id = short_image_id(id);
    let mut holders = Vec::new();
    for entry in entries {
        let Some(img) = entry.configuration.image else {
            continue;
        };
        let norm_ref = normalize_container_name(&img.reference);
        let ref_matches = !normalized_target.is_empty() && norm_ref == normalized_target;
        let digest_matches = !normalized_id.is_empty()
            && img
                .descriptor
                .as_ref()
                .is_some_and(|d| short_image_id(&d.digest) == normalized_id);
        if (ref_matches || digest_matches) && !entry.configuration.id.is_empty() {
            holders.push(entry.configuration.id);
        }
    }
    holders
}

async fn ensure_container_image_unused(arg: &str, id: &str) -> Result<()> {
    let Ok(list_out) = local_cli_command("container")
        .args(["list", "--all", "--format", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    else {
        return Ok(());
    };
    if !list_out.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&list_out.stdout);
    let holders = parse_container_in_use_ids(&stdout, arg, id);
    if holders.is_empty() {
        return Ok(());
    }
    Err(Error::Other {
        message: format!(
            "container image delete {arg} failed: image is in use by container {}",
            holders.join(", ")
        ),
    })
}

/// Removes a local container image via the `docker`, `podman`, or `container` CLI.
pub async fn remove_local_image(
    runtime: LocalRuntime,
    target: &str,
    id: &str,
    force: bool,
) -> Result<()> {
    let arg = if !target.trim().is_empty() {
        target.trim()
    } else {
        id.trim()
    };
    if arg.is_empty() {
        return Err(Error::Other {
            message: "missing local image target".to_string(),
        });
    }

    if runtime == LocalRuntime::Container && !force {
        ensure_container_image_unused(arg, id).await?;
    }

    let cli = runtime.cli();
    let mut cmd = local_cli_command(cli);
    let subcmd = match runtime {
        LocalRuntime::Docker => {
            cmd.args(["image", "rm"]);
            "docker image rm"
        }
        LocalRuntime::Podman => {
            cmd.arg("rmi");
            "podman rmi"
        }
        LocalRuntime::Container => {
            cmd.args(["image", "delete"]);
            "container image delete"
        }
    };
    if force {
        cmd.arg("--force");
    }
    let output = cmd
        .arg(arg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| Error::Other {
            message: format!("failed to run `{subcmd}`: {err}"),
        })?;
    if !output.status.success() {
        let detail = command_error_detail(&output);
        return Err(Error::Other {
            message: format!("{subcmd} {arg} failed: {detail}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cache_arch_drops_the_platform_for_single_image_manifests() {
        assert_eq!(cache_arch(Some(MEDIA_TYPE_IMAGE_INDEX), "arm64"), "arm64");
        assert_eq!(cache_arch(Some(MEDIA_TYPE_MANIFEST_LIST), "arm64"), "arm64");
        assert_eq!(cache_arch(None, "arm64"), "arm64");
        assert_eq!(
            cache_arch(Some(MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST), "arm64"),
            ""
        );
        assert_eq!(
            cache_arch(Some("application/vnd.oci.image.manifest.v1+json"), "amd64"),
            ""
        );
    }

    #[test]
    fn parses_docker_ndjson_images_and_filters_dangling() {
        let stdout = r#"
{"Repository":"redis","Tag":"alpine","ID":"sha256:abcdef1234567890","Size":"41.2MB","CreatedSince":"2 weeks ago"}
{"Repository":"<none>","Tag":"<none>","ID":"999999999999","Size":"10MB","CreatedSince":"1 day ago"}
{"Repository":"ghcr.io/org/app","Tag":"<none>","ID":"1111222233334444","Size":"88MB","CreatedAt":"2026-11-01"}
"#;
        let images = parse_local_images_output(LocalRuntime::Docker, stdout).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].id, "abcdef123456");
        assert_eq!(images[0].reference(), "docker://redis:alpine");
        assert_eq!(images[0].created, "2 weeks ago");
        assert_eq!(images[0].size_bytes, Some(41_200_000));
        assert_eq!(images[1].id, "111122223333");
        assert_eq!(images[1].reference(), "docker://111122223333");
        assert_eq!(images[1].created, "2026-11-01");
        assert_eq!(images[1].size_bytes, Some(88_000_000));
        assert!(images[1].created_at.is_some());

        let empty = parse_local_images_output(LocalRuntime::Docker, "  \n ").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn parses_podman_json_images_with_registry_port() {
        let stdout = r#"[
            {
                "Id": "sha256:0123456789abcdef",
                "Names": ["localhost:5000/my/app:1.2.0"],
                "Size": 10485760,
                "Created": 1760000000,
                "CreatedSince": "3 hours ago"
            },
            {
                "Id": "fedcba987654",
                "Names": [],
                "RepoTags": ["<none>:<none>"]
            }
        ]"#;
        let images = parse_local_images_output(LocalRuntime::Podman, stdout).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].repository, "localhost:5000/my/app");
        assert_eq!(images[0].tag, "1.2.0");
        assert_eq!(images[0].id, "0123456789ab");
        assert_eq!(
            images[0].reference(),
            "podman://localhost:5000/my/app:1.2.0"
        );
        assert_eq!(images[0].size_bytes, Some(10_485_760));
        assert_eq!(images[0].created_at, Some(1_760_000_000));
        assert!(!images[0].size.is_empty());
    }

    #[test]
    fn parses_apple_container_json_images_with_attestation_and_multi_arch() {
        let stdout = r#"[
            {
                "id": "718f745deb7d00112233445566778899",
                "configuration": {
                    "name": "docker.io/library/redis:8.10",
                    "creationDate": "2026-08-26T23:03:21Z"
                },
                "variants": [
                    {
                        "size": 55565214,
                        "platform": {"architecture": "amd64", "os": "linux"}
                    },
                    {
                        "size": 2016841,
                        "platform": {"architecture": "unknown", "os": "unknown"}
                    },
                    {
                        "size": 55350198,
                        "platform": {"architecture": "arm64", "os": "linux", "variant": "v8"}
                    }
                ]
            },
            {
                "id": "2345f2c129f2aabbccdd",
                "configuration": {
                    "name": "ghcr.io/fun7257/xai-proxy:v0.1.0",
                    "creationDate": "2026-10-07T21:48:02Z"
                },
                "variants": [
                    {
                        "size": 10485760,
                        "platform": {"architecture": "amd64", "os": "linux"}
                    }
                ]
            },
            {
                "id": "000000000000",
                "configuration": {"name": ""},
                "variants": []
            }
        ]"#;
        let images = parse_local_images_output(LocalRuntime::Container, stdout).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].repository, "redis");
        assert_eq!(images[0].tag, "8.10");
        assert_eq!(images[0].id, "718f745deb7d");
        assert_eq!(images[0].reference(), "container://redis:8.10");
        assert_eq!(images[0].cli_target(), "redis:8.10");
        assert_eq!(images[0].created, "2026-08-26 23:03");
        let expected_bytes = if host_arch() == "arm64" {
            55_350_198
        } else {
            55_565_214
        };
        assert_eq!(images[0].size_bytes, Some(expected_bytes));
        assert_eq!(images[0].size, ByteSize::b(expected_bytes).to_string());
        assert_eq!(images[0].created_at, Some(1_787_785_401));

        assert_eq!(images[1].repository, "ghcr.io/fun7257/xai-proxy");
        assert_eq!(images[1].tag, "v0.1.0");
        assert_eq!(
            images[1].reference(),
            "container://ghcr.io/fun7257/xai-proxy:v0.1.0"
        );
        assert_eq!(images[1].size_bytes, Some(10_485_760));
        assert_eq!(images[1].size, ByteSize::b(10_485_760).to_string());
    }

    #[test]
    fn orders_runtimes_with_installed_first_and_detects_executables() {
        assert_eq!(
            LocalRuntime::ordered_by_installed_with(|rt| rt == LocalRuntime::Container),
            [
                LocalRuntime::Container,
                LocalRuntime::Docker,
                LocalRuntime::Podman
            ]
        );
        assert_eq!(
            LocalRuntime::ordered_by_installed_with(|rt| {
                matches!(rt, LocalRuntime::Podman | LocalRuntime::Container)
            }),
            [
                LocalRuntime::Podman,
                LocalRuntime::Container,
                LocalRuntime::Docker
            ]
        );
        assert_eq!(
            LocalRuntime::ordered_by_installed_with(|_| false),
            [
                LocalRuntime::Docker,
                LocalRuntime::Podman,
                LocalRuntime::Container
            ]
        );

        let dir = tempdir().unwrap();
        let bin_path = dir.path().join("my-runtime");
        fs::write(&bin_path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&bin_path).unwrap().permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&bin_path, perms.clone()).unwrap();
            assert!(!is_executable_in_dirs(
                "my-runtime",
                &[dir.path().to_path_buf()]
            ));
            perms.set_mode(0o755);
            fs::set_permissions(&bin_path, perms).unwrap();
        }
        assert!(is_executable_in_dirs(
            "my-runtime",
            &[dir.path().to_path_buf()]
        ));
        assert!(!is_executable_in_dirs(
            "missing-runtime",
            &[dir.path().to_path_buf()]
        ));
    }

    #[test]
    fn parses_size_bytes_and_created_timestamps_for_sorting() {
        assert_eq!(parse_human_size_bytes("0B"), Some(0));
        assert_eq!(parse_human_size_bytes("46.4 MiB"), Some(48_653_926));
        assert_eq!(parse_human_size_bytes("222.2 MiB"), Some(232_993_587));
        assert_eq!(parse_human_size_bytes("41.2MB"), Some(41_200_000));
        assert_eq!(parse_human_size_bytes("1.5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_human_size_bytes("unknown"), None);

        assert_eq!(
            parse_created_timestamp("2026-10-07T21:48:02Z"),
            Some(1_791_409_682)
        );
        assert_eq!(
            parse_created_timestamp("2026-10-07 21:48:02 +0000 UTC"),
            Some(1_791_409_682)
        );
        assert_eq!(
            parse_created_timestamp("2026-10-07 21:48"),
            Some(1_791_409_680)
        );
        assert_eq!(parse_created_timestamp("2026-10-07"), Some(1_791_331_200));
        let two_hours = parse_created_timestamp("2 hours ago").unwrap();
        let two_days = parse_created_timestamp("2 days ago").unwrap();
        assert!(two_hours > two_days);
        assert_eq!(parse_created_timestamp("not a date"), None);
    }

    #[test]
    fn detects_in_use_errors_and_apple_container_holders() {
        assert!(is_image_in_use_error(
            "Error response from daemon: conflict: unable to remove repository reference \"redis:7\" (must force) - container abc123 is using its referenced image 71da9275c5f3"
        ));
        assert!(is_image_in_use_error(
            "Error: Image used by 1234567890ab: image is being used by a container"
        ));
        assert!(is_image_in_use_error(
            "container image delete mysql:8.4 failed: image is in use by container cyberapi-mysql"
        ));
        assert!(!is_image_in_use_error("No such image: missing:latest"));

        let ps_json = r#"[
            {
                "configuration": {
                    "id": "cyberapi-mysql",
                    "image": {
                        "reference": "docker.io/library/mysql:8.4",
                        "descriptor": {
                            "digest": "sha256:24989179f58856db763f418010c1169fecc944e3ec1ea7e921572d83eb070f70"
                        }
                    }
                }
            },
            {
                "configuration": {
                    "id": "etcd",
                    "image": {
                        "reference": "gcr.io/etcd-development/etcd:v3.6.5",
                        "descriptor": {
                            "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        }
                    }
                }
            }
        ]"#;
        assert_eq!(
            parse_container_in_use_ids(ps_json, "mysql:8.4", "24989179f588"),
            vec!["cyberapi-mysql".to_string()]
        );
        assert_eq!(
            parse_container_in_use_ids(
                ps_json,
                "gcr.io/etcd-development/etcd:v3.6.5",
                "bbbbbbbbbbbb"
            ),
            vec!["etcd".to_string()]
        );
        assert!(parse_container_in_use_ids(ps_json, "redis:7", "71da9275c5f3").is_empty());
    }
}
