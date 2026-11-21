//! Pure domain derivations, async service bridge, and reactive application state.

pub mod model;
pub mod service;
pub mod state;
pub mod treemap;

pub use model::{
    AppRoute, DiffFileTab, FilePreviewContent, FileTreeFilter, FileTreeRow, ImageSourceKind,
    IndexedTree, IndexedTreeNode, InspectorTab, LayerFileCounts, LayerFilterMode, RecentImageEntry,
    SharedRef, TreemapViewMode, build_indexed_tree, classify_file_preview, classify_layer_opcode,
    count_layer_file_ops, findings_total_count, flatten_file_tree, flatten_indexed_tree,
    format_bytes, format_command_lines, format_runtime_compat, image_source_kind, remove_recent,
    upsert_recent,
};
pub use service::{AnalyzeMessage, AnalyzeTaskHandle, DiffMessage, DiffRunResult, DivingService};
pub use state::{AppState, DiffErrorState, PreviewTarget, TreemapHoverInfo};
pub use treemap::{
    FsNode, TreemapChildTile, TreemapRect, TreemapTile, build_final_tree, dominant_layer,
    find_fs_node, layer_bytes, layout_treemap_tiles, squarify, top_three_layers,
};
