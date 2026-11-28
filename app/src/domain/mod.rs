//! Pure domain derivations, async service bridge, and reactive application state.

pub mod model;
pub mod service;
pub mod shell_fmt;
pub mod state;
pub mod treemap;
pub mod updater;

pub use model::{
    AppRoute, DiffFileTab, FileKind, FilePreviewContent, FileProvenanceHit, FileTreeFilter,
    FileTreeRow, ImageSourceKind, IndexedTree, IndexedTreeNode, InspectorTab, LARGE_LAYER_BYTES,
    LayerFileCounts, LayerFilterMode, RecentImageEntry, SharedRef, TreemapViewMode,
    WhatIfProjection, build_indexed_tree, classify_file_entry, classify_file_kind,
    classify_file_preview, classify_layer_opcode, compute_what_if_projection, count_layer_file_ops,
    findings_total_count, flatten_file_tree, flatten_indexed_tree, format_bytes,
    format_command_lines, format_runtime_compat, image_source_kind, layer_matches_filter,
    relocalize_image_diff, remove_recent, search_file_provenance, symlink_preview_content,
    upsert_recent, visible_layer_indices,
};
pub use service::{AnalyzeMessage, AnalyzeTaskHandle, DiffMessage, DiffRunResult, DivingService};
pub use shell_fmt::{ParsedLayerCommand, format_dockerfile, parse_layer_command};
pub use state::{
    AppState, DiffErrorState, PreviewTarget, TreemapHoverInfo, UpdatePhase,
    has_pending_update_indicator,
};
pub use treemap::{
    FsNode, TreemapChildTile, TreemapRect, TreemapTile, build_final_tree, dominant_layer,
    find_fs_node, layer_bytes, layout_treemap_tiles, slot_for_layer, squarify, top_three_layers,
};
