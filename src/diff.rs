//! Image diff: what a target image adds, removes and grows relative to a
//! base image — typically the same repository one release or one commit
//! earlier. A pure function of the two analyses, like `recommend`.

use bytesize::ByteSize;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

use crate::i18n::{self, Lang};
use crate::image::{final_filesystem, DockerAnalyzeResult, FinalFile, ImageLayer};
use crate::markdown::{clip, icon};
use crate::recommend::Recommendation;

/// Files kept per change kind (largest first); the counts and byte totals
/// always cover every file.
pub const DIFF_MAX_FILES: usize = 50;
/// Directories kept, largest size change first.
pub const DIFF_MAX_DIRS: usize = 20;
/// Rows per table in the Markdown rendering.
const MD_ROWS: usize = 10;
/// Files count toward their directory at most this deep (`usr/local/lib`),
/// so every byte lands in exactly one directory row.
const DIR_DEPTH: usize = 3;

/// The headline numbers of one side of the diff.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSide {
    pub name: String,
    pub arch: String,
    /// Compressed (pull) size.
    pub size: u64,
    /// Uncompressed size, the SIZE `docker images` shows.
    pub total_size: u64,
    /// Layers that carry files; metadata-only layers are left out.
    pub layer_count: usize,
    pub efficiency_score: u64,
    pub wasted_size: u64,
    /// Entries in the final filesystem.
    pub file_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_as_of: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLayer {
    /// 1-based, as numbered in that image's own report.
    pub layer: usize,
    pub digest: String,
    pub cmd: String,
    pub size: u64,
    pub unpack_size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// 0 for an added file.
    pub base_size: u64,
    /// 0 for a removed file.
    pub target_size: u64,
    /// 1-based layer holding the copy: the target's for added and changed
    /// files, the base's for removed ones.
    pub layer: usize,
}

impl FileChange {
    pub fn delta(&self) -> i64 {
        self.target_size as i64 - self.base_size as i64
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    /// Every file of this kind.
    pub count: usize,
    /// Net bytes over all of them, target − base.
    pub bytes: i64,
    /// The largest of them, at most [`DIFF_MAX_FILES`].
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirChange {
    /// `""` for files at the image root.
    pub path: String,
    pub base_size: u64,
    pub target_size: u64,
}

impl DirChange {
    pub fn delta(&self) -> i64 {
        self.target_size as i64 - self.base_size as i64
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDiff {
    pub base: DiffSide,
    pub target: DiffSide,
    /// Uncompressed size change, target − base.
    pub size_delta: i64,
    /// File-carrying layers both images start with (same digests, same
    /// order) — usually the common base image.
    pub shared_layers: usize,
    /// Uncompressed size of the shared layers.
    pub shared_size: u64,
    /// The base's layers after the shared ones.
    pub base_layers: Vec<DiffLayer>,
    /// The target's layers after the shared ones.
    pub target_layers: Vec<DiffLayer>,
    pub added: ChangeSet,
    pub removed: ChangeSet,
    /// Files in both images whose size differs.
    pub changed: ChangeSet,
    /// Directories whose size changed, largest change first.
    pub dirs: Vec<DirChange>,
    /// Recommendations (by id) the target has and the base does not.
    pub new_recommendations: Vec<Recommendation>,
    /// Recommendations (by id) the base had and the target no longer does.
    pub resolved_recommendations: Vec<Recommendation>,
}

fn side(result: &DockerAnalyzeResult, file_count: usize) -> DiffSide {
    let summary = result.summary();
    DiffSide {
        name: result.name.clone(),
        arch: result.arch.clone(),
        size: result.size,
        total_size: result.total_size,
        layer_count: result.layers.iter().filter(|l| !l.empty).count(),
        efficiency_score: summary.score,
        wasted_size: summary.wasted_size,
        file_count,
        stale_as_of: result.stale_as_of.clone(),
    }
}

/// File-carrying layers with their 1-based numbers.
fn file_layers(layers: &[ImageLayer]) -> Vec<(usize, &ImageLayer)> {
    layers
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.empty)
        .map(|(i, l)| (i + 1, l))
        .collect()
}

/// Whether two layers hold the same files: by diff id (the uncompressed
/// tar's digest, recorded by every source) when both have one, else by a
/// content digest. A `docker save` tar may name its layers by path only,
/// which says nothing about their contents.
fn same_layer(a: &ImageLayer, b: &ImageLayer) -> bool {
    if !a.diff_id.is_empty() && !b.diff_id.is_empty() {
        return a.diff_id == b.diff_id;
    }
    a.digest.starts_with("sha256:") && a.digest == b.digest
}

fn diff_layer((layer, l): &(usize, &ImageLayer)) -> DiffLayer {
    DiffLayer {
        layer: *layer,
        digest: l.digest.clone(),
        cmd: l.cmd.clone(),
        size: l.size,
        unpack_size: l.unpack_size,
    }
}

/// The directory a file's bytes count toward: its parent, cut to
/// [`DIR_DEPTH`] segments.
fn dir_bucket(path: &str) -> &str {
    let mut end = 0;
    for (depth, (i, _)) in path.match_indices('/').enumerate() {
        if depth == DIR_DEPTH {
            break;
        }
        end = i;
    }
    &path[..end]
}

fn change_set(mut files: Vec<FileChange>, sort_key: impl Fn(&FileChange) -> u64) -> ChangeSet {
    files.sort_by_key(|f| Reverse(sort_key(f)));
    let bytes = files.iter().map(FileChange::delta).sum();
    let count = files.len();
    files.truncate(DIFF_MAX_FILES);
    ChangeSet {
        count,
        bytes,
        files,
    }
}

fn rec_ids(recs: &[Recommendation]) -> HashSet<&str> {
    recs.iter().map(|r| r.id.as_str()).collect()
}

/// Compare `target` against `base`.
pub fn diff_images(base: &DockerAnalyzeResult, target: &DockerAnalyzeResult) -> ImageDiff {
    let base_fs = final_filesystem(&base.file_tree_list);
    let target_fs = final_filesystem(&target.file_tree_list);

    let base_layers = file_layers(&base.layers);
    let target_layers = file_layers(&target.layers);
    let shared = base_layers
        .iter()
        .zip(&target_layers)
        .take_while(|((_, a), (_, b))| same_layer(a, b))
        .count();

    let mut added = vec![];
    let mut changed = vec![];
    let mut dirs: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for (path, file) in &target_fs {
        dirs.entry(dir_bucket(path)).or_default().1 += file.size;
        match base_fs.get(path) {
            None => added.push(file_change(path, None, Some(file))),
            Some(old) if old.size != file.size => {
                changed.push(file_change(path, Some(old), Some(file)))
            }
            Some(_) => {}
        }
    }
    let mut removed = vec![];
    for (path, file) in &base_fs {
        dirs.entry(dir_bucket(path)).or_default().0 += file.size;
        if !target_fs.contains_key(path) {
            removed.push(file_change(path, Some(file), None));
        }
    }
    let mut dirs: Vec<DirChange> = dirs
        .into_iter()
        .filter(|(_, (b, t))| b != t)
        .map(|(path, (base_size, target_size))| DirChange {
            path: path.to_string(),
            base_size,
            target_size,
        })
        .collect();
    dirs.sort_by_key(|d| Reverse(d.delta().unsigned_abs()));
    dirs.truncate(DIFF_MAX_DIRS);

    let base_ids = rec_ids(&base.recommendations);
    let target_ids = rec_ids(&target.recommendations);
    ImageDiff {
        base: side(base, base_fs.len()),
        target: side(target, target_fs.len()),
        size_delta: target.total_size as i64 - base.total_size as i64,
        shared_layers: shared,
        shared_size: base_layers[..shared]
            .iter()
            .map(|(_, l)| l.unpack_size)
            .sum(),
        base_layers: base_layers[shared..].iter().map(diff_layer).collect(),
        target_layers: target_layers[shared..].iter().map(diff_layer).collect(),
        added: change_set(added, |f| f.target_size),
        removed: change_set(removed, |f| f.base_size),
        changed: change_set(changed, |f| f.delta().unsigned_abs()),
        dirs,
        new_recommendations: target
            .recommendations
            .iter()
            .filter(|r| !base_ids.contains(r.id.as_str()))
            .cloned()
            .collect(),
        resolved_recommendations: base
            .recommendations
            .iter()
            .filter(|r| !target_ids.contains(r.id.as_str()))
            .cloned()
            .collect(),
    }
}

fn file_change(path: &str, base: Option<&FinalFile>, target: Option<&FinalFile>) -> FileChange {
    FileChange {
        path: path.to_string(),
        base_size: base.map_or(0, |f| f.size),
        target_size: target.map_or(0, |f| f.size),
        layer: target.or(base).map_or(0, |f| f.layer + 1),
    }
}

/// `+12.0 MiB` / `-3.1 KiB` / `0 B`.
fn signed_bytes(delta: i64) -> String {
    let size = ByteSize(delta.unsigned_abs());
    match delta {
        d if d > 0 => format!("+{size}"),
        d if d < 0 => format!("-{size}"),
        _ => size.to_string(),
    }
}

fn signed(delta: i64) -> String {
    if delta > 0 {
        format!("+{delta}")
    } else {
        delta.to_string()
    }
}

/// Inline code that survives a table cell: whitespace runs (the tabs and
/// newlines of a `RUN` script) folded to one space, pipes escaped.
fn cell_code(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("`{}`", clip(&flat.replace('|', "\\|"), 120))
}

/// The diff as Markdown: headline table, layers, where the size went, the
/// largest file changes and the recommendation changes. `verdict` is the
/// CI outcome, when there is one.
pub fn to_diff_markdown(diff: &ImageDiff, lang: Lang, verdict: Option<&str>) -> String {
    let t = |k: &str| -> String { i18n::tr(lang, k).to_string() };
    let f = |k: &str, args: &[&str]| -> String { i18n::fill(i18n::tr(lang, k), args) };
    let (base, target) = (&diff.base, &diff.target);
    let mut md = format!("### diving diff · `{}` → `{}`\n\n", base.name, target.name);
    if let Some(verdict) = verdict {
        md.push_str(&format!("{verdict}\n\n"));
    }
    for side in [base, target] {
        if let Some(as_of) = &side.stale_as_of {
            md.push_str(&format!(
                "> ⚠️ `{}`: {}\n\n",
                side.name,
                f("md.stale", &[as_of])
            ));
        }
    }

    let pct = if base.total_size > 0 {
        format!(
            " ({:+.1} %)",
            diff.size_delta as f64 / base.total_size as f64 * 100.0
        )
    } else {
        String::new()
    };
    md.push_str(&format!(
        "| | {} | {} | {} |\n|---|---:|---:|---:|\n",
        t("diff.col.base"),
        t("diff.col.target"),
        t("diff.col.change"),
    ));
    let rows: [(String, String, String, String); 6] = [
        (
            t("sum.size"),
            ByteSize(base.total_size).to_string(),
            ByteSize(target.total_size).to_string(),
            format!("**{}**{pct}", signed_bytes(diff.size_delta)),
        ),
        (
            t("diff.compressed"),
            ByteSize(base.size).to_string(),
            ByteSize(target.size).to_string(),
            signed_bytes(target.size as i64 - base.size as i64),
        ),
        (
            t("sum.layers"),
            base.layer_count.to_string(),
            target.layer_count.to_string(),
            signed(target.layer_count as i64 - base.layer_count as i64),
        ),
        (
            t("md.f.eff"),
            format!("{} %", base.efficiency_score),
            format!("{} %", target.efficiency_score),
            signed(target.efficiency_score as i64 - base.efficiency_score as i64),
        ),
        (
            t("md.f.wasted"),
            ByteSize(base.wasted_size).to_string(),
            ByteSize(target.wasted_size).to_string(),
            signed_bytes(target.wasted_size as i64 - base.wasted_size as i64),
        ),
        (
            t("diff.files"),
            base.file_count.to_string(),
            target.file_count.to_string(),
            signed(target.file_count as i64 - base.file_count as i64),
        ),
    ];
    for (label, b, t, change) in rows {
        md.push_str(&format!("| {label} | {b} | {t} | {change} |\n"));
    }
    md.push('\n');

    if diff.shared_layers > 0 {
        md.push_str(&f(
            "diff.shared",
            &[
                &diff.shared_layers.to_string(),
                &ByteSize(diff.shared_size).to_string(),
            ],
        ));
    } else {
        md.push_str(&t("diff.noshared"));
    }
    md.push_str("\n\n");

    for (key, layers) in [
        ("diff.layers.base", &diff.base_layers),
        ("diff.layers.target", &diff.target_layers),
    ] {
        if layers.is_empty() {
            continue;
        }
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} |\n|---:|---:|---|\n",
            f(key, &[&layers.len().to_string()]),
            t("sum.col.layer"),
            t("sum.col.size"),
            t("md.command"),
        ));
        for l in layers.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                l.layer,
                ByteSize(l.unpack_size),
                cell_code(&l.cmd)
            ));
        }
        push_more(&mut md, layers.len(), lang);
    }

    if !diff.dirs.is_empty() {
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} | {} |\n|---:|---:|---:|---|\n",
            t("diff.dirs"),
            t("diff.col.change"),
            t("diff.col.base"),
            t("diff.col.target"),
            t("diff.col.dir"),
        ));
        for d in diff.dirs.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | {} | `/{}` |\n",
                signed_bytes(d.delta()),
                ByteSize(d.base_size),
                ByteSize(d.target_size),
                d.path
            ));
        }
        md.push('\n');
    }

    for (key, set, size_of) in [
        (
            "diff.added",
            &diff.added,
            (|c| c.target_size) as fn(&FileChange) -> u64,
        ),
        ("diff.removed", &diff.removed, |c| c.base_size),
    ] {
        if set.count == 0 {
            continue;
        }
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} |\n|---:|---:|---|\n",
            f(key, &[&set.count.to_string(), &signed_bytes(set.bytes)]),
            t("sum.col.size"),
            t("sum.col.layer"),
            t("sum.col.path"),
        ));
        for c in set.files.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | `{}` |\n",
                ByteSize(size_of(c)),
                c.layer,
                c.path
            ));
        }
        push_more(&mut md, set.count, lang);
    }
    if diff.changed.count > 0 {
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} | {} |\n|---:|---:|---:|---|\n",
            f(
                "diff.changed",
                &[
                    &diff.changed.count.to_string(),
                    &signed_bytes(diff.changed.bytes)
                ]
            ),
            t("diff.col.change"),
            t("diff.col.base"),
            t("diff.col.target"),
            t("sum.col.path"),
        ));
        for c in diff.changed.files.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | {} | `{}` |\n",
                signed_bytes(c.delta()),
                ByteSize(c.base_size),
                ByteSize(c.target_size),
                c.path
            ));
        }
        push_more(&mut md, diff.changed.count, lang);
    }

    for (key, recs) in [
        ("diff.recs.new", &diff.new_recommendations),
        ("diff.recs.resolved", &diff.resolved_recommendations),
    ] {
        if recs.is_empty() {
            continue;
        }
        md.push_str(&format!("**{}**\n\n", f(key, &[&recs.len().to_string()])));
        for r in recs {
            md.push_str(&format!(
                "- {} **{}** · {} (`{}`)\n",
                icon(&r.severity),
                r.title,
                t(&format!("cat.{}", r.category)),
                r.id
            ));
        }
        md.push('\n');
    }
    md
}

/// Close a table, noting the rows it left out.
fn push_more(md: &mut String, total: usize, lang: Lang) {
    if total > MD_ROWS {
        md.push_str(&format!(
            "\n_{}_\n",
            i18n::fill(
                i18n::tr(lang, "diff.more"),
                &[&(total - MD_ROWS).to_string()]
            )
        ));
    }
    md.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{FileTreeItem, Op};

    fn leaf(name: &str, size: u64) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            size,
            ..Default::default()
        }
    }

    fn node(name: &str, children: Vec<FileTreeItem>) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            size: children.iter().map(|c| c.size).sum(),
            children,
            ..Default::default()
        }
    }

    fn layer(digest: &str, unpack_size: u64) -> ImageLayer {
        ImageLayer {
            digest: digest.to_string(),
            cmd: format!("RUN build {digest}"),
            unpack_size,
            size: unpack_size / 2,
            ..Default::default()
        }
    }

    fn rec(id: &str) -> Recommendation {
        Recommendation {
            id: id.to_string(),
            category: "size".to_string(),
            severity: "medium".to_string(),
            title: format!("{id} title"),
            ..Default::default()
        }
    }

    fn image(
        name: &str,
        layers: Vec<ImageLayer>,
        trees: Vec<Vec<FileTreeItem>>,
        recs: Vec<Recommendation>,
    ) -> DockerAnalyzeResult {
        DockerAnalyzeResult {
            name: name.to_string(),
            total_size: layers.iter().map(|l| l.unpack_size).sum(),
            size: layers.iter().map(|l| l.size).sum(),
            layers,
            file_tree_list: trees,
            recommendations: recs,
            ..Default::default()
        }
    }

    fn base_os() -> Vec<FileTreeItem> {
        vec![node(
            "usr",
            vec![node(
                "lib",
                vec![leaf("libc.so", 1000), leaf("libz.so", 100)],
            )],
        )]
    }

    fn sample() -> ImageDiff {
        let env = ImageLayer {
            empty: true,
            ..Default::default()
        };
        let base = image(
            "app:1",
            vec![
                layer("sha256:os", 1100),
                env.clone(),
                layer("sha256:app1", 600),
            ],
            vec![
                base_os(),
                vec![],
                vec![node(
                    "app",
                    vec![
                        leaf("server", 500),
                        leaf("old.txt", 100),
                        node("assets", vec![leaf("logo.png", 0)]),
                    ],
                )],
            ],
            vec![rec("runasroot"), rec("pkgcache")],
        );
        let target = image(
            "app:2",
            vec![layer("sha256:os", 1100), layer("sha256:app2", 5900), env],
            vec![
                base_os(),
                vec![
                    node(
                        "app",
                        vec![
                            leaf("server", 800),
                            node("node_modules", vec![node("big", vec![leaf("x.js", 5000)])]),
                            node("assets", vec![leaf("logo.png", 0)]),
                        ],
                    ),
                    node("usr", vec![node("lib", vec![leaf("libz.so", 0)])]),
                    // a whiteout, so it never shows up as added
                    FileTreeItem {
                        op: Op::Removed,
                        ..leaf("tmp", 0)
                    },
                ],
                vec![],
            ],
            vec![rec("runasroot"), rec("unstripped")],
        );
        diff_images(&base, &target)
    }

    #[test]
    fn layers_match_by_diff_id_then_by_content_digest() {
        let with = |digest: &str, diff_id: &str| ImageLayer {
            digest: digest.to_string(),
            diff_id: diff_id.to_string(),
            ..Default::default()
        };
        // docker save paths say nothing; the diff ids decide
        assert!(same_layer(
            &with("layer1/layer.tar", "sha256:aa"),
            &with("blobs/sha256/aa", "sha256:aa")
        ));
        assert!(!same_layer(
            &with("layer1/layer.tar", "sha256:aa"),
            &with("layer1/layer.tar", "sha256:bb")
        ));
        // a cache from before diff ids were recorded: registry digests
        assert!(same_layer(
            &with("sha256:cc", ""),
            &with("sha256:cc", "sha256:x")
        ));
        assert!(!same_layer(
            &with("layer1/layer.tar", ""),
            &with("layer1/layer.tar", "")
        ));
    }

    #[test]
    fn dir_bucket_cuts_to_the_parent_at_most_three_deep() {
        assert_eq!(dir_bucket("README"), "");
        assert_eq!(dir_bucket("app/server"), "app");
        assert_eq!(dir_bucket("usr/local/lib/libpython.so"), "usr/local/lib");
        assert_eq!(
            dir_bucket("app/node_modules/big/lib/x.js"),
            "app/node_modules/big"
        );
    }

    #[test]
    fn diff_aligns_layers_and_compares_final_files() {
        let diff = sample();
        assert_eq!(diff.size_delta, 7000 - 1700);
        assert_eq!(diff.shared_layers, 1);
        assert_eq!(diff.shared_size, 1100);
        // Layer numbers are each image's own (the base has an ENV layer
        // in between).
        assert_eq!(diff.base_layers[0].layer, 3);
        assert_eq!(diff.target_layers[0].layer, 2);
        assert_eq!((diff.base.layer_count, diff.target.layer_count), (2, 2));

        assert_eq!(diff.added.count, 1);
        assert_eq!(diff.added.files[0].path, "app/node_modules/big/x.js");
        assert_eq!(diff.added.files[0].layer, 2);
        assert_eq!(diff.added.bytes, 5000);
        assert_eq!(diff.removed.count, 1);
        assert_eq!(diff.removed.files[0].path, "app/old.txt");
        assert_eq!(diff.removed.files[0].layer, 3);
        assert_eq!(diff.removed.bytes, -100);
        // server grew, libz.so was overwritten with an empty copy; the
        // unchanged zero-byte logo is not a change.
        let changed: Vec<(&str, i64)> = diff
            .changed
            .files
            .iter()
            .map(|c| (c.path.as_str(), c.delta()))
            .collect();
        assert_eq!(changed, [("app/server", 300), ("usr/lib/libz.so", -100)]);
        assert_eq!(diff.changed.bytes, 200);

        let dirs: Vec<(&str, i64)> = diff
            .dirs
            .iter()
            .map(|d| (d.path.as_str(), d.delta()))
            .collect();
        assert_eq!(
            dirs,
            [
                ("app/node_modules/big", 5000),
                ("app", 200),
                ("usr/lib", -100)
            ]
        );

        let ids = |recs: &[Recommendation]| recs.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&diff.new_recommendations), ["unstripped"]);
        assert_eq!(ids(&diff.resolved_recommendations), ["pkgcache"]);
    }

    #[test]
    fn diff_markdown_lists_the_changes() {
        let md = to_diff_markdown(&sample(), Lang::En, Some("**PASS**"));
        assert!(md.starts_with("### diving diff · `app:1` → `app:2`\n\n**PASS**"));
        assert!(md.contains("| Image size | 1.7 KiB | 6.8 KiB | **+5.2 KiB** (+311.8 %) |"));
        assert!(md.contains("the first 1 (1.1 KiB) are identical"));
        assert!(md.contains("`RUN build sha256:app2`"));
        assert!(md.contains("| +4.9 KiB | 0 B | 4.9 KiB | `/app/node_modules/big` |"));
        assert!(md.contains("`app/node_modules/big/x.js`"));
        assert!(md.contains("`app/old.txt`"));
        assert!(md.contains("(`unstripped`)"));
        assert!(md.contains("(`pkgcache`)"));
        let zh = to_diff_markdown(&sample(), Lang::Zh, None);
        assert!(zh.contains("新增文件"));
    }

    #[test]
    fn identical_images_have_no_changes() {
        let a = image(
            "a",
            vec![layer("sha256:os", 1100)],
            vec![base_os()],
            vec![rec("runasroot")],
        );
        let diff = diff_images(&a, &a);
        assert_eq!(diff.size_delta, 0);
        assert_eq!(diff.shared_layers, 1);
        assert!(diff.base_layers.is_empty() && diff.target_layers.is_empty());
        assert_eq!(
            (diff.added.count, diff.removed.count, diff.changed.count),
            (0, 0, 0)
        );
        assert!(diff.dirs.is_empty());
        assert!(diff.new_recommendations.is_empty());
    }

    #[test]
    fn cell_code_keeps_a_table_row_intact() {
        assert_eq!(cell_code("RUN a |\tb\n\t\tc"), "`RUN a \\| b c`");
    }
}
