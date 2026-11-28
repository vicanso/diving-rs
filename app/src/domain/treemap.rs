//! Pure final-filesystem reconstruction, top-3 layer attribution, and squarified Treemap layout.

use std::collections::BTreeMap;

use diving_core::{FileTreeItem, Op};

/// Reconstructed node of the container's final visible filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsNode {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub layer: Option<usize>,
    pub children: BTreeMap<String, FsNode>,
}

impl FsNode {
    fn new_dir(name: String, path: String) -> Self {
        Self {
            name,
            path,
            size: 0,
            layer: None,
            children: BTreeMap::new(),
        }
    }

    pub fn is_dir(&self) -> bool {
        !self.children.is_empty()
    }
}

/// Merges per-layer file trees into the final visible container filesystem.
pub fn build_final_tree(file_tree_list: &[Vec<FileTreeItem>]) -> FsNode {
    fn collect_layer(
        items: &[FileTreeItem],
        segments: &[String],
        removed: &mut Vec<Vec<String>>,
        added: &mut Vec<(Vec<String>, u64)>,
    ) {
        for item in items {
            let next: Vec<String> = if segments.is_empty() && item.name == "." {
                Vec::new()
            } else {
                let mut v = segments.to_vec();
                v.push(item.name.clone());
                v
            };

            if item.op == Op::Removed && !next.is_empty() && !item.name.is_empty() {
                removed.push(next.clone());
            }

            if !item.children.is_empty() {
                collect_layer(&item.children, &next, removed, added);
            } else if next.is_empty() || item.name.is_empty() {
                // Opaque whiteout marker on root itself — skip.
            } else if item.op != Op::Removed {
                added.push((next, item.size));
            }
        }
    }

    fn delete_path(node: &mut FsNode, segments: &[String]) {
        if segments.is_empty() {
            return;
        }
        let mut cursor = node;
        for seg in &segments[..segments.len() - 1] {
            let Some(child) = cursor.children.get_mut(seg) else {
                return;
            };
            cursor = child;
        }
        if let Some(last) = segments.last() {
            cursor.children.remove(last);
        }
    }

    fn insert_file(node: &mut FsNode, segments: &[String], size: u64, layer: usize) {
        if segments.is_empty() {
            return;
        }
        let mut cursor = node;
        for idx in 0..(segments.len() - 1) {
            let name = &segments[idx];
            let sub_path = segments[..=idx].join("/");
            let needs_dir = cursor.children.get(name).is_none_or(|c| c.layer.is_some());
            if needs_dir {
                cursor
                    .children
                    .insert(name.clone(), FsNode::new_dir(name.clone(), sub_path));
            }
            cursor = cursor.children.get_mut(name).expect("inserted directory");
            cursor.layer = None;
        }
        let file_name = segments[segments.len() - 1].clone();
        cursor.children.insert(
            file_name.clone(),
            FsNode {
                name: file_name,
                path: segments.join("/"),
                size,
                layer: Some(layer),
                children: BTreeMap::new(),
            },
        );
    }

    fn rollup(node: &mut FsNode) -> u64 {
        if node.children.is_empty() {
            return node.size;
        }
        let mut total = 0u64;
        for child in node.children.values_mut() {
            total += rollup(child);
        }
        node.size = total;
        total
    }

    let mut root = FsNode::new_dir("/".to_owned(), String::new());
    for (layer_idx, tree) in file_tree_list.iter().enumerate() {
        let mut removed = Vec::new();
        let mut added = Vec::new();
        collect_layer(tree, &[], &mut removed, &mut added);
        for segs in removed {
            delete_path(&mut root, &segs);
        }
        for (segs, size) in added {
            insert_file(&mut root, &segs, size, layer_idx);
        }
    }
    rollup(&mut root);
    root
}

/// Resolves a node along breadcrumb `segments`, falling back cleanly to the deepest valid prefix.
pub fn find_fs_node<'a>(root: &'a FsNode, segments: &[String]) -> &'a FsNode {
    let mut cursor = root;
    for seg in segments {
        let Some(child) = cursor.children.get(seg) else {
            break;
        };
        cursor = child;
    }
    cursor
}

/// Accumulates visible byte counts per 0-based layer index inside `node`.
pub fn layer_bytes(node: &FsNode) -> BTreeMap<usize, u64> {
    fn walk(n: &FsNode, out: &mut BTreeMap<usize, u64>) {
        if n.children.is_empty() {
            if let Some(layer) = n.layer {
                *out.entry(layer).or_insert(0) += n.size;
            }
            return;
        }
        for child in n.children.values() {
            walk(child, out);
        }
    }
    let mut out = BTreeMap::new();
    walk(node, &mut out);
    out
}

/// Returns `(layer_idx, share_0_to_1)` for the layer owning the most bytes inside `node`.
pub fn dominant_layer(node: &FsNode) -> Option<(usize, f32)> {
    if node.size == 0 {
        return None;
    }
    let bytes = layer_bytes(node);
    bytes
        .into_iter()
        .max_by(|(la, ba), (lb, bb)| ba.cmp(bb).then_with(|| lb.cmp(la)))
        .map(|(layer, count)| (layer, (count as f64 / node.size as f64) as f32))
}

/// Top three layers (0-based index, visible bytes) ordered descending by byte contribution.
pub fn top_three_layers(root: &FsNode) -> Vec<(usize, u64)> {
    let mut pairs: Vec<(usize, u64)> = layer_bytes(root)
        .into_iter()
        .filter(|&(_, bytes)| bytes > 0)
        .collect();
    pairs.sort_by(|(la, ba), (lb, bb)| bb.cmp(ba).then_with(|| la.cmp(lb)));
    pairs.truncate(3);
    pairs
}

/// Axis-aligned layout box in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreemapRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Bruls-Huizing-van Wijk squarified treemap layout for descending positive `values`.
pub fn squarify(values: &[f64], w: f32, h: f32) -> Vec<TreemapRect> {
    let total: f64 = values.iter().sum();
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return vec![
            TreemapRect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            };
            values.len()
        ];
    }

    fn worst(row: &[f64], side: f64) -> f64 {
        let sum: f64 = row.iter().sum();
        if sum <= 0.0 || side <= 0.0 {
            return f64::INFINITY;
        }
        let mut max_v = 0.0f64;
        let mut min_v = f64::INFINITY;
        for &v in row {
            max_v = max_v.max(v);
            min_v = min_v.min(v);
        }
        if min_v <= 0.0 {
            return f64::INFINITY;
        }
        let s2 = side * side;
        let sum2 = sum * sum;
        ((s2 * max_v) / sum2).max(sum2 / (s2 * min_v))
    }

    let scale = (w as f64 * h as f64) / total;
    let areas: Vec<f64> = values.iter().map(|v| (*v * scale).max(0.0)).collect();
    let mut rects = Vec::with_capacity(values.len());
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    let mut width = w as f64;
    let mut height = h as f64;
    let mut i = 0usize;

    while i < areas.len() {
        if areas[i] <= 0.0 {
            rects.push(TreemapRect {
                x: x as f32,
                y: y as f32,
                w: 0.0,
                h: 0.0,
            });
            i += 1;
            continue;
        }
        let side = width.min(height).max(1e-6);
        let mut row = vec![areas[i]];
        let mut j = i + 1;
        while j < areas.len() && areas[j] > 0.0 {
            let mut next_row = row.clone();
            next_row.push(areas[j]);
            if worst(&next_row, side) <= worst(&row, side) {
                row = next_row;
                j += 1;
            } else {
                break;
            }
        }

        let sum: f64 = row.iter().sum();
        if width >= height {
            let col_w = (sum / height.max(1e-6)).min(width);
            let mut cy = y;
            for area in row {
                let rh = area / col_w.max(1e-6);
                rects.push(TreemapRect {
                    x: x as f32,
                    y: cy as f32,
                    w: col_w as f32,
                    h: rh as f32,
                });
                cy += rh;
            }
            x += col_w;
            width = (width - col_w).max(0.0);
        } else {
            let row_h = (sum / width.max(1e-6)).min(height);
            let mut cx = x;
            for area in row {
                let rw = area / row_h.max(1e-6);
                rects.push(TreemapRect {
                    x: cx as f32,
                    y: y as f32,
                    w: rw as f32,
                    h: row_h as f32,
                });
                cx += rw;
            }
            y += row_h;
            height = (height - row_h).max(0.0);
        }
        i = j;
    }

    rects
}

/// Level-2 child tile nested inside an enlarged parent directory tile.
#[derive(Debug, Clone, PartialEq)]
pub struct TreemapChildTile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub layer: Option<usize>,
    pub layer_share: f32,
    pub color_slot: usize,
    pub rect: TreemapRect,
}

/// Level-1 treemap block plus optional level-2 nested tiles.
#[derive(Debug, Clone, PartialEq)]
pub struct TreemapTile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub layer: Option<usize>,
    pub layer_share: f32,
    pub color_slot: usize,
    pub rect: TreemapRect,
    pub children: Vec<TreemapChildTile>,
}

/// Resolves the legend/treemap color slot (`1..=3` for top layers, `0` for others).
pub fn slot_for_layer(layer: Option<usize>, top_layers: &[usize]) -> usize {
    layer
        .and_then(|idx| top_layers.iter().position(|top| *top == idx))
        .map_or(0, |pos| pos + 1)
}

fn sorted_nonzero_children(node: &FsNode) -> Vec<&FsNode> {
    let mut children: Vec<&FsNode> = node.children.values().filter(|c| c.size > 0).collect();
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    children
}

/// Computes level-1 squarified treemap tiles and nested level-2 tiles inside spacious folders.
pub fn layout_treemap_tiles(
    node: &FsNode,
    w: f32,
    h: f32,
    top_layers: &[usize],
) -> Vec<TreemapTile> {
    let items = sorted_nonzero_children(node);
    if items.is_empty() {
        return Vec::new();
    }
    let values: Vec<f64> = items.iter().map(|c| c.size as f64).collect();
    let rects = squarify(&values, w, h);

    items
        .into_iter()
        .zip(rects)
        .map(|(child, rect)| {
            let dom = dominant_layer(child);
            let layer = dom.map(|(l, _)| l).or(child.layer);
            let layer_share = dom.map_or(1.0, |(_, share)| share);
            let color_slot = slot_for_layer(layer, top_layers);

            let children = if child.is_dir() && rect.w >= 84.0 && rect.h >= 58.0 {
                let inner_w = (rect.w - 4.0).max(0.0);
                let inner_h = (rect.h - 22.0).max(0.0);
                let sub_items = sorted_nonzero_children(child);
                let sub_values: Vec<f64> = sub_items.iter().map(|c| c.size as f64).collect();
                let sub_rects = squarify(&sub_values, inner_w, inner_h);
                sub_items
                    .into_iter()
                    .zip(sub_rects)
                    .filter(|(_, r)| r.w >= 4.0 && r.h >= 4.0)
                    .map(|(grand, r)| {
                        let g_dom = dominant_layer(grand);
                        let g_layer = g_dom.map(|(l, _)| l).or(grand.layer);
                        let g_share = g_dom.map_or(1.0, |(_, s)| s);
                        TreemapChildTile {
                            name: grand.name.clone(),
                            path: grand.path.clone(),
                            size: grand.size,
                            is_dir: grand.is_dir(),
                            layer: g_layer,
                            layer_share: g_share,
                            color_slot: slot_for_layer(g_layer, top_layers),
                            rect: TreemapRect {
                                x: r.x + 2.0,
                                y: r.y + 20.0,
                                w: r.w,
                                h: r.h,
                            },
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

            TreemapTile {
                name: child.name.clone(),
                path: child.path.clone(),
                size: child.size,
                is_dir: child.is_dir(),
                layer,
                layer_share,
                color_slot,
                rect,
                children,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, op: Op) -> FileTreeItem {
        FileTreeItem {
            name: name.to_owned(),
            size,
            op,
            ..Default::default()
        }
    }

    fn dir(name: &str, size: u64, op: Op, children: Vec<FileTreeItem>) -> FileTreeItem {
        FileTreeItem {
            name: name.to_owned(),
            size,
            op,
            children,
            ..Default::default()
        }
    }

    #[test]
    fn build_final_tree_applies_whiteouts_overwrites_and_sizes() {
        let layer0 = vec![
            dir(
                "usr",
                150,
                Op::None,
                vec![file("keep", 100, Op::None), file("drop", 50, Op::None)],
            ),
            file("cfg", 20, Op::None),
        ];
        let layer1 = vec![
            dir("usr", 0, Op::None, vec![file("drop", 0, Op::Removed)]),
            file("cfg", 40, Op::Modified),
            file("new", 60, Op::None),
        ];

        let root = build_final_tree(&[layer0, layer1]);
        assert_eq!(root.size, 200);
        let usr = root.children.get("usr").expect("usr dir");
        assert_eq!(usr.size, 100);
        assert!(!usr.children.contains_key("drop"));
        assert_eq!(usr.children.get("keep").and_then(|n| n.layer), Some(0));
        assert_eq!(
            root.children.get("cfg").map(|n| (n.size, n.layer)),
            Some((40, Some(1)))
        );
        assert_eq!(
            root.children.get("new").map(|n| (n.size, n.layer)),
            Some((60, Some(1)))
        );

        let top = top_three_layers(&root);
        assert_eq!(top, vec![(0, 100), (1, 100)]);
    }

    #[test]
    fn squarify_fills_viewport_without_out_of_bounds_rects() {
        let rects = squarify(&[60.0, 30.0, 10.0], 300.0, 200.0);
        assert_eq!(rects.len(), 3);
        let mut total_area = 0.0f32;
        for r in rects {
            assert!(r.x >= -0.01 && r.y >= -0.01);
            assert!(r.x + r.w <= 300.05 && r.y + r.h <= 200.05);
            assert!(r.w > 0.0 && r.h > 0.0);
            total_area += r.w * r.h;
        }
        assert!((total_area - 60_000.0).abs() < 1.0);
    }
}
