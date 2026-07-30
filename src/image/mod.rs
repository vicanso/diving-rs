mod docker;
mod elf;
mod image_ref;
mod layer;
mod oci_image;
mod registry_auth;
mod sensitive;

pub use docker::{
    analyze_docker_image, DockerAnalyzeResult, DockerAnalyzeSummary, SensitiveFileInfo,
};
pub use elf::RuntimeCompat;
pub use image_ref::{parse_image_info, ImageInfo, REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE};
pub use layer::{get_file_content_from_layer, get_files_from_layer, get_os_release_from_layer};
pub use oci_image::{
    convert_files_to_file_tree, FileTreeItem, ImageConfig, ImageFileInfo, ImageIndex, ImageLayer,
    ImageManifest, ImageManifestConfig, Op, MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
    MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_MANIFEST_LIST,
};
pub use registry_auth::{registry_host, resolve_explicit, RegistryCredentials};
pub(crate) use sensitive::has_path_frag;
