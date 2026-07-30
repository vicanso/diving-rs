//! Docker/OCI image reference parsing.
//!
//! Accepts the same forms as the CLI: `name:tag`, `user/name:tag`,
//! `registry:port/user/name:tag`, nested namespaces (`ghcr.io/a/b/c:tag`),
//! `file://` / `docker://` protocols, and `?arch=` query params.

/// Default Docker Hub registry (v2 API base).
pub const REGISTRY_DOCKER_HUB: &str = "https://index.docker.io/v2";
pub const REGISTRY_LOCAL_FILE: &str = "local-file";
pub const REGISTRY_LOCAL_DOCKER: &str = "local-docker";

static FILE_PROTOCOL: &str = "file://";
static LOCAL_DOCKER_PROTOCOL: &str = "docker://";

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
    if value.starts_with(FILE_PROTOCOL) {
        return ImageInfo {
            registry: REGISTRY_LOCAL_FILE.to_string(),
            name: value.replace(FILE_PROTOCOL, ""),
            ..Default::default()
        };
    }
    if value.starts_with(LOCAL_DOCKER_PROTOCOL) {
        return ImageInfo {
            registry: REGISTRY_LOCAL_DOCKER.to_string(),
            name: value.replace(LOCAL_DOCKER_PROTOCOL, ""),
            ..Default::default()
        };
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
        let info = parse_image_info("docker://redis:alpine");
        assert_eq!(info.registry, REGISTRY_LOCAL_DOCKER);
        assert_eq!(info.name, "redis:alpine");
    }

    #[test]
    fn test_parse_image_info_arch_query_param() {
        let info = parse_image_info("redis:alpine?arch=arm64");
        assert_eq!(info.arch, "arm64");
        assert_eq!(info.tag, "alpine");
        assert_eq!(info.name, "redis");
    }
}
