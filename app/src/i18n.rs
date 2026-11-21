//! Application UI localization (`EN` / `ZH`) aligned with `diving_core::Lang`.

use diving_core::Lang;

/// Display language chosen by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiLang {
    #[default]
    En,
    Zh,
}

impl UiLang {
    pub fn to_core_lang(self) -> Lang {
        match self {
            Self::En => Lang::En,
            Self::Zh => Lang::Zh,
        }
    }

    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }

    pub fn from_storage_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "zh" | "zh-cn" | "zh-hans" => Self::Zh,
            _ => Self::En,
        }
    }
}

const CATALOG: &[(&str, &str, &str)] = &[
    // App shell & navigation
    ("app.title", "Diving", "Diving"),
    ("app.version_badge", "v0.12.3", "v0.12.3"),
    ("nav.back_home", "Back to Home", "返回首页"),
    ("nav.ui_gallery", "UI Gallery", "组件画廊"),
    ("menu.more_actions", "More actions", "更多操作"),
    ("menu.open_local_file", "Open local file…", "打开本地文件…"),
    (
        "menu.browse_local_images",
        "Browse Docker / Podman…",
        "浏览 Docker / Podman…",
    ),
    (
        "menu.export_markdown",
        "Export Markdown report…",
        "导出 Markdown 报告…",
    ),
    ("menu.clear_cache", "Clear local cache", "清理本地缓存"),
    ("menu.compare_with", "Compare with…", "对比镜像…"),
    ("theme.system", "System", "系统"),
    ("theme.light", "Light", "浅色"),
    ("theme.dark", "Dark", "深色"),
    (
        "theme.cycle_tooltip",
        "Cycle theme (System / Light / Dark)",
        "切换主题（跟随系统 / 浅色 / 深色）",
    ),
    // Landing & progressive preview
    (
        "landing.eyebrow",
        "Docker · OCI · layers",
        "Docker · OCI · 镜像层",
    ),
    ("landing.title", "Dive into every layer", "潜入每一层镜像"),
    (
        "landing.lead",
        "Inspect layer waste, file trees, secrets, and runtime compatibility across Docker/OCI images.",
        "深入分析 Docker/OCI 镜像的分层体积、文件树、敏感文件、跨层重复与运行时兼容性。",
    ),
    (
        "landing.placeholder",
        "redis:alpine, ghcr.io/org/app:tag, file://…",
        "redis:alpine、ghcr.io/org/app:tag 或 file://…",
    ),
    ("landing.analyze", "Analyze", "开始分析"),
    ("landing.analyzing", "Analyzing…", "分析中…"),
    ("landing.cancel", "Cancel", "取消"),
    ("landing.retry", "Retry", "重试"),
    ("landing.samples", "Examples", "示例镜像"),
    ("landing.recent", "Recent Analyses", "最近分析"),
    (
        "landing.local_picker_pending",
        "Docker/Podman image picker will open here in M3; use docker://image:tag or podman://image:tag for now.",
        "本地 Docker/Podman 镜像选择器将在 M3 上线；当前可直接输入 docker:// 或 podman:// 镜像名。",
    ),
    ("landing.arch_auto", "Auto arch", "默认架构"),
    (
        "preview.title",
        "Image Manifest Preview",
        "镜像 Manifest 预览",
    ),
    (
        "preview.meta",
        "{0} · {1} layers · {2} compressed",
        "{0} · {1} 层 · 压缩后 {2}",
    ),
    (
        "preview.hint",
        "Layers from the image manifest. File tree and findings appear once layers finish reading.",
        "以上为 Manifest 解析出的镜像层。各层下载解压完成后将进入完整分析工作台。",
    ),
    (
        "progress.auth",
        "Authenticating with {0}…",
        "正在认证仓库 {0}…",
    ),
    (
        "progress.manifest",
        "Fetching manifest…",
        "正在获取 Manifest…",
    ),
    (
        "progress.layers_discovered",
        "Discovered {0} layers ({1})…",
        "已发现 {0} 个镜像层（{1}）…",
    ),
    (
        "progress.downloading",
        "Downloading layer ({0})…",
        "正在下载镜像层（{0}）…",
    ),
    (
        "progress.layer_cached",
        "Loaded cached layer ({0})",
        "命中本地缓存镜像层（{0}）",
    ),
    (
        "progress.cache_hit",
        "Loaded analysis from cache",
        "命中本地分析缓存",
    ),
    // KPI & Stale banner
    (
        "stale.banner",
        "Registry unavailable — showing cached snapshot from {0}.",
        "仓库暂不可达，当前显示 {0} 的缓存分析快照。",
    ),
    ("kpi.efficiency", "Efficiency", "效率评分"),
    ("kpi.image_size", "Image Size", "解压大小"),
    ("kpi.compressed", "Compressed", "压缩大小"),
    ("kpi.wasted", "Wasted", "浪费空间"),
    ("kpi.platform", "Platform", "系统 / 架构"),
    ("kpi.base_os", "Base OS", "基础系统"),
    ("kpi.user", "User", "运行用户"),
    ("kpi.libc", "Runtime libc", "运行时 libc"),
    ("kpi.libc_ok", "ok", "兼容"),
    ("kpi.libc_too_old", "host glibc too old", "宿主 glibc 过低"),
    (
        "kpi.libc_glibc_on_musl",
        "glibc binary on musl",
        "glibc 程序无法运行于 musl",
    ),
    (
        "kpi.libc_musl_on_glibc",
        "musl binary on glibc",
        "musl 程序需验证依赖",
    ),
    // Inspector workspace & layers
    ("layers.title", "Layers", "镜像层"),
    ("layers.filter_all", "All", "全部"),
    ("layers.filter_files", "Files", "含文件"),
    ("layers.filter_large", "> 1 MB", "> 1 MB"),
    (
        "layers.no_match",
        "No layers match this filter.",
        "没有匹配筛选条件的镜像层。",
    ),
    ("layers.ctx_copy_cmd", "Copy full command", "复制完整指令"),
    (
        "layers.ctx_copy_digest",
        "Copy layer digest",
        "复制层 Digest",
    ),
    ("tab.files", "File Tree", "文件树"),
    ("tab.treemap", "Treemap", "空间分布"),
    ("tab.findings", "Findings", "分析发现"),
    ("tab.dockerfile", "Dockerfile", "Dockerfile"),
    ("cmd.show_more", "Show {0} more lines", "展开其余 {0} 行"),
    ("cmd.collapse", "Collapse", "收起"),
    ("cmd.wrap", "Wrap", "换行"),
    ("cmd.copy_digest", "Copy digest", "复制摘要"),
    ("common.copy", "Copy", "复制"),
    ("common.copied", "Copied", "已复制"),
    ("common.close", "Close", "关闭"),
    ("common.save_as", "Save as…", "另存为…"),
    ("common.done", "Done", "完成"),
    ("common.cancel", "Cancel", "取消"),
    // File tree toolbar & columns
    (
        "tree.filter_placeholder",
        "Filter files in layer…",
        "按路径或文件名过滤…",
    ),
    ("tree.modifications_only", "Modifications", "仅改/删"),
    ("tree.compact_paths", "Compact paths", "合并单目录"),
    ("tree.show_meta_cols", "Mode & UID", "权限与属主"),
    ("tree.expand_all", "Expand all", "全部展开"),
    ("tree.collapse_all", "Collapse all", "全部折叠"),
    ("tree.size_all", "All sizes", "全部大小"),
    ("tree.size_100k", "> 100 KB", "> 100 KB"),
    ("tree.size_1m", "> 1 MB", "> 1 MB"),
    ("tree.size_10m", "> 10 MB", "> 10 MB"),
    ("tree.col_status", "Op", "变更"),
    ("tree.col_mode", "Mode", "权限"),
    ("tree.col_uid", "UID:GID", "UID:GID"),
    ("tree.col_size", "Size", "大小"),
    ("tree.col_path", "Path", "路径"),
    ("tree.preview", "Preview", "预览"),
    ("tree.ctx_preview", "Preview file", "预览文件"),
    ("tree.ctx_copy_path", "Copy path", "复制路径"),
    ("tree.ctx_save_as", "Save file as…", "导出文件…"),
    (
        "tree.empty_layer",
        "No files in this layer (metadata / config only).",
        "该层无文件变更（仅元数据或配置）。",
    ),
    (
        "tree.empty_filter",
        "No files match the current filter.",
        "没有匹配当前过滤条件的文件。",
    ),
    // File preview sheet
    (
        "sheet.loading",
        "Reading file contents from layer blob…",
        "正在从镜像层读取文件内容…",
    ),
    (
        "sheet.truncated",
        "Large file — showing first {0} of {1}.",
        "文件较大 — 仅展示前 {0}（共 {1}）。",
    ),
    (
        "sheet.binary",
        "Binary file — showing leading bytes as hex dump.",
        "二进制文件 — 以十六进制形式展示文件头部内容。",
    ),
    ("sheet.empty", "Empty file (0 bytes).", "空文件（0 字节）。"),
    // Treemap & Findings & Dockerfile
    (
        "treemap.m3_pending",
        "Interactive two-level Treemap & Table view will land in M3.",
        "交互式双层 Treemap 视图与表格模式将在 M3 阶段交付。",
    ),
    (
        "findings.recommendations",
        "Optimization Recommendations",
        "优化建议",
    ),
    (
        "findings.recommendations_empty",
        "No optimization issues flagged.",
        "未发现优化问题。",
    ),
    ("findings.est_savings", "est. savings: {0}", "预计节省：{0}"),
    ("findings.wasted", "Wasted Space", "浪费空间"),
    (
        "findings.wasted_empty",
        "Zero cross-layer wasted bytes detected.",
        "未发现跨层覆盖或删除造成的空间浪费。",
    ),
    (
        "findings.waste_removed_from",
        "written in layer {0}, deleted in layer {1}",
        "第 {0} 层写入，第 {1} 层删除",
    ),
    (
        "findings.waste_overwritten_from",
        "written in layer {0}, overwritten in layer {1}",
        "第 {0} 层写入，第 {1} 层覆盖",
    ),
    (
        "findings.waste_removed_in",
        "deleted in layer {0}",
        "第 {0} 层删除",
    ),
    (
        "findings.waste_overwritten_in",
        "overwritten in layer {0}",
        "第 {0} 层覆盖",
    ),
    ("findings.sensitive", "Sensitive Files", "敏感文件"),
    (
        "findings.sensitive_empty",
        "No sensitive files detected.",
        "未检测到敏感文件。",
    ),
    (
        "findings.duplicates",
        "Cross-Layer Duplicates",
        "跨层重复文件",
    ),
    (
        "findings.duplicates_empty",
        "No cross-layer duplicates detected.",
        "未检测到跨层重复文件。",
    ),
    (
        "findings.duplicates_copies",
        "{0} copies · wasted {1}",
        "{0} 份副本 · 额外占用 {1}",
    ),
    (
        "findings.big_files",
        "Large Modified / Added Files",
        "大体积新增或变更文件",
    ),
    (
        "findings.big_files_empty",
        "No large upper-layer file additions detected.",
        "未发现较大的上层新增或修改文件。",
    ),
    ("dockerfile.wrap", "Wrap lines", "自动换行"),
    ("dockerfile.export", "Export…", "导出…"),
    // Status bar & notices
    (
        "status.hints_files",
        "Click folder to expand · Click file or Preview to inspect · / Focus filter",
        "点击目录展开或折叠 · 点击文件或预览查看详情 · / 快速搜索",
    ),
    (
        "status.hints_general",
        "Switch layers on left · Switch tabs above",
        "左侧选择镜像层 · 上方切换工作区标签",
    ),
    ("status.source_registry", "Registry", "远程仓库"),
    ("status.source_file", "Local File", "本地文件"),
    ("status.source_docker", "Local Docker", "本地 Docker"),
    ("status.source_podman", "Local Podman", "本地 Podman"),
    (
        "notice.cache_cleaned",
        "Cleaned {0} expired files ({1}).",
        "已清理 {0} 个过期缓存文件（释放 {1}）。",
    ),
    ("notice.exported_to", "Saved to {0}.", "已保存至 {0}。"),
    // Treemap
    ("treemap.root", "Root", "根目录"),
    ("treemap.mode_map", "Map", "空间图"),
    ("treemap.mode_table", "Table", "表格"),
    ("treemap.other_layers", "Other layers", "其他层"),
    ("treemap.share", "Share", "占比"),
    ("treemap.dominant_layer", "Dominant layer", "主要写入层"),
    (
        "treemap.hover_hint",
        "Hover or select a directory or file to inspect layer ownership and size. Click directories to drill down.",
        "悬停或选择目录/文件查看写入层与占比，点击目录可继续下钻。",
    ),
    (
        "treemap.empty",
        "No files present in the final container filesystem.",
        "最终容器文件系统中不包含任何可见文件。",
    ),
    ("treemap.col_name", "Name", "名称"),
    ("treemap.col_size", "Size", "大小"),
    ("treemap.col_share", "Share", "占比"),
    ("treemap.col_layer", "Layer", "所属层"),
    // Image Diff & Compare Dialog
    ("diff.title", "Image Diff", "镜像对比"),
    ("diff.compare_with", "Compare with…", "对比镜像…"),
    ("diff.base_label", "Base image", "基准镜像 (Base)"),
    ("diff.target_label", "Target image", "目标镜像 (Target)"),
    ("diff.compare_action", "Compare", "开始对比"),
    ("diff.swap", "Swap", "交换基准"),
    ("diff.open_base", "Open Base", "查看基准镜像"),
    ("diff.open_target", "Open Target", "查看目标镜像"),
    ("diff.export_md", "Export Markdown…", "导出 Markdown…"),
    ("diff.edit_inputs", "Edit inputs…", "修改对比参数…"),
    ("diff.metric_image_size", "Image Size", "解压大小"),
    ("diff.metric_compressed", "Compressed", "压缩大小"),
    ("diff.metric_layers", "Layers", "镜像层数"),
    ("diff.metric_files", "Files", "文件数"),
    ("diff.metric_delta", "Delta", "变化量"),
    ("diff.layers_section", "Layers Diff", "镜像层差异"),
    (
        "diff.shared_layers",
        "{0} shared base layers ({1})",
        "共享 {0} 个基础层（{1}）",
    ),
    (
        "diff.no_shared_layers",
        "No shared base layers between these images.",
        "两个镜像之间没有共享的基础层。",
    ),
    ("diff.only_in_base", "Only in Base", "仅存在于基准镜像"),
    ("diff.only_in_target", "Only in Target", "仅存在于目标镜像"),
    (
        "diff.dirs_section",
        "Where the Size Changed",
        "目录体积变化分布",
    ),
    (
        "diff.dirs_empty",
        "No directory size differences detected.",
        "未检测到目录体积变化。",
    ),
    ("diff.files_section", "File Changes", "文件变更明细"),
    ("diff.tab_added", "Added", "新增"),
    ("diff.tab_removed", "Removed", "删除"),
    ("diff.tab_resized", "Resized", "大小变化"),
    (
        "diff.files_empty",
        "No file changes in this category.",
        "该分类下没有文件变化。",
    ),
    (
        "diff.truncated_files",
        "Showing the largest {0} of {1} files.",
        "当前仅显示体积变化最大的 {0} / {1} 个文件。",
    ),
    ("diff.recs_section", "Recommendations Diff", "优化建议变化"),
    ("diff.recs_new", "New issues", "新增建议"),
    ("diff.recs_resolved", "Resolved", "已解决"),
    (
        "diff.recs_none",
        "No changes in recommendations.",
        "相关优化建议无变化。",
    ),
    (
        "diff.loading",
        "Analyzing both images for diff comparison…",
        "正在并行分析基准与目标镜像…",
    ),
    (
        "diff.err_base_failed",
        "Base image failed",
        "基准镜像分析失败",
    ),
    (
        "diff.err_target_failed",
        "Target image failed",
        "目标镜像分析失败",
    ),
    (
        "diff.err_both_failed",
        "Both Base and Target images failed to analyze.",
        "基准镜像与目标镜像均分析失败。",
    ),
    ("diff.quick_arch", "Compare {0} with {1}", "对比 {0} 与 {1}"),
    // Local Docker / Podman Image Picker
    ("local.title", "Local Images", "本地镜像"),
    ("local.docker", "Docker", "Docker"),
    ("local.podman", "Podman", "Podman"),
    (
        "local.filter_placeholder",
        "Filter repository, tag, or ID…",
        "搜索仓库名、标签或镜像 ID…",
    ),
    ("local.refresh", "Refresh list", "刷新列表"),
    ("local.analyze_selected", "Analyze selected", "分析所选镜像"),
    (
        "local.cli_hint",
        "Exports the selected image via local CLI for layer analysis.",
        "将通过本地容器 CLI 导出所选镜像进行层级分析。",
    ),
    ("local.col_repo", "Repository:Tag", "仓库与标签"),
    ("local.col_id", "Image ID", "镜像 ID"),
    ("local.col_created", "Created", "创建时间"),
    ("local.col_size", "Size", "大小"),
    (
        "local.loading",
        "Listing local images…",
        "正在读取本地镜像列表…",
    ),
    (
        "local.empty",
        "No local images matched.",
        "未找到匹配的本地镜像。",
    ),
    (
        "local.daemon_err_title",
        "Unable to list local images",
        "无法读取本地镜像列表",
    ),
    ("local.switch_docker", "Switch to Docker", "切换 Docker"),
    ("local.switch_podman", "Switch to Podman", "切换 Podman"),
    // Command Palette & Application Menu
    (
        "cmd.placeholder",
        "Type a command or jump to a recent image…",
        "输入命令或快速跳转最近分析的镜像…",
    ),
    ("cmd.empty", "No matching commands.", "没有匹配的命令。"),
    ("cmd.group_recent", "Recent Images", "最近分析"),
    ("cmd.group_tabs", "Inspector Tabs", "工作区标签"),
    ("cmd.group_actions", "Actions", "常用操作"),
    ("cmd.group_appearance", "Appearance", "界面外观"),
    ("cmd.open_palette", "Command Palette…", "命令面板…"),
    ("cmd.focus_search", "Focus Image Search", "聚焦镜像搜索框"),
    (
        "cmd.reanalyze",
        "Re-analyze Current Image",
        "强制重新分析当前镜像",
    ),
    ("cmd.export_md", "Export Markdown…", "导出 Markdown…"),
    ("cmd.clear_cache", "Clear Local Cache", "清理本地缓存"),
    ("cmd.back_home", "Back to Home", "返回首页"),
    ("cmd.theme_system", "Theme: System", "主题：跟随系统"),
    ("cmd.theme_light", "Theme: Light", "主题：浅色"),
    ("cmd.theme_dark", "Theme: Dark", "主题：深色"),
    ("cmd.lang_en", "Language: English", "语言：English"),
    ("cmd.lang_zh", "Language: 中文", "语言：简体中文"),
    ("menu.file", "File", "文件"),
    ("menu.edit", "Edit", "编辑"),
    ("menu.view", "View", "视图"),
    ("menu.window", "Window", "窗口"),
];

/// Looks up a localized UI label by key, returning the key itself as a visible fallback if unknown.
pub fn t(lang: UiLang, key: &'static str) -> &'static str {
    for (k, en, zh) in CATALOG {
        if *k == key {
            return match lang {
                UiLang::En => en,
                UiLang::Zh => zh,
            };
        }
    }
    key
}

/// Formats a localized template string replacing `{0}`, `{1}`, ... with positional arguments.
pub fn tf(lang: UiLang, key: &'static str, args: &[&str]) -> String {
    let mut out = t(lang, key).to_owned();
    for (idx, val) in args.iter().enumerate() {
        let placeholder = format!("{{{idx}}}");
        out = out.replace(&placeholder, val);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_catalog_entries_have_unique_keys_and_non_empty_translations() {
        let mut seen = HashSet::new();
        for (key, en, zh) in CATALOG {
            assert!(seen.insert(*key), "duplicate i18n key: {key}");
            assert!(!en.trim().is_empty(), "empty EN string for key: {key}");
            assert!(!zh.trim().is_empty(), "empty ZH string for key: {key}");
            assert_eq!(t(UiLang::En, key), *en);
            assert_eq!(t(UiLang::Zh, key), *zh);
        }
    }

    #[test]
    fn template_formatting_replaces_numbered_placeholders() {
        let en = tf(UiLang::En, "preview.meta", &["linux/amd64", "5", "42 MB"]);
        let zh = tf(UiLang::Zh, "preview.meta", &["linux/amd64", "5", "42 MB"]);
        assert_eq!(en, "linux/amd64 · 5 layers · 42 MB compressed");
        assert_eq!(zh, "linux/amd64 · 5 层 · 压缩后 42 MB");
    }
}
