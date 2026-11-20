use super::image_ref::{host_arch, platform_matches};
use super::layer::TarIndex;
use super::oci_image::{
    ImageManifest, ImageManifestConfig, ImageManifestLayer, MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
    MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_MANIFEST_LIST, MEDIA_TYPE_OCI_MANIFEST,
};
use crate::error::{Error, IoSnafu, JsonSnafu, Result};
use crate::tl_info;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::fs::File;
use std::path::Path;
use std::process::Stdio;

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

pub(crate) async fn save_local_image_to_file(cli: &str, image: &str, path: &Path) -> Result<()> {
    tl_info!(image = image, cli = cli, "saving image");
    let file = File::create(path).context(IoSnafu {})?;
    let status = tokio::process::Command::new(cli)
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
}
