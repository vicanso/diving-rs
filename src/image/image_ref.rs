//! Docker/OCI image reference parsing.
//!
//! Accepts the same forms as the CLI: `name:tag`, `user/name:tag`,
//! `registry:port/user/name:tag`, nested namespaces (`ghcr.io/a/b/c:tag`),
//! `file://` / `docker://` / `podman://` protocols, and `?arch=` query params.

/// Default Docker Hub registry (v2 API base).
pub const REGISTRY_DOCKER_HUB: &str = "https://index.docker.io/v2";
pub const REGISTRY_LOCAL_FILE: &str = "local-file";
pub const REGISTRY_LOCAL_DOCKER: &str = "local-docker";
pub const REGISTRY_LOCAL_PODMAN: &str = "local-podman";

static FILE_PROTOCOL: &str = "file://";
static LOCAL_DOCKER_PROTOCOL: &str = "docker://";
static LOCAL_PODMAN_PROTOCOL: &str = "podman://";

/// The platform an image is analyzed for when the reference names none:
/// the host's, as `docker pull` would pick.
pub fn host_arch() -> &'static str {
    let arch = std::env::consts::ARCH;
    if arch.contains("aarch64") || arch.contains("arm") {
        "arm64"
    } else {
        "amd64"
    }
}

/// An architecture as OCI platforms spell it: trimmed, lowercased, and the
/// kernel's names (`x86_64`, `aarch64`) mapped to Go's (`amd64`, `arm64`),
/// so every spelling of one platform selects, and caches, the same image.
/// A variant stays (`arm/v7`), written as [`platform_name`] writes it.
pub fn normalize_arch(arch: &str) -> String {
    let arch = arch.trim().to_ascii_lowercase();
    let (arch, variant) = match arch.split_once('/') {
        Some((arch, variant)) => (arch, Some(variant)),
        None => (arch.as_str(), None),
    };
    let arch = match arch {
        "x86_64" | "x86-64" => "amd64",
        "aarch64" => "arm64",
        _ => arch,
    };
    platform_name(arch, variant)
}

/// A platform as `?arch=` and `supported_archs` write it: the architecture,
/// plus the variant where it tells images apart (`arm/v6` vs `arm/v7`).
/// `arm64/v8` is the only arm64 there is, so it is plain `arm64`.
pub fn platform_name(architecture: &str, variant: Option<&str>) -> String {
    match variant {
        Some(v) if !v.is_empty() && !(architecture == "arm64" && v == "v8") => {
            format!("{architecture}/{v}")
        }
        _ => architecture.to_string(),
    }
}

/// Whether a manifest for `architecture` / `variant` is the platform
/// `wanted` names: the architecture must match, and so must the variant
/// when `wanted` has one (`arm/v7`; plain `arm` takes any arm).
pub fn platform_matches(wanted: &str, architecture: &str, variant: Option<&str>) -> bool {
    let (arch, wanted_variant) = match wanted.split_once('/') {
        Some((arch, variant)) => (arch, Some(variant)),
        None => (wanted, None),
    };
    arch == architecture && wanted_variant.is_none_or(|v| variant == Some(v))
}

/// Whether `registry` is one of the local-source markers (`file://`,
/// `docker://`, `podman://`) rather than a registry URL.
pub fn is_local_registry(registry: &str) -> bool {
    matches!(
        registry,
        REGISTRY_LOCAL_FILE | REGISTRY_LOCAL_DOCKER | REGISTRY_LOCAL_PODMAN
    )
}

#[derive(Debug, Clone, Default)]
pub struct ImageInfo {
    /// Registry base URL (`https://host/v2`) or a local protocol marker.
    pub registry: String,
    /// Namespace / org (empty for single-segment custom-registry repos).
    pub user: String,
    /// Repository name (may contain `/` for nested namespaces).
    pub name: String,
    pub tag: String,
    pub arch: String,
}

/// Whether the first path component looks like a registry host (not a
/// Docker Hub library / user name). Matches docker/distribution rules:
/// contains `.`, contains `:` (host:port), or is `localhost`.
fn looks_like_registry_host(host: &str) -> bool {
    host == "localhost"
        || host.starts_with("localhost:")
        || host.contains('.')
        || host.contains(':')
}

/// Split `name[:tag]` where the tag colon only counts inside the last
/// path component — so `localhost:5000/user/img:tag` keeps the port.
fn split_name_and_tag(reference: &str) -> (String, String) {
    // `name@sha256:…` (also `name:tag@sha256:…`): the digest is what the
    // registry is asked for; a tag beside it is only a label. Splitting on
    // the last `:` instead asked for repository `name@sha256`.
    if let Some((name, digest)) = reference.split_once('@') {
        return (split_name_and_tag(name).0, digest.to_string());
    }
    let last_slash = reference.rfind('/').map(|i| i + 1).unwrap_or(0);
    let after_slash = &reference[last_slash..];
    if let Some(colon) = after_slash.rfind(':') {
        let tag = &after_slash[colon + 1..];
        if !tag.is_empty() {
            let name = format!("{}{}", &reference[..last_slash], &after_slash[..colon]);
            return (name, tag.to_string());
        }
    }
    (reference.to_string(), "latest".to_string())
}

/// Build the repository path used in registry URLs (`user/name` or just
/// `name` when the namespace is empty). Nested names keep internal `/`.
pub fn repository_path(user: &str, name: &str) -> String {
    if user.is_empty() {
        name.to_string()
    } else {
        format!("{user}/{name}")
    }
}

pub fn parse_image_info(image: &str) -> ImageInfo {
    let mut value = image.to_string();
    if let Some(path) = value.strip_prefix(FILE_PROTOCOL) {
        // `?arch=` picks a platform out of a multi-arch OCI archive.
        let (path, arch) = match path.rsplit_once("?arch=") {
            Some((path, arch)) => (path, arch),
            None => (path, ""),
        };
        return ImageInfo {
            registry: REGISTRY_LOCAL_FILE.to_string(),
            name: path.to_string(),
            arch: normalize_arch(arch),
            ..Default::default()
        };
    }
    for (protocol, registry) in [
        (LOCAL_DOCKER_PROTOCOL, REGISTRY_LOCAL_DOCKER),
        (LOCAL_PODMAN_PROTOCOL, REGISTRY_LOCAL_PODMAN),
    ] {
        if let Some(name) = value.strip_prefix(protocol) {
            return ImageInfo {
                registry: registry.to_string(),
                name: name.to_string(),
                ..Default::default()
            };
        }
    }
    let mut arch = "".to_string();
    if let Some(index) = value.find('?') {
        let query = &value[index + 1..];
        for item in query.split('&') {
            let arr: Vec<&str> = item.split('=').collect();
            if arr.len() == 2 && arr[0] == "arch" {
                arch = arr[1].to_string();
            }
        }
        value = value[..index].to_string();
    }
    // A registry image always names its platform: no `?arch=` means the
    // host's, which is what the manifest list resolves to anyway. Keeping
    // it implicit split one image across two cache keys (`""` vs `arm64`).
    let arch = match normalize_arch(&arch) {
        arch if arch.is_empty() => host_arch().to_string(),
        arch => arch,
    };

    let (path, tag) = split_name_and_tag(&value);
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        // Degenerate / empty input still resolves to Docker Hub library
        // so callers (e.g. AI history keys) get a stable, non-empty identity.
        return ImageInfo {
            registry: REGISTRY_DOCKER_HUB.to_string(),
            user: "library".to_string(),
            name: String::new(),
            tag,
            arch,
        };
    }

    let first = parts[0];
    let has_registry = looks_like_registry_host(first);
    let (registry, repo_parts): (String, &[&str]) = if has_registry && parts.len() >= 2 {
        (format!("https://{first}/v2"), &parts[1..])
    } else {
        (REGISTRY_DOCKER_HUB.to_string(), &parts[..])
    };

    let (user, name) = match repo_parts {
        [] => ("library".to_string(), String::new()),
        [single] => {
            if registry == REGISTRY_DOCKER_HUB {
                ("library".to_string(), single.to_string())
            } else {
                (String::new(), single.to_string())
            }
        }
        [ns, rest @ ..] => (ns.to_string(), rest.join("/")),
    };

    ImageInfo {
        registry,
        user,
        name,
        tag,
        arch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_image_info_simple() {
        let info = parse_image_info("redis:alpine");
        assert_eq!(info.name, "redis");
        assert_eq!(info.tag, "alpine");
        assert_eq!(info.user, "library");
        assert_eq!(info.registry, REGISTRY_DOCKER_HUB);
    }

    #[test]
    fn test_parse_image_info_no_tag_defaults_to_latest() {
        let info = parse_image_info("redis");
        assert_eq!(info.name, "redis");
        assert_eq!(info.tag, "latest");
    }

    #[test]
    fn test_parse_image_info_with_user() {
        let info = parse_image_info("vicanso/diving:v1.0");
        assert_eq!(info.user, "vicanso");
        assert_eq!(info.name, "diving");
        assert_eq!(info.tag, "v1.0");
    }

    #[test]
    fn test_parse_image_info_with_registry() {
        let info = parse_image_info("registry.example.com/user/image:v2.3");
        assert_eq!(info.registry, "https://registry.example.com/v2");
        assert_eq!(info.user, "user");
        assert_eq!(info.name, "image");
        assert_eq!(info.tag, "v2.3");
    }

    #[test]
    fn test_parse_image_info_nested_namespace() {
        let info = parse_image_info("ghcr.io/org/project/image:v1");
        assert_eq!(info.registry, "https://ghcr.io/v2");
        assert_eq!(info.user, "org");
        assert_eq!(info.name, "project/image");
        assert_eq!(info.tag, "v1");
        assert_eq!(repository_path(&info.user, &info.name), "org/project/image");
    }

    #[test]
    fn test_parse_image_info_registry_with_port() {
        let info = parse_image_info("localhost:5000/user/img:tag");
        assert_eq!(info.registry, "https://localhost:5000/v2");
        assert_eq!(info.user, "user");
        assert_eq!(info.name, "img");
        assert_eq!(info.tag, "tag");
    }

    #[test]
    fn test_parse_image_info_registry_port_no_tag() {
        let info = parse_image_info("my.registry:443/team/app");
        assert_eq!(info.registry, "https://my.registry:443/v2");
        assert_eq!(info.user, "team");
        assert_eq!(info.name, "app");
        assert_eq!(info.tag, "latest");
    }

    #[test]
    fn test_parse_image_info_single_segment_on_custom_registry() {
        let info = parse_image_info("registry.example.com/solo:v1");
        assert_eq!(info.registry, "https://registry.example.com/v2");
        assert_eq!(info.user, "");
        assert_eq!(info.name, "solo");
        assert_eq!(info.tag, "v1");
        assert_eq!(repository_path(&info.user, &info.name), "solo");
    }

    #[test]
    fn test_parse_image_info_file_protocol() {
        let info = parse_image_info("file:///tmp/image.tar");
        assert_eq!(info.registry, REGISTRY_LOCAL_FILE);
        assert_eq!(info.name, "/tmp/image.tar");
    }

    #[test]
    fn test_parse_image_info_docker_protocol() {
        let info = parse_image_info("file:///tmp/app-oci.tar?arch=arm64");
        assert_eq!(info.registry, REGISTRY_LOCAL_FILE);
        assert_eq!(
            (info.name.as_str(), info.arch.as_str()),
            ("/tmp/app-oci.tar", "arm64")
        );
        let info = parse_image_info("file:///tmp/app.tar");
        assert_eq!(
            (info.name.as_str(), info.arch.as_str()),
            ("/tmp/app.tar", "")
        );
        let info = parse_image_info("podman://localhost/app:dev");
        assert_eq!(info.registry, REGISTRY_LOCAL_PODMAN);
        assert_eq!(info.name, "localhost/app:dev");
        assert!(is_local_registry(&info.registry));
        assert!(!is_local_registry(REGISTRY_DOCKER_HUB));
        let info = parse_image_info("docker://redis:alpine");
        assert_eq!(info.registry, REGISTRY_LOCAL_DOCKER);
        assert_eq!(info.name, "redis:alpine");
    }

    #[test]
    fn platforms_are_written_one_way() {
        assert_eq!(normalize_arch("ARM/V7"), "arm/v7");
        assert_eq!(normalize_arch("arm64/v8"), "arm64");
        assert_eq!(normalize_arch("aarch64/v8"), "arm64");
        assert_eq!(normalize_arch(" x86_64 "), "amd64");
        assert_eq!(platform_name("arm", Some("v6")), "arm/v6");
        assert_eq!(platform_name("arm64", Some("v8")), "arm64");
        assert_eq!(platform_name("amd64", None), "amd64");
        assert!(platform_matches("arm/v7", "arm", Some("v7")));
        assert!(!platform_matches("arm/v7", "arm", Some("v6")));
        assert!(platform_matches("arm", "arm", Some("v6")));
        assert!(platform_matches("arm64", "arm64", Some("v8")));
        assert!(!platform_matches("arm64", "amd64", None));
    }

    #[test]
    fn registry_images_always_name_their_platform() {
        // No `?arch=`: the host's, as the manifest list resolves it, so a
        // bare reference and `?arch=<host>` are one cache key.
        assert_eq!(parse_image_info("redis:alpine").arch, host_arch());
        assert_eq!(parse_image_info("ghcr.io/org/app:1").arch, host_arch());
        // Kernel spellings and case map to the OCI names.
        assert_eq!(parse_image_info("redis?arch=x86_64").arch, "amd64");
        assert_eq!(parse_image_info("redis?arch=AARCH64").arch, "arm64");
        // Local sources keep "unspecified": the archive itself decides.
        assert_eq!(parse_image_info("file:///tmp/a.tar").arch, "");
        assert_eq!(parse_image_info("file:///tmp/a?arch=aarch64").arch, "arm64");
        assert_eq!(parse_image_info("docker://app:dev").arch, "");
    }

    #[test]
    fn digest_references_ask_for_the_digest() {
        let info = parse_image_info("alpine@sha256:abc");
        assert_eq!(
            (info.user.as_str(), info.name.as_str(), info.tag.as_str()),
            ("library", "alpine", "sha256:abc")
        );
        let info = parse_image_info("registry.example.com:5000/team/app:1.2@sha256:abc?arch=arm64");
        assert_eq!(info.registry, "https://registry.example.com:5000/v2");
        assert_eq!(
            (info.user.as_str(), info.name.as_str(), info.tag.as_str()),
            ("team", "app", "sha256:abc")
        );
        assert_eq!(info.arch, "arm64");
    }

    #[test]
    fn test_parse_image_info_arch_query_param() {
        let info = parse_image_info("redis:alpine?arch=arm64");
        assert_eq!(info.arch, "arm64");
        assert_eq!(info.tag, "alpine");
        assert_eq!(info.name, "redis");
    }
}
