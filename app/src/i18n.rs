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
    ("nav.back_home", "Back to Home", "返回首页"),
    #[cfg(debug_assertions)]
    ("nav.ui_gallery", "UI Gallery", "组件画廊"),
    ("menu.more_actions", "More actions", "更多操作"),
    ("menu.open_local_file", "Open local file…", "打开本地文件…"),
    (
        "menu.browse_local_images",
        "Browse local images…",
        "浏览本地镜像…",
    ),
    (
        "menu.export_markdown",
        "Export Markdown report…",
        "导出 Markdown 报告…",
    ),
    ("menu.clear_cache", "Clear local cache", "清理本地缓存"),
    ("menu.compare_with", "Compare with…", "对比镜像…"),
    ("menu.registries", "Registries…", "私有仓库…"),
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
    ("landing.title", "Inspect container image", "分析容器镜像"),
    (
        "landing.lead",
        "Inspect layer waste, file trees, secrets, and runtime compatibility across Docker/OCI images.",
        "深入分析 Docker/OCI 镜像的分层体积、文件树、敏感文件、跨层重复与运行时兼容性。",
    ),
    (
        "landing.quick_local_file_desc",
        "Analyze .tar or OCI layout",
        "分析 .tar 归档或 OCI 目录",
    ),
    (
        "landing.quick_docker_desc",
        "Pick from local daemon",
        "从本地容器引擎选择镜像",
    ),
    (
        "landing.quick_compare_desc",
        "Diff layers & file sizes",
        "对比两版镜像的层级与体积",
    ),
    ("sidebar.section_workbench", "Workbench", "工作台"),
    ("sidebar.new_analysis", "New Analysis", "新建分析"),
    ("sidebar.local_images", "Local Images", "本地镜像"),
    ("sidebar.compare", "Image Diff", "镜像对比"),
    ("sidebar.open_file", "Open File…", "打开本地文件…"),
    ("action.open_folder", "Open OCI folder…", "打开 OCI 目录…"),
    ("sidebar.section_recent", "Recent", "最近分析"),
    (
        "sidebar.recent_empty",
        "No recent analyses yet",
        "暂无分析记录",
    ),
    (
        "sidebar.toggle_tooltip",
        "Toggle sidebar (⌘\\)",
        "切换侧边栏 (⌘\\)",
    ),
    (
        "header.switch_image_hint",
        "Click or ⌘L to switch image",
        "点击或按 ⌘L 切换镜像",
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
        "progress.layer_done",
        "Processed layer ({0})",
        "已解析镜像层（{0}）",
    ),
    (
        "progress.cache_hit",
        "Loaded analysis from cache",
        "命中本地分析缓存",
    ),
    ("progress.analyzing_image", "Analyzing {0}", "正在分析 {0}"),
    (
        "config.fallback_warning",
        "Config load failed ({0}); using default settings with Docker credentials.",
        "配置加载失败（{0}），已回退默认配置并保留 Docker 凭证。",
    ),
    ("diff.base_progress", "Base: {0}", "基准：{0}"),
    ("diff.target_progress", "Target: {0}", "目标：{0}"),
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
    ("kpi.libc_needs", "needs {0}", "需要 {0}"),
    ("kpi.libc_host", "host {0}", "宿主 {0}"),
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
    ("layers.order_desc", "#N→1", "#N→1"),
    ("layers.order_asc", "#1→N", "#1→N"),
    (
        "layers.order_desc_tip",
        "Upper layers first (#N → #1) — click for build order",
        "上层优先（#N → #1）— 点击切换构建顺序",
    ),
    (
        "layers.order_asc_tip",
        "Build order (#1 → #N) — click for upper layers first",
        "构建顺序（#1 → #N）— 点击切换上层优先",
    ),
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
    ("cmd.raw", "Raw", "原始"),
    ("cmd.wrap", "Wrap", "换行"),
    ("cmd.copy_digest", "Copy digest", "复制摘要"),
    ("common.copy", "Copy", "复制"),
    ("common.copied", "Copied", "已复制"),
    ("common.close", "Close", "关闭"),
    ("common.close_esc", "Close (Esc)", "关闭 (Esc)"),
    ("common.clear_esc", "Clear (Esc)", "清空 (Esc)"),
    ("common.save_as", "Save as…", "另存为…"),
    ("common.done", "Done", "完成"),
    ("common.cancel", "Cancel", "取消"),
    ("common.layer_number", "Layer #{0}", "第 {0} 层"),
    (
        "tooltip.open_palette",
        "Command Palette (⌘K)",
        "命令面板 (⌘K)",
    ),
    (
        "inspector.empty_title",
        "No image analyzed yet",
        "尚未分析镜像",
    ),
    (
        "inspector.empty_desc",
        "Enter an image reference or open a local tar file.",
        "请输入镜像名称或打开本地 tar / OCI 归档文件。",
    ),
    ("sheet.title_default", "File Preview", "文件预览"),
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
        "Binary file — showing first {0} of {1} as hex dump.",
        "二进制文件 — 以十六进制展示前 {0}（共 {1}）。",
    ),
    ("sheet.empty", "Empty file (0 bytes).", "空文件（0 字节）。"),
    ("sheet.symlink", "Symbolic link", "符号链接"),
    (
        "sheet.symlink_target",
        "Symbolic link → {0}",
        "符号链接 → {0}",
    ),
    (
        "sheet.symlink_no_target",
        "Symbolic link (no target recorded).",
        "符号链接（未记录目标路径）。",
    ),
    ("sheet.mode_preview", "Preview", "预览"),
    ("sheet.mode_hex", "Hex", "十六进制"),
    ("sheet.mode_source", "Source", "源码"),
    ("sheet.mode_formatted", "Formatted", "格式化"),
    ("sheet.mode_raw", "Raw", "原始"),
    (
        "sheet.auto_formatted",
        "Auto-formatted for readability — switch to Raw to inspect exact file contents.",
        "当前已自动格式化以便阅读 — 切换到「原始」可查看真实文件内容。",
    ),
    ("sheet.view_raw_data", "View raw data", "查看原始数据"),
    ("sheet.wrap_lines", "Wrap", "自动换行"),
    ("sheet.mode_contents", "Contents", "目录内容"),
    ("sheet.mode_inspector", "Inspector", "结构信息"),
    ("sheet.fit_window", "Fit", "适应窗口"),
    ("sheet.actual_size", "1:1", "原始尺寸"),
    (
        "sheet.decompressed",
        "Decompressed from {0} stream ({1} shown).",
        "已自动解压 {0} 流（展示 {1}）。",
    ),
    (
        "sheet.archive_summary",
        "{0} · {1} entries · {2} uncompressed",
        "{0} · 共 {1} 项 · 解压大小 {2}",
    ),
    (
        "sheet.archive_truncated",
        "Showing first {0} of {1} entries.",
        "仅显示前 {0} 项（共 {1} 项）。",
    ),
    ("sheet.archive_col_mode", "Mode", "权限"),
    ("sheet.archive_col_size", "Size", "大小"),
    ("sheet.archive_col_path", "Path", "路径"),
    (
        "sheet.image_too_large",
        "Image exceeds {0} inline limit — use Save as… or switch to Hex.",
        "图片超过 {0} 内嵌预览上限 — 请点击“导出”或切换到十六进制查看。",
    ),
    (
        "sheet.media_hint",
        "Media container detected. Audio/video playback is supported in the Web viewer, or use Save as… to open locally.",
        "已识别媒体容器。Web 端支持直接播放音视频，也可点击“导出”到本地打开。",
    ),
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
    ("findings.heuristic", "heuristic", "启发式"),
    ("findings.show_all", "Show all ({0})", "显示全部（{0}）"),
    ("findings.show_less", "Show fewer", "收起"),
    (
        "findings.what_if_summary",
        "If applied: save ~{0} → projected size {1} (efficiency ~{2}%)",
        "若按建议优化：预计节省约 {0} → 镜像体积约 {1}（效率约 {2}%）",
    ),
    (
        "provenance.title",
        "Cross-Layer File Provenance",
        "跨层文件溯源",
    ),
    (
        "provenance.placeholder",
        "Search file path across all layers (e.g. nginx.conf, .env)…",
        "跨所有镜像层搜索文件路径（如 nginx.conf、.env）…",
    ),
    (
        "provenance.empty",
        "Type a path fragment to trace where files are added, modified, or deleted across layers.",
        "输入路径关键字，追踪文件在各层的添加、修改与删除历史。",
    ),
    (
        "provenance.no_matches",
        "No matching files found across layers.",
        "所有镜像层中未找到匹配文件。",
    ),
    ("provenance.button", "Trace across layers", "跨层溯源"),
    (
        "findings.unstripped",
        "Unstripped Binaries",
        "未剥离符号的二进制",
    ),
    (
        "findings.unstripped_empty",
        "No unstripped binaries detected.",
        "未检测到未剥离符号的二进制文件。",
    ),
    (
        "findings.unstripped_detail",
        "{0} (debug {1})",
        "{0}（调试信息 {1}）",
    ),
    ("severity.high", "HIGH", "高"),
    ("severity.medium", "MEDIUM", "中"),
    ("severity.low", "LOW", "低"),
    ("severity.info", "INFO", "提示"),
    ("category.size", "Size", "体积"),
    ("category.necessity", "Necessity", "必要性"),
    ("category.security", "Security", "安全"),
    (
        "tag.pkg_cache",
        "Contains Package Manager Cache",
        "含包管理器缓存",
    ),
    (
        "tag.dev_artifacts",
        "Development Artifacts",
        "含开发构建产物",
    ),
    ("tag.potential_secrets", "Potential Secrets", "疑似存在密钥"),
    ("tag.runs_as_root", "Runs as Root", "以 root 运行"),
    (
        "tag.high_layer_count",
        "High Layer Count: {0}",
        "镜像层数过多：{0}",
    ),
    ("dockerfile.wrap", "Wrap lines", "自动换行"),
    ("dockerfile.raw", "Raw", "原始指令"),
    ("dockerfile.export", "Export…", "导出…"),
    ("dockerfile.metadata", "Image Metadata", "镜像元数据"),
    ("dockerfile.envs", "Environment Variables", "环境变量"),
    ("dockerfile.labels", "Labels", "标签"),
    ("menu.export_json", "Export JSON Report…", "导出 JSON 报告…"),
    ("recent.clear_all", "Clear all", "清空"),
    ("recent.remove", "Remove", "移除"),
    ("recent.stale_badge", "stale", "快照"),
    ("common.image_placeholder", "Image", "镜像"),
    (
        "error.preview_cancelled",
        "Preview worker cancelled",
        "文件预览任务已取消",
    ),
    (
        "error.local_list_cancelled",
        "Local image listing worker cancelled",
        "本地镜像列表加载已取消",
    ),
    (
        "error.local_delete_cancelled",
        "Local image removal worker cancelled",
        "本地镜像删除已取消",
    ),
    (
        "error.local_archive_prefix",
        "Unable to read local archive: {0}",
        "无法读取本地镜像归档：{0}",
    ),
    (
        "error.io_prefix",
        "File I/O error: {0}",
        "文件读写失败：{0}",
    ),
    (
        "error.daemon_run_prefix",
        "Unable to run {0}: {1}",
        "无法运行 {0}：{1}",
    ),
    (
        "error.daemon_cmd_failed_prefix",
        "{0} failed: {1}",
        "{0} 执行失败：{1}",
    ),
    (
        "error.invalid_digest_prefix",
        "Invalid layer digest: {0}",
        "无效的镜像层摘要：{0}",
    ),
    (
        "error.file_not_found",
        "File not found in layer archive.",
        "在镜像层归档中未找到该文件。",
    ),
    (
        "error.decompress_prefix",
        "Unable to decompress layer data: {0}",
        "无法解压镜像层数据：{0}",
    ),
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
        "status.source_container",
        "Local Container",
        "本地 Container",
    ),
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
        "diff.shared_base_layers_one",
        "{0} shared base layer ({1})",
        "共享 {0} 个基础层（{1}）",
    ),
    (
        "diff.shared_base_layers",
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
    ("diff.tab_resized", "Modified", "文件变更"),
    (
        "diff.files_empty",
        "No file changes in this category.",
        "该分类下没有文件变化。",
    ),
    (
        "diff.truncated_files",
        "Showing the largest {0} of {1} files.",
        "当前仅显示变化最显著的 {0} / {1} 个文件。",
    ),
    ("diff.mode_label", "Base layers", "基础层策略"),
    (
        "diff.mode_auto",
        "Auto (skip on cross-arch)",
        "自动（跨架构跳过）",
    ),
    (
        "diff.mode_force",
        "Force skip base layers",
        "强制跳过基础层",
    ),
    ("diff.mode_off", "Include base layers", "包含基础层"),
    ("diff.mode_skip_n", "Skip first {0} layers", "跳过前 {0} 层"),
    (
        "diff.metric_upper_size",
        "Upper Layers Size",
        "上层解压大小",
    ),
    (
        "diff.skipped_base_layers",
        "Skipped leading base layers: {0} base ({1}) / {2} target ({3}) · {4}",
        "已跳过基础层：基准前 {0} 层（{1}）/ 目标前 {2} 层（{3}）· {4}",
    ),
    (
        "diff.skip_created_gap",
        "detected >1h layer build gap",
        "自动识别 >1h 构建时间间隔",
    ),
    (
        "diff.skip_first_layer",
        "cross-arch 1st rootfs layer",
        "跨架构跳过首层 rootfs",
    ),
    (
        "diff.skip_forced_gap",
        "forced >1h layer build gap",
        "强制按 >1h 时间间隔跳过",
    ),
    (
        "diff.skip_forced_first",
        "forced 1st rootfs layer",
        "强制跳过首层 rootfs",
    ),
    ("diff.skip_manual", "manual skip count", "手动指定跳过层数"),
    (
        "diff.parity_consistent",
        "Parity: Consistent — configuration, runtime, and compared files match.",
        "一致性结论：一致 —— 配置、运行时及比对文件均一致。",
    ),
    (
        "diff.parity_binary_only",
        "Parity: Binary diffs only — config & runtime match; {0} binary file(s) changed size.",
        "一致性结论：仅二进制差异 —— 配置与运行时一致；共 {0} 个编译二进制文件体积变化。",
    ),
    (
        "diff.parity_drift",
        "Parity: Config or file drift — {0} unexpected file diff(s), {1} config diff(s), {2} runtime diff(s) ({3} binary diff(s)).",
        "一致性结论：存在配置或文件差异 —— {0} 项非预期文件差异、{1} 项配置差异、{2} 项运行时差异（另有 {3} 个二进制变化）。",
    ),
    (
        "diff.content_verified",
        "Same-size blake3 check: verified {0} file(s) (≤ 1 MiB, {1} skipped).",
        "同大小内容校对：已验证 {0} 个文件（≤ 1 MiB，跳过 {1} 个）。",
    ),
    (
        "diff.content_unverified",
        "Same-size blake3 check skipped / blobs unavailable ({0} file(s)).",
        "同大小内容校对未执行或缺少层数据（{0} 个文件未哈希）。",
    ),
    (
        "diff.config_runtime_section",
        "Config & Runtime Parity",
        "配置与运行时一致性",
    ),
    (
        "diff.config_runtime_ok",
        "Configuration and runtime settings match across both images.",
        "镜像配置与运行时探测结果完全一致。",
    ),
    ("diff.col_setting", "Setting", "配置/运行时项"),
    ("diff.setting_base_os", "Base OS", "基础系统"),
    (
        "diff.setting_runtime_entrypoint",
        "Runtime entrypoint",
        "运行时入口",
    ),
    ("diff.setting_runtime_libc", "Runtime libc", "运行时 libc"),
    ("diff.setting_min_glibc", "Min glibc", "最低 glibc"),
    ("diff.setting_os_glibc", "OS glibc", "系统 glibc"),
    ("diff.setting_runtime_issue", "Runtime issue", "运行时问题"),
    (
        "diff.setting_unstripped",
        "Unstripped binaries",
        "未剥离符号的二进制",
    ),
    ("diff.healthcheck_configured", "Configured", "已配置"),
    ("diff.healthcheck_none", "Not configured", "未配置"),
    ("diff.unexpected_only", "Unexpected only", "仅看非预期差异"),
    ("diff.badge_binary", "Binary", "二进制"),
    ("diff.badge_content", "Content", "内容变更"),
    ("diff.badge_mode", "Mode", "权限"),
    ("diff.badge_link", "Symlink", "软链"),
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
    // Local Docker / Podman / Container Image Picker
    ("local.title", "Local Images", "本地镜像"),
    ("local.docker", "Docker", "Docker"),
    ("local.podman", "Podman", "Podman"),
    ("local.container", "Container", "Container"),
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
    (
        "local.switch_container",
        "Switch to Container",
        "切换 Container",
    ),
    ("local.delete_btn", "Delete…", "删除…"),
    ("local.ctx_analyze", "Analyze image", "分析镜像"),
    ("local.ctx_copy_ref", "Copy reference", "复制镜像引用"),
    ("local.ctx_delete", "Delete image…", "删除镜像…"),
    (
        "local.delete_confirm_title",
        "Delete “{0}”?",
        "删除「{0}」？",
    ),
    (
        "local.delete_confirm_desc",
        "Removes this image ({0}) from local {1}. This action cannot be undone.",
        "将从本地 {1} 中移除该镜像（{0}），此操作不可撤销。",
    ),
    ("local.delete_confirm_action", "Delete", "删除"),
    (
        "local.force_delete_title",
        "Force delete “{0}”?",
        "强制删除「{0}」？",
    ),
    (
        "local.force_delete_desc",
        "{0}\n\nForce deleting passes --force to remove the image reference even while used by a container.",
        "{0}\n\n强制删除将附带 --force 参数移除该镜像引用（即使仍有容器关联）。",
    ),
    ("local.force_delete_action", "Force delete", "强制删除"),
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
    ("cmd.toggle_sidebar", "Toggle Sidebar", "切换侧边栏"),
    ("cmd.theme_system", "Theme: System", "主题：跟随系统"),
    ("cmd.theme_light", "Theme: Light", "主题：浅色"),
    ("cmd.theme_dark", "Theme: Dark", "主题：深色"),
    ("cmd.lang_en", "Language: English", "语言：English"),
    ("cmd.lang_zh", "Language: 中文", "语言：简体中文"),
    ("menu.file", "File", "文件"),
    ("menu.edit", "Edit", "编辑"),
    ("menu.view", "View", "视图"),
    ("menu.window", "Window", "窗口"),
    // Auto-updater
    ("menu.check_updates", "Check for updates…", "检查更新…"),
    ("update.checking", "Checking for updates…", "正在检查更新…"),
    (
        "update.up_to_date",
        "Diving Desktop is up to date ({0}).",
        "Diving Desktop 已是最新版本（{0}）。",
    ),
    (
        "update.available_notice",
        "Update available: {0} → {1}",
        "发现新版本：{0} → {1}",
    ),
    ("update.action_install", "Update & restart", "更新并重启"),
    (
        "update.action_install_version",
        "Update & restart ({0})",
        "更新并重启（{0}）",
    ),
    ("update.action_skip", "Skip this version", "跳过此版本"),
    (
        "update.action_resume",
        "Resume reminders ({0})",
        "恢复此版本提醒（{0}）",
    ),
    (
        "update.downloading",
        "Downloading {0} ({1})…",
        "正在下载 {0}（{1}）…",
    ),
    ("update.installing", "Installing {0}…", "正在安装 {0}…"),
    (
        "update.ready_restart",
        "Installed {0}. Restart to finish updating.",
        "已安装 {0}，重启应用以完成更新。",
    ),
    ("update.restart_now", "Restart now", "立即重启"),
    (
        "update.opened_installer",
        "Opened installer for {0}. Drag Diving.app to Applications.",
        "已打开 {0} 安装包，请将 Diving.app 拖入「应用程序」。",
    ),
    ("update.failed", "Update failed: {0}", "更新失败：{0}"),
    ("update.retry", "Retry update", "重试更新"),
    (
        "update.skipped_notice",
        "Skipped update reminders for {0}.",
        "已跳过 {0} 的更新提醒。",
    ),
    (
        "update.unsupported",
        "In-place updates are not available for {0}/{1}. Download manually from GitHub Releases.",
        "当前平台（{0}/{1}）不支持自动更新，请前往 GitHub Releases 手动下载。",
    ),
    // Private registry auth & transport config
    ("error.configure_auth", "Configure Auth…", "配置仓库认证…"),
    ("registry.title", "Private Registries", "私有仓库认证"),
    (
        "registry.subtitle",
        "Per-domain credentials, bearer tokens, custom headers, and HTTP/TLS settings",
        "按域名配置账号密码、Bearer Token、自定义请求头及 HTTP / TLS 证书策略",
    ),
    ("registry.add", "Add", "添加"),
    (
        "registry.empty_title",
        "No registry rules yet",
        "暂未配置私有仓库规则",
    ),
    (
        "registry.empty_desc",
        "Add a domain or wildcard rule to pull from private or self-hosted registries.",
        "添加域名或通配符规则，用于拉取私有仓库或内网部署镜像。",
    ),
    ("registry.unnamed", "<new registry>", "<未命名域名>"),
    ("registry.host_label", "Host / Pattern", "仓库域名 / 通配符"),
    (
        "registry.host_placeholder",
        "harbor.corp.com, *.corp.internal, or 10.0.1.5:5000",
        "harbor.corp.com、*.corp.internal 或 10.0.1.5:5000",
    ),
    (
        "registry.host_help",
        "Exact host[:port] wins over longest *.domain wildcard suffix.",
        "精确域名（含端口）优先匹配，其次匹配最长 *.domain 通配符后缀。",
    ),
    ("registry.username_label", "Username", "用户名"),
    (
        "registry.username_placeholder",
        "robot$ci or ${REGISTRY_USER}",
        "robot$ci 或 ${REGISTRY_USER}",
    ),
    ("registry.password_label", "Password", "密码"),
    (
        "registry.password_placeholder",
        "Password or ${REGISTRY_PASSWORD}",
        "密码或 ${REGISTRY_PASSWORD}",
    ),
    (
        "registry.token_label",
        "Static Bearer Token",
        "静态 Bearer Token",
    ),
    (
        "registry.token_placeholder",
        "Optional static token or ${STATIC_TOKEN}",
        "可选静态 Token 或 ${STATIC_TOKEN}",
    ),
    (
        "registry.token_help",
        "Used when Username is empty; sent as Authorization: Bearer unless headers override it.",
        "未填写用户名时直接发送 Bearer Token（可被自定义 Authorization 头覆盖）。",
    ),
    (
        "registry.headers_label",
        "Custom Request Headers",
        "自定义请求头",
    ),
    ("registry.headers_add", "Add Header", "添加请求头"),
    (
        "registry.headers_empty",
        "No custom headers configured (origin-scoped; stripped on external S3 redirects).",
        "暂无自定义请求头（仅发送至匹配域名，跨源 307 下载重定向自动剥离）。",
    ),
    (
        "registry.header_name_placeholder",
        "Header name (e.g. X-Gateway-Token)",
        "请求头名称（如 X-Gateway-Token）",
    ),
    (
        "registry.header_value_placeholder",
        "Value or ${ENV_SECRET}",
        "请求头值或 ${ENV_SECRET}",
    ),
    (
        "registry.insecure_http",
        "Default to plain HTTP (http://) instead of HTTPS",
        "默认使用 HTTP（http://）而非 HTTPS",
    ),
    (
        "registry.skip_tls",
        "Skip TLS certificate verification (insecure)",
        "跳过 TLS 证书校验（自签名证书，不安全）",
    ),
    (
        "registry.ca_cert_label",
        "Custom CA Certificate (PEM)",
        "自定义 CA 根证书（PEM）",
    ),
    (
        "registry.ca_cert_placeholder",
        "~/.diving/certs/corp-root-ca.pem",
        "~/.diving/certs/corp-root-ca.pem",
    ),
    ("registry.ca_cert_browse", "Browse…", "选择…"),
    (
        "registry.ca_cert_dialog",
        "Select CA certificate PEM file",
        "选择 CA 根证书（PEM）文件",
    ),
    ("registry.delete_rule", "Delete Rule", "删除规则"),
    ("registry.reset_rule", "Reset Override", "还原默认配置"),
    ("registry.save", "Save", "保存"),
    ("registry.save_and_retry", "Save & Retry", "保存并重试"),
    (
        "registry.footer_hint",
        "Saved to ~/.diving/registries.yml (0600) · Supports ${ENV_VAR}",
        "保存至 ~/.diving/registries.yml（权限 0600）· 支持 ${ENV_VAR} 变量",
    ),
    ("registry.badge_config_yml", "config.yml", "config.yml"),
    ("registry.badge_override", "override", "已覆盖"),
    (
        "registry.error_empty_host",
        "Every registry rule must specify a non-empty host or wildcard pattern.",
        "每条私有仓库规则都必须填写域名或通配符。",
    ),
    (
        "registry.saved_notice",
        "Registry settings saved and applied.",
        "私有仓库配置已保存并热生效。",
    ),
];

/// Returns the desktop version badge string (preferring `DIVING_APP_VERSION` in CI/nightly builds).
pub fn app_version_badge() -> String {
    let ver = option_env!("DIVING_APP_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    format!("v{ver}")
}

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

/// Maps `diving_core::Lang` into the desktop UI `UiLang`.
pub fn ui_lang_from_core(lang: Lang) -> UiLang {
    match lang {
        Lang::En => UiLang::En,
        Lang::Zh => UiLang::Zh,
    }
}

/// Localizes bracketed risk tags produced by `diving-core` (e.g. `"[Potential Secrets]"`).
pub fn localize_risk_tag(lang: UiLang, raw: &str) -> String {
    let trimmed = raw.trim().trim_matches(|c| c == '[' || c == ']').trim();
    match trimmed {
        "Contains Package Manager Cache" => t(lang, "tag.pkg_cache").to_owned(),
        "Development Artifacts" => t(lang, "tag.dev_artifacts").to_owned(),
        "Potential Secrets" => t(lang, "tag.potential_secrets").to_owned(),
        "Runs as Root" => t(lang, "tag.runs_as_root").to_owned(),
        _ if trimmed.starts_with("High Layer Count:") => {
            let count = trimmed
                .strip_prefix("High Layer Count:")
                .map(str::trim)
                .unwrap_or_default();
            tf(lang, "tag.high_layer_count", &[count])
        }
        _ => trimmed.to_owned(),
    }
}

/// Localizes recommendation severity codes (`high` / `medium` / `low` / `info`) consistently.
pub fn localize_severity(lang: UiLang, raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "high" => t(lang, "severity.high").to_owned(),
        "medium" => t(lang, "severity.medium").to_owned(),
        "low" => t(lang, "severity.low").to_owned(),
        "info" => t(lang, "severity.info").to_owned(),
        _ => raw.trim().to_owned(),
    }
}

/// Localizes recommendation categories (`size` / `necessity` / `security`).
pub fn localize_category(lang: UiLang, raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "size" => t(lang, "category.size").to_owned(),
        "necessity" => t(lang, "category.necessity").to_owned(),
        "security" => t(lang, "category.security").to_owned(),
        _ => raw.trim().to_owned(),
    }
}

/// Formats shared base layer counts with EN singular/plural grammar and localized ZH copy.
pub fn format_shared_base_layers(lang: UiLang, count: usize, size_label: &str) -> String {
    let key = if count == 1 {
        "diff.shared_base_layers_one"
    } else {
        "diff.shared_base_layers"
    };
    tf(lang, key, &[&count.to_string(), size_label])
}

/// Converts raw `diving-core` error prefixes (`Tar fail:`, `IO fail:`, daemon CLI errors) into friendlier copy.
pub fn format_user_error(lang: UiLang, raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("Tar fail:") {
        return tf(lang, "error.local_archive_prefix", &[rest.trim()]);
    }
    if let Some(rest) = trimmed.strip_prefix("IO fail:") {
        return tf(lang, "error.io_prefix", &[rest.trim()]);
    }
    if let Some(rest) = trimmed.strip_prefix("Invalid blob digest:") {
        return tf(lang, "error.invalid_digest_prefix", &[rest.trim()]);
    }
    if trimmed == "File not found" {
        return t(lang, "error.file_not_found").to_owned();
    }
    if let Some(rest) = trimmed.strip_prefix("Decompress fail:") {
        return tf(lang, "error.decompress_prefix", &[rest.trim()]);
    }
    if let Some(rest) = trimmed.strip_prefix("failed to run ")
        && let Some((cmd, detail)) = rest.split_once(':')
    {
        let clean_cmd = cmd.trim().trim_matches('`');
        if !clean_cmd.is_empty() {
            return tf(lang, "error.daemon_run_prefix", &[clean_cmd, detail.trim()]);
        }
    }
    if let Some((cmd, detail)) = trimmed.split_once(" failed:") {
        let clean_cmd = cmd.trim().trim_matches('`');
        if !clean_cmd.is_empty() {
            return tf(
                lang,
                "error.daemon_cmd_failed_prefix",
                &[clean_cmd, detail.trim()],
            );
        }
    }
    if let Some((cmd, status)) = trimmed.split_once(" failed (") {
        let clean_cmd = cmd.trim().trim_matches('`');
        if matches!(
            clean_cmd.split_whitespace().next(),
            Some("docker" | "podman" | "container")
        ) {
            let clean_status = status.trim_end_matches(')').trim();
            return tf(
                lang,
                "error.daemon_cmd_failed_prefix",
                &[clean_cmd, clean_status],
            );
        }
    }
    trimmed.to_owned()
}

/// Strips inline Markdown emphasis (`**bold**`, `*italic*`) and inline code backticks (`` `code` ``)
/// while preserving path globs such as `/var/lib/apt/lists/*` or `*.pem`.
pub fn strip_inline_markdown(raw: &str) -> String {
    let no_backticks = strip_paired_backticks(raw);
    let chars: Vec<char> = no_backticks.chars().collect();
    let mut out = String::with_capacity(no_backticks.len());
    let mut i = 0;

    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing_double_star(&chars, i + 2) {
                for &ch in &chars[i + 2..end] {
                    out.push(ch);
                }
                i = end + 2;
                continue;
            }
        } else if chars[i] == '*'
            && i.checked_sub(1).is_none_or(|p| chars[p] != '/')
            && i + 1 < chars.len()
            && chars[i + 1] != '*'
            && !chars[i + 1].is_whitespace()
            && let Some(end) = find_closing_single_star(&chars, i + 1)
        {
            for &ch in &chars[i + 1..end] {
                out.push(ch);
            }
            i = end + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn strip_paired_backticks(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`'
            && let Some(rel_end) = chars[i + 1..].iter().position(|&c| c == '`')
        {
            let end = i + 1 + rel_end;
            if !chars[i + 1..end].contains(&'\n') {
                for &ch in &chars[i + 1..end] {
                    out.push(ch);
                }
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_closing_double_star(chars: &[char], start: usize) -> Option<usize> {
    if start >= chars.len() || chars[start].is_whitespace() {
        return None;
    }
    let mut j = start;
    while j + 1 < chars.len() {
        if chars[j] == '\n' {
            return None;
        }
        if chars[j] == '*' && chars[j + 1] == '*' && !chars[j - 1].is_whitespace() {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn find_closing_single_star(chars: &[char], start: usize) -> Option<usize> {
    let mut j = start;
    while j < chars.len() {
        if chars[j] == '\n' {
            return None;
        }
        if chars[j] == '*'
            && chars[j - 1] != '/'
            && !chars[j - 1].is_whitespace()
            && chars.get(j + 1).is_none_or(|&n| n != '*')
        {
            return Some(j);
        }
        j += 1;
    }
    None
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

        let bin_en = tf(UiLang::En, "sheet.binary", &["8 KiB", "64 KiB"]);
        let bin_zh = tf(UiLang::Zh, "sheet.binary", &["8 KiB", "64 KiB"]);
        assert_eq!(
            bin_en,
            "Binary file — showing first 8 KiB of 64 KiB as hex dump."
        );
        assert_eq!(bin_zh, "二进制文件 — 以十六进制展示前 8 KiB（共 64 KiB）。");
    }

    #[test]
    fn risk_tags_severities_categories_and_shared_layers_localize() {
        assert_eq!(
            localize_risk_tag(UiLang::En, "[Potential Secrets]"),
            "Potential Secrets"
        );
        assert_eq!(
            localize_risk_tag(UiLang::Zh, "[Potential Secrets]"),
            "疑似存在密钥"
        );
        assert_eq!(
            localize_risk_tag(UiLang::Zh, "[High Layer Count: 42]"),
            "镜像层数过多：42"
        );

        assert_eq!(localize_severity(UiLang::En, "high"), "HIGH");
        assert_eq!(localize_severity(UiLang::Zh, "high"), "高");
        assert_eq!(localize_category(UiLang::En, "security"), "Security");
        assert_eq!(localize_category(UiLang::Zh, "security"), "安全");

        assert_eq!(
            format_shared_base_layers(UiLang::En, 1, "5 MB"),
            "1 shared base layer (5 MB)"
        );
        assert_eq!(
            format_shared_base_layers(UiLang::En, 3, "12 MB"),
            "3 shared base layers (12 MB)"
        );
        assert_eq!(
            format_shared_base_layers(UiLang::Zh, 1, "5 MB"),
            "共享 1 个基础层（5 MB）"
        );

        assert_eq!(
            format_user_error(UiLang::En, "Tar fail: unexpected EOF"),
            "Unable to read local archive: unexpected EOF"
        );
        assert_eq!(
            format_user_error(UiLang::Zh, "Tar fail: unexpected EOF"),
            "无法读取本地镜像归档：unexpected EOF"
        );
        assert_eq!(
            format_user_error(
                UiLang::En,
                "failed to run `docker images`: No such file or directory (os error 2)"
            ),
            "Unable to run docker images: No such file or directory (os error 2)"
        );
        assert_eq!(
            format_user_error(
                UiLang::Zh,
                "failed to run `docker images`: No such file or directory (os error 2)"
            ),
            "无法运行 docker images：No such file or directory (os error 2)"
        );
        assert_eq!(
            format_user_error(UiLang::Zh, "failed to run `podman save`: permission denied"),
            "无法运行 podman save：permission denied"
        );
        assert_eq!(
            format_user_error(UiLang::En, "docker images failed: daemon not running"),
            "docker images failed: daemon not running"
        );
        assert_eq!(
            format_user_error(UiLang::Zh, "podman images failed: exit status 125"),
            "podman images 执行失败：exit status 125"
        );
        assert_eq!(
            format_user_error(
                UiLang::Zh,
                "docker save alpine:latest failed (exit status: 1)"
            ),
            "docker save alpine:latest 执行失败：exit status: 1"
        );
        assert_eq!(
            format_user_error(UiLang::En, "Invalid blob digest: sha256:bad"),
            "Invalid layer digest: sha256:bad"
        );
        assert_eq!(
            format_user_error(UiLang::Zh, "Invalid blob digest: sha256:bad"),
            "无效的镜像层摘要：sha256:bad"
        );
        assert_eq!(
            format_user_error(UiLang::En, "File not found"),
            "File not found in layer archive."
        );
        assert_eq!(
            format_user_error(UiLang::Zh, "File not found"),
            "在镜像层归档中未找到该文件。"
        );
        assert_eq!(
            format_user_error(UiLang::En, "Decompress fail: corrupt gzip"),
            "Unable to decompress layer data: corrupt gzip"
        );
        assert_eq!(
            format_user_error(UiLang::Zh, "Decompress fail: corrupt gzip"),
            "无法解压镜像层数据：corrupt gzip"
        );
    }

    #[test]
    fn app_version_badge_matches_package_version() {
        let expected = format!(
            "v{}",
            option_env!("DIVING_APP_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(app_version_badge(), expected);
        assert!(app_version_badge().starts_with('v'));
    }

    #[test]
    fn strip_inline_markdown_cleans_backticks_and_emphasis_preserving_globs() {
        assert_eq!(
            strip_inline_markdown("Add a dedicated user and `USER nonroot` before CMD."),
            "Add a dedicated user and USER nonroot before CMD."
        );
        assert_eq!(
            strip_inline_markdown("Create and clean up in the *same* RUN instruction."),
            "Create and clean up in the same RUN instruction."
        );
        assert_eq!(
            strip_inline_markdown("在*同一条* RUN 指令内创建并清理数据。"),
            "在同一条 RUN 指令内创建并清理数据。"
        );
        assert_eq!(
            strip_inline_markdown("**Shared layers:** none"),
            "Shared layers: none"
        );
        assert_eq!(
            strip_inline_markdown("apt: `rm -rf /var/lib/apt/lists/*` in the same RUN"),
            "apt: rm -rf /var/lib/apt/lists/* in the same RUN"
        );
        assert_eq!(strip_inline_markdown("*.pem and *.key"), "*.pem and *.key");
    }
}
