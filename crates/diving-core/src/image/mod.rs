mod elf;
mod image_ref;
mod layer;
mod local;
mod model;
mod oci_image;
mod pipeline;
mod registry;
mod registry_auth;
mod scan;

pub use elf::{RuntimeCompat, UnstrippedBinary};
pub(crate) use image_ref::normalize_arch;
pub use image_ref::{
    host_arch, is_local_registry, parse_image_info, ImageInfo, REGISTRY_DOCKER_HUB,
    REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE, REGISTRY_LOCAL_PODMAN,
};
pub use layer::{get_file_content_from_layer, get_file_head_from_layer, stream_file_from_layer};
pub(crate) use layer::{hash_files_from_layer, TarIndex};
pub use local::{list_local_images, parse_local_images_output, LocalContainerImage, LocalRuntime};
pub use model::{
    AnalyzeReport, BigModifiedFileInfo, DockerAnalyzeResult, DockerAnalyzeSummary,
    ImageFileWastedSummary, ImagePreview, SensitiveFileInfo, WasteChange,
};
pub use oci_image::{
    final_filesystem, DuplicateGroup, DuplicatePath, FileTreeItem, FinalFile, ImageFileInfo,
    ImageFileSummary, ImageLayer, Op,
};
pub(crate) use pipeline::{
    analyze_once, analyze_or_last_known, preview_image, PipelineContext, PipelineOptions,
};
pub use registry::is_private_or_loopback_host;
pub(crate) use registry::RegistryCaches;
pub(crate) use registry_auth::{load_docker_config, DockerConfig};
pub use registry_auth::{registry_host, resolve_explicit, RegistryCredentials};
