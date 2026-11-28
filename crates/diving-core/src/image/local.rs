use super::image_ref::{host_arch, platform_matches};
use super::layer::TarIndex;
use super::oci_image::{
    ImageManifest, ImageManifestConfig, ImageManifestLayer, MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
    MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_MANIFEST_LIST, MEDIA_TYPE_OCI_MANIFEST,
};
use crate::error::{Error, IoSnafu, JsonSnafu, Result};
use crate::tl_info;
use bytesize::ByteSize;
use chrono::DateTime;
use home::home_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snafu::ResultExt;
use std::env;
use std::fs::File;
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

fn local_cli_command(cli: &str) -> Command {
    let mut cmd = Command::new(cli);
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
}

impl LocalContainerImage {
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

fn format_container_created(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    trimmed.to_string()
}

fn normalize_container_name(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("docker.io/library/")
        .or_else(|| trimmed.strip_prefix("docker.io/"))
        .unwrap_or(trimmed)
}

fn pick_container_variant_size(variants: &[ContainerVariant]) -> String {
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
    chosen
        .and_then(|v| v.size)
        .map(|bytes| ByteSize::b(bytes).to_string())
        .unwrap_or_default()
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

fn format_podman_size(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|bytes| ByteSize::b(bytes).to_string())
            .unwrap_or_default(),
        _ => String::new(),
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
                    row.created_at
                } else {
                    row.created_since
                };
                images.push(LocalContainerImage {
                    runtime,
                    repository: row.repository,
                    tag: row.tag,
                    id: short_image_id(&row.id),
                    size: row.size,
                    created,
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
                let size = format_podman_size(row.size.as_ref());
                let created = if row.created_since.is_empty() {
                    row.created_at
                } else {
                    row.created_since
                };
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
                images.push(LocalContainerImage {
                    runtime,
                    repository,
                    tag,
                    id: short_image_id(&entry.id),
                    size: pick_container_variant_size(&entry.variants),
                    created: format_container_created(&entry.configuration.creation_date),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(images[1].id, "111122223333");
        assert_eq!(images[1].reference(), "docker://111122223333");
        assert_eq!(images[1].created, "2026-11-01");

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
        assert_eq!(images[0].created, "2026-08-26 23:03");
        let expected_first = if host_arch() == "arm64" {
            ByteSize::b(55350198).to_string()
        } else {
            ByteSize::b(55565214).to_string()
        };
        assert_eq!(images[0].size, expected_first);

        assert_eq!(images[1].repository, "ghcr.io/fun7257/xai-proxy");
        assert_eq!(images[1].tag, "v0.1.0");
        assert_eq!(
            images[1].reference(),
            "container://ghcr.io/fun7257/xai-proxy:v0.1.0"
        );
        assert_eq!(images[1].size, ByteSize::b(10485760).to_string());
    }
}
